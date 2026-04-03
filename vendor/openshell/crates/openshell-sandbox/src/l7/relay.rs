// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Protocol-aware bidirectional relay with L7 inspection.
//!
//! Replaces `copy_bidirectional` for endpoints with L7 configuration.
//! Parses each request within the tunnel, evaluates it against OPA policy,
//! and either forwards or denies the request.

use crate::l7::provider::L7Provider;
use crate::l7::{EnforcementMode, L7EndpointConfig, L7Protocol, L7RequestInfo};
use crate::proxy::RestRouteContext;
use crate::secrets::ScopedSecretInjector;
use miette::{IntoDiagnostic, Result, miette};
use std::sync::Mutex;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info, warn};

/// Context for L7 request policy evaluation.
pub struct L7EvalContext {
    /// Host from the CONNECT request.
    pub host: String,
    /// Port from the CONNECT request.
    pub port: u16,
    /// Matched policy name from L4 evaluation.
    pub policy_name: String,
    /// Binary path (for cross-layer Rego evaluation).
    pub binary_path: String,
    /// Ancestor paths.
    pub ancestors: Vec<String>,
    /// Cmdline paths.
    pub cmdline_paths: Vec<String>,
    /// Request-scoped boundary secret injector for this endpoint.
    pub(crate) scoped_secret_injector: Option<ScopedSecretInjector>,
    /// Per-endpoint request router used to open the correct upstream per request.
    pub(crate) route_context: RestRouteContext,
}

/// Run protocol-aware L7 inspection on a tunnel.
///
/// This replaces `copy_bidirectional` for L7-enabled endpoints.
/// Protocol detection (peek) is the caller's responsibility — this function
/// assumes the streams are already proven to carry the expected protocol.
/// For TLS-terminated connections, ALPN proves HTTP; for plaintext, the
/// caller peeks on the raw `TcpStream` before calling this.
pub async fn relay_with_inspection<C>(
    config: &L7EndpointConfig,
    engine: Mutex<regorus::Engine>,
    client: &mut C,
    ctx: &L7EvalContext,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    match config.protocol {
        L7Protocol::Rest => relay_rest(config, &engine, client, ctx).await,
        L7Protocol::Sql => {
            warn!(
                host = %ctx.host,
                port = ctx.port,
                "SQL L7 provider not yet implemented, falling back to passthrough"
            );
            let mut upstream = ctx.route_context.connect_for_request(false).await?.stream;
            tokio::io::copy_bidirectional(client, &mut upstream)
                .await
                .into_diagnostic()?;
            Ok(())
        }
    }
}

/// REST relay loop: parse request -> evaluate -> allow/deny -> relay response -> repeat.
async fn relay_rest<C>(
    config: &L7EndpointConfig,
    engine: &Mutex<regorus::Engine>,
    client: &mut C,
    ctx: &L7EvalContext,
) -> Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    loop {
        let req = match crate::l7::rest::RestProvider.parse_request(client).await {
            Ok(Some(req)) => req,
            Ok(None) => return Ok(()),
            Err(e) => {
                if is_benign_connection_error(&e) {
                    debug!(
                        host = %ctx.host,
                        port = ctx.port,
                        error = %e,
                        "L7 connection closed"
                    );
                } else {
                    warn!(
                        host = %ctx.host,
                        port = ctx.port,
                        error = %e,
                        "HTTP parse error in L7 relay"
                    );
                }
                return Ok(());
            }
        };

        let request_info = L7RequestInfo {
            action: req.action.clone(),
            target: req.target.clone(),
        };

        let (allowed, reason) = evaluate_l7_request(engine, ctx, &request_info)?;
        let decision_str = match (allowed, config.enforcement) {
            (true, _) => "allow",
            (false, EnforcementMode::Audit) => "audit",
            (false, EnforcementMode::Enforce) => "deny",
        };

        if !allowed && config.enforcement == EnforcementMode::Enforce {
            info!(
                dst_host = %ctx.host,
                dst_port = ctx.port,
                policy = %ctx.policy_name,
                l7_protocol = "rest",
                l7_action = %request_info.action,
                l7_target = %request_info.target,
                l7_decision = decision_str,
                l7_deny_reason = %reason,
                secret_injection_action = "none",
                secret_swaps = ?Vec::<crate::secrets::SecretSwap>::new(),
                route_before = %ctx.route_context.normal_route_name(),
                route_after = "-",
                route_switch_reason = "none",
                upstream_proxy = "-",
                "L7_REQUEST",
            );
            crate::l7::rest::RestProvider
                .deny(&req, &ctx.policy_name, &reason, client)
                .await?;
            return Ok(());
        }

        let prepared = match crate::l7::rest::prepare_http_request(
            &req,
            ctx.scoped_secret_injector.as_ref(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let deny_reason = error.to_string();
                info!(
                    dst_host = %ctx.host,
                    dst_port = ctx.port,
                    policy = %ctx.policy_name,
                    l7_protocol = "rest",
                    l7_action = %request_info.action,
                    l7_target = %request_info.target,
                    l7_decision = "deny",
                    l7_deny_reason = %deny_reason,
                    secret_injection_action = "denied",
                    secret_swaps = ?Vec::<crate::secrets::SecretSwap>::new(),
                    route_before = %ctx.route_context.normal_route_name(),
                    route_after = "-",
                    route_switch_reason = "none",
                    upstream_proxy = "-",
                    "L7_REQUEST",
                );
                crate::l7::rest::RestProvider
                    .deny(&req, &ctx.policy_name, &deny_reason, client)
                    .await?;
                return Ok(());
            }
        };

        let route_before = ctx.route_context.normal_route_name();
        let secret_applied = !prepared.swaps.is_empty();
        let mut upstream = ctx
            .route_context
            .connect_for_request(secret_applied)
            .await?;
        let route_switch_reason = if route_before != upstream.route_name {
            "placeholder_present"
        } else {
            "none"
        };
        let secret_injection_action = if secret_applied { "applied" } else { "none" };

        info!(
            dst_host = %ctx.host,
            dst_port = ctx.port,
            policy = %ctx.policy_name,
            l7_protocol = "rest",
            l7_action = %request_info.action,
            l7_target = %request_info.target,
            l7_decision = decision_str,
            l7_deny_reason = %reason,
            secret_injection_action,
            secret_swaps = ?prepared.swaps,
            route_before,
            route_after = %upstream.route_name,
            route_switch_reason,
            upstream_proxy = %upstream.upstream_proxy,
            egress_profile = %upstream.egress_profile,
            "L7_REQUEST",
        );

        let _ = crate::l7::rest::relay_http_request_with_prepared_request(
            &req,
            client,
            &mut upstream.stream,
            &prepared,
        )
        .await?;
    }
}

fn is_benign_connection_error(err: &miette::Report) -> bool {
    const BENIGN: &[&str] = &[
        "close_notify",
        "tls handshake eof",
        "connection reset",
        "broken pipe",
        "unexpected eof",
        "client disconnected mid-request",
    ];
    let msg = err.to_string().to_ascii_lowercase();
    BENIGN.iter().any(|pat| msg.contains(pat))
}

fn evaluate_l7_request(
    engine: &Mutex<regorus::Engine>,
    ctx: &L7EvalContext,
    request: &L7RequestInfo,
) -> Result<(bool, String)> {
    let input_json = serde_json::json!({
        "network": {
            "host": ctx.host,
            "port": ctx.port,
        },
        "exec": {
            "path": ctx.binary_path,
            "ancestors": ctx.ancestors,
            "cmdline_paths": ctx.cmdline_paths,
        },
        "request": {
            "method": request.action,
            "path": request.target,
        }
    });

    let mut engine = engine
        .lock()
        .map_err(|_| miette!("OPA engine lock poisoned"))?;

    engine
        .set_input_json(&input_json.to_string())
        .map_err(|e| miette!("{e}"))?;

    let allowed = engine
        .eval_rule("data.openshell.sandbox.allow_request".into())
        .map_err(|e| miette!("{e}"))?;
    let allowed = allowed == regorus::Value::from(true);

    let reason = if allowed {
        String::new()
    } else {
        let val = engine
            .eval_rule("data.openshell.sandbox.request_deny_reason".into())
            .map_err(|e| miette!("{e}"))?;
        match val {
            regorus::Value::String(s) => s.to_string(),
            regorus::Value::Undefined => "request denied by policy".to_string(),
            other => other.to_string(),
        }
    };

    Ok((allowed, reason))
}
