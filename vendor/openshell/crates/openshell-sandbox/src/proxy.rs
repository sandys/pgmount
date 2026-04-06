// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! HTTP CONNECT proxy with OPA policy evaluation and process-identity binding.

use crate::denial_aggregator::DenialEvent;
use crate::identity::BinaryIdentityCache;
use crate::l7::tls::{ProxyTlsState, build_upstream_client_config_with_extra_certs};
use crate::opa::{NetworkAction, OpaEngine};
use crate::policy::ProxyPolicy;
use crate::secrets::{SecretInjectionRule, SecretResolver, contains_placeholder_bytes};
use miette::{IntoDiagnostic, Result};
use rustls::pki_types::ServerName;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tracing::{debug, info, warn};
use url::Url;

const MAX_HEADER_BYTES: usize = 8192;
const INFERENCE_LOCAL_HOST: &str = "inference.local";
const PACKAGE_PROXY_ENABLED_ENV: &str = "OPENERAL_PACKAGE_PROXY_ENABLED";
const PACKAGE_PROXY_PROFILE_ENV: &str = "OPENERAL_PACKAGE_PROXY_PROFILE";
const PACKAGE_PROXY_UPSTREAM_URL_ENV: &str = "OPENERAL_PACKAGE_PROXY_UPSTREAM_URL";
const PACKAGE_PROXY_CA_FILE_ENV: &str = "OPENERAL_PACKAGE_PROXY_CA_FILE";
const PACKAGE_PROXY_AUTHORIZATION_FILE_ENV: &str = "OPENERAL_PACKAGE_PROXY_AUTHORIZATION_FILE";

pub(crate) trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(crate) type BoxedStream = Box<dyn AsyncStream>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointEgressVia {
    Direct,
    PackageProxy,
}

#[derive(Debug, Clone)]
struct EndpointSettings {
    allowed_ips: Vec<String>,
    l7_config: Option<crate::l7::L7EndpointConfig>,
    egress_via: EndpointEgressVia,
    egress_profile: Option<String>,
    secret_injection: Vec<SecretInjectionRule>,
}

impl Default for EndpointSettings {
    fn default() -> Self {
        Self {
            allowed_ips: Vec::new(),
            l7_config: None,
            egress_via: EndpointEgressVia::Direct,
            egress_profile: None,
            secret_injection: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RestNormalRoute {
    Direct,
    PackageProxy {
        profile: String,
        upstream_proxy: String,
    },
}

#[derive(Clone)]
pub(crate) struct RestRouteContext {
    host: String,
    port: u16,
    tls_mode: crate::l7::TlsMode,
    resolved_addrs: Vec<SocketAddr>,
    normal_route: RestNormalRoute,
    package_proxy: Option<PackageProxyConfig>,
    tls_state: Option<Arc<ProxyTlsState>>,
}

pub(crate) struct RestUpstreamConnection {
    pub stream: BoxedStream,
    pub route_name: &'static str,
    pub egress_profile: String,
    pub upstream_proxy: String,
}

impl RestRouteContext {
    pub(crate) fn normal_route_name(&self) -> &'static str {
        match self.normal_route {
            RestNormalRoute::Direct => "direct",
            RestNormalRoute::PackageProxy { .. } => "package_proxy",
        }
    }

    pub(crate) fn normal_egress_profile(&self) -> &str {
        match &self.normal_route {
            RestNormalRoute::Direct => "-",
            RestNormalRoute::PackageProxy { profile, .. } => profile.as_str(),
        }
    }

    pub(crate) fn normal_upstream_proxy(&self) -> &str {
        match &self.normal_route {
            RestNormalRoute::Direct => "-",
            RestNormalRoute::PackageProxy { upstream_proxy, .. } => upstream_proxy.as_str(),
        }
    }

    pub(crate) async fn connect_for_request(
        &self,
        force_direct: bool,
    ) -> Result<RestUpstreamConnection> {
        let use_direct = force_direct || matches!(self.normal_route, RestNormalRoute::Direct);

        if use_direct {
            let tcp = TcpStream::connect(self.resolved_addrs.as_slice())
                .await
                .into_diagnostic()?;
            let stream: BoxedStream = if self.tls_mode == crate::l7::TlsMode::Terminate {
                let tls_state = self.tls_state.as_ref().ok_or_else(|| {
                    miette::miette!("TLS termination requested but TLS state is not configured")
                })?;
                let tls_stream = crate::l7::tls::tls_connect_upstream(
                    tcp,
                    self.host.clone(),
                    Arc::clone(tls_state.upstream_config()),
                )
                .await?;
                Box::new(tls_stream)
            } else {
                Box::new(tcp)
            };
            return Ok(RestUpstreamConnection {
                stream,
                route_name: "direct",
                egress_profile: "-".to_string(),
                upstream_proxy: "-".to_string(),
            });
        }

        let (profile, upstream_proxy) = match &self.normal_route {
            RestNormalRoute::PackageProxy {
                profile,
                upstream_proxy,
            } => (profile.clone(), upstream_proxy.clone()),
            RestNormalRoute::Direct => unreachable!("direct routes are handled above"),
        };
        let package_proxy = self.package_proxy.as_ref().ok_or_else(|| {
            miette::miette!(
                "package proxy route requested but sandbox package proxy is not configured"
            )
        })?;
        let upstream = connect_via_package_proxy(package_proxy, &self.host, self.port).await?;
        let stream: BoxedStream = if self.tls_mode == crate::l7::TlsMode::Terminate {
            let tls_state = self.tls_state.as_ref().ok_or_else(|| {
                miette::miette!("TLS termination requested but TLS state is not configured")
            })?;
            let tls_stream = crate::l7::tls::tls_connect_upstream(
                upstream,
                self.host.clone(),
                Arc::clone(tls_state.upstream_config()),
            )
            .await?;
            Box::new(tls_stream)
        } else {
            upstream
        };

        Ok(RestUpstreamConnection {
            stream,
            route_name: "package_proxy",
            egress_profile: profile,
            upstream_proxy,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageProxyScheme {
    Http,
    Https,
}

#[derive(Debug, Clone)]
pub struct PackageProxyConfig {
    profile: String,
    upstream_url: String,
    upstream_host: String,
    upstream_port: u16,
    scheme: PackageProxyScheme,
    authorization: Option<String>,
    extra_ca_paths: Vec<PathBuf>,
    upstream_tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl PackageProxyConfig {
    pub fn from_env() -> Result<Option<Self>> {
        if !env_var_enabled(PACKAGE_PROXY_ENABLED_ENV) {
            return Ok(None);
        }

        let upstream_url = std::env::var(PACKAGE_PROXY_UPSTREAM_URL_ENV)
            .map_err(|_| miette::miette!("{PACKAGE_PROXY_UPSTREAM_URL_ENV} is required"))?;
        let parsed = Url::parse(&upstream_url).map_err(|error| {
            miette::miette!("invalid package proxy URL {upstream_url}: {error}")
        })?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(miette::miette!(
                "package proxy URL credentials are not supported; use {PACKAGE_PROXY_AUTHORIZATION_FILE_ENV}"
            ));
        }
        if !matches!(parsed.path(), "" | "/") {
            return Err(miette::miette!(
                "package proxy URL must not include a path: {upstream_url}"
            ));
        }
        let upstream_host = parsed
            .host_str()
            .ok_or_else(|| miette::miette!("package proxy URL is missing a host: {upstream_url}"))?
            .to_string();
        let upstream_port = parsed.port_or_known_default().ok_or_else(|| {
            miette::miette!("package proxy URL is missing a usable port: {upstream_url}")
        })?;
        let scheme = match parsed.scheme() {
            "http" => PackageProxyScheme::Http,
            "https" => PackageProxyScheme::Https,
            other => {
                return Err(miette::miette!(
                    "unsupported package proxy URL scheme {other}; expected http or https"
                ));
            }
        };

        let profile = std::env::var(PACKAGE_PROXY_PROFILE_ENV).unwrap_or_else(|_| "socket".into());

        let extra_ca_paths = match std::env::var(PACKAGE_PROXY_CA_FILE_ENV) {
            Ok(path) if !path.trim().is_empty() => vec![PathBuf::from(path)],
            _ => Vec::new(),
        };
        let authorization = match std::env::var(PACKAGE_PROXY_AUTHORIZATION_FILE_ENV) {
            Ok(path) if !path.trim().is_empty() => {
                let value = std::fs::read_to_string(path.trim()).into_diagnostic()?;
                let value = value.trim().to_string();
                if value.is_empty() { None } else { Some(value) }
            }
            _ => None,
        };

        let upstream_tls_config = if matches!(scheme, PackageProxyScheme::Https) {
            Some(build_upstream_client_config_with_extra_certs(
                &extra_ca_paths,
            )?)
        } else {
            None
        };

        Ok(Some(Self {
            profile,
            upstream_url,
            upstream_host,
            upstream_port,
            scheme,
            authorization,
            extra_ca_paths,
            upstream_tls_config,
        }))
    }

    pub fn extra_ca_paths(&self) -> Vec<PathBuf> {
        self.extra_ca_paths.clone()
    }

    fn upstream_url(&self) -> &str {
        &self.upstream_url
    }

    fn profile(&self) -> &str {
        &self.profile
    }

    fn authorization(&self) -> Option<&str> {
        self.authorization.as_deref()
    }

    async fn connect(&self) -> Result<BoxedStream> {
        let tcp = TcpStream::connect((self.upstream_host.as_str(), self.upstream_port))
            .await
            .into_diagnostic()?;
        match self.scheme {
            PackageProxyScheme::Http => Ok(Box::new(tcp)),
            PackageProxyScheme::Https => {
                let connector = TlsConnector::from(Arc::clone(
                    self.upstream_tls_config
                        .as_ref()
                        .expect("https proxy config must have tls config"),
                ));
                let server_name =
                    ServerName::try_from(self.upstream_host.clone()).into_diagnostic()?;
                let tls_stream = connector
                    .connect(server_name, tcp)
                    .await
                    .into_diagnostic()?;
                Ok(Box::new(tls_stream))
            }
        }
    }
}

fn is_known_package_manager_name(name: &str) -> bool {
    matches!(
        name,
        "npm"
            | "npm-cli.js"
            | "npx"
            | "npx-cli.js"
            | "node"
            | "pnpm"
            | "pnpm.cjs"
            | "pnpm.js"
            | "yarn"
            | "yarnpkg"
            | "yarn.js"
            | "yarnpkg.js"
            | "corepack"
            | "corepack.js"
            | "cargo"
            | "pip"
            | "pip3"
            | "python"
            | "python3"
            | "uv"
    )
}

fn should_route_via_package_proxy(
    endpoint_settings: &EndpointSettings,
    package_proxy: Option<&PackageProxyConfig>,
    decision: &ConnectDecision,
) -> bool {
    if matches!(
        endpoint_settings.egress_via,
        EndpointEgressVia::PackageProxy
    ) {
        return true;
    }
    if package_proxy.is_none() {
        return false;
    }

    decision
        .binary
        .iter()
        .chain(decision.ancestors.iter())
        .chain(decision.cmdline_paths.iter())
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .any(is_known_package_manager_name)
}

fn env_var_enabled(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("yes")
            | Some("YES")
            | Some("on")
            | Some("ON")
    )
}

/// Result of a proxy CONNECT policy decision.
struct ConnectDecision {
    action: NetworkAction,
    /// Resolved binary path.
    binary: Option<PathBuf>,
    /// PID owning the socket.
    binary_pid: Option<u32>,
    /// Ancestor binary paths from process tree walk.
    ancestors: Vec<PathBuf>,
    /// Cmdline-derived absolute paths (for script detection).
    cmdline_paths: Vec<PathBuf>,
}

/// Outcome of an inference interception attempt.
///
/// Returned by [`handle_inference_interception`] so the call site can emit
/// a structured CONNECT deny log when the connection is not successfully routed.
enum InferenceOutcome {
    /// At least one request was successfully routed to a local inference backend.
    Routed,
    /// The connection was denied (TLS failure, non-inference request, etc.).
    Denied { reason: String },
}

/// Inference routing context for sandbox-local execution.
///
/// Holds a `Router` (HTTP client) and cached sets of resolved routes.
/// User routes serve `inference.local` traffic; system routes are consumed
/// in-process by the supervisor for platform functions (e.g. agent harness).
pub struct InferenceContext {
    pub patterns: Vec<crate::l7::inference::InferenceApiPattern>,
    router: openshell_router::Router,
    /// Routes for the user-facing `inference.local` endpoint.
    routes: Arc<tokio::sync::RwLock<Vec<openshell_router::config::ResolvedRoute>>>,
    /// Routes for supervisor-only system inference (`sandbox-system`).
    system_routes: Arc<tokio::sync::RwLock<Vec<openshell_router::config::ResolvedRoute>>>,
}

impl InferenceContext {
    pub fn new(
        patterns: Vec<crate::l7::inference::InferenceApiPattern>,
        router: openshell_router::Router,
        routes: Vec<openshell_router::config::ResolvedRoute>,
        system_routes: Vec<openshell_router::config::ResolvedRoute>,
    ) -> Self {
        Self {
            patterns,
            router,
            routes: Arc::new(tokio::sync::RwLock::new(routes)),
            system_routes: Arc::new(tokio::sync::RwLock::new(system_routes)),
        }
    }

    /// Get a handle to the user route cache for background refresh.
    pub fn route_cache(
        &self,
    ) -> Arc<tokio::sync::RwLock<Vec<openshell_router::config::ResolvedRoute>>> {
        self.routes.clone()
    }

    /// Get a handle to the system route cache for background refresh.
    pub fn system_route_cache(
        &self,
    ) -> Arc<tokio::sync::RwLock<Vec<openshell_router::config::ResolvedRoute>>> {
        self.system_routes.clone()
    }

    /// Make an inference call using system routes (supervisor-only).
    ///
    /// This is the in-process API for platform functions. It bypasses the
    /// CONNECT proxy entirely — the supervisor calls the router directly
    /// from the host network namespace.
    pub async fn system_inference(
        &self,
        protocol: &str,
        method: &str,
        path: &str,
        headers: Vec<(String, String)>,
        body: bytes::Bytes,
    ) -> Result<openshell_router::ProxyResponse, openshell_router::RouterError> {
        let routes = self.system_routes.read().await;
        self.router
            .proxy_with_candidates(protocol, method, path, headers, body, &routes)
            .await
    }
}

#[derive(Debug)]
pub struct ProxyHandle {
    #[allow(dead_code)]
    http_addr: Option<SocketAddr>,
    join: JoinHandle<()>,
}

impl ProxyHandle {
    /// Start the proxy with OPA engine for policy evaluation.
    ///
    /// The proxy uses OPA for network decisions with process-identity binding
    /// via `/proc/net/tcp`. All connections are evaluated through OPA policy.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_with_bind_addr(
        policy: &ProxyPolicy,
        bind_addr: Option<SocketAddr>,
        opa_engine: Arc<OpaEngine>,
        identity_cache: Arc<BinaryIdentityCache>,
        entrypoint_pid: Arc<AtomicU32>,
        tls_state: Option<Arc<ProxyTlsState>>,
        inference_ctx: Option<Arc<InferenceContext>>,
        secret_resolver: Option<Arc<SecretResolver>>,
        package_proxy: Option<PackageProxyConfig>,
        denial_tx: Option<mpsc::UnboundedSender<DenialEvent>>,
    ) -> Result<Self> {
        // Use override bind_addr, fall back to policy http_addr, then default
        // to loopback:3128.  The default allows the proxy to function when no
        // network namespace is available (e.g. missing CAP_NET_ADMIN) and the
        // policy doesn't specify an explicit address.
        let default_addr: SocketAddr = ([127, 0, 0, 1], 3128).into();
        let http_addr = bind_addr.or(policy.http_addr).unwrap_or(default_addr);

        // Only enforce loopback restriction when not using network namespace override
        if bind_addr.is_none() && !http_addr.ip().is_loopback() {
            return Err(miette::miette!(
                "Proxy http_addr must be loopback-only: {http_addr}"
            ));
        }

        let listener = TcpListener::bind(http_addr).await.into_diagnostic()?;
        let local_addr = listener.local_addr().into_diagnostic()?;
        info!(addr = %local_addr, "Proxy listening (tcp)");

        let join = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _addr)) => {
                        let opa = opa_engine.clone();
                        let cache = identity_cache.clone();
                        let spid = entrypoint_pid.clone();
                        let tls = tls_state.clone();
                        let inf = inference_ctx.clone();
                        let resolver = secret_resolver.clone();
                        let package_proxy = package_proxy.clone();
                        let dtx = denial_tx.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_tcp_connection(
                                stream,
                                opa,
                                cache,
                                spid,
                                tls,
                                inf,
                                resolver,
                                package_proxy,
                                dtx,
                            )
                            .await
                            {
                                warn!(error = %err, "Proxy connection error");
                            }
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "Proxy accept error");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            http_addr: Some(local_addr),
            join,
        })
    }

    #[allow(dead_code)]
    pub const fn http_addr(&self) -> Option<SocketAddr> {
        self.http_addr
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
}

/// Emit a denial event to the aggregator channel (if configured).
/// Used by `handle_tcp_connection` which owns `Option<Sender>`.
fn emit_denial(
    tx: &Option<mpsc::UnboundedSender<DenialEvent>>,
    host: &str,
    port: u16,
    binary: &str,
    decision: &ConnectDecision,
    reason: &str,
    stage: &str,
) {
    if let Some(tx) = tx {
        let _ = tx.send(DenialEvent {
            host: host.to_string(),
            port,
            binary: binary.to_string(),
            ancestors: decision
                .ancestors
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            deny_reason: reason.to_string(),
            denial_stage: stage.to_string(),
            l7_method: None,
            l7_path: None,
        });
    }
}

/// Emit a denial event from a borrowed sender reference.
/// Used by `handle_forward_proxy` which borrows `Option<&Sender>`.
fn emit_denial_simple(
    tx: Option<&mpsc::UnboundedSender<DenialEvent>>,
    host: &str,
    port: u16,
    binary: &str,
    decision: &ConnectDecision,
    reason: &str,
    stage: &str,
) {
    if let Some(tx) = tx {
        let _ = tx.send(DenialEvent {
            host: host.to_string(),
            port,
            binary: binary.to_string(),
            ancestors: decision
                .ancestors
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            deny_reason: reason.to_string(),
            denial_stage: stage.to_string(),
            l7_method: None,
            l7_path: None,
        });
    }
}

async fn handle_tcp_connection(
    mut client: TcpStream,
    opa_engine: Arc<OpaEngine>,
    identity_cache: Arc<BinaryIdentityCache>,
    entrypoint_pid: Arc<AtomicU32>,
    tls_state: Option<Arc<ProxyTlsState>>,
    inference_ctx: Option<Arc<InferenceContext>>,
    secret_resolver: Option<Arc<SecretResolver>>,
    package_proxy: Option<PackageProxyConfig>,
    denial_tx: Option<mpsc::UnboundedSender<DenialEvent>>,
) -> Result<()> {
    let mut buf = vec![0u8; MAX_HEADER_BYTES];
    let mut used = 0usize;

    loop {
        if used == buf.len() {
            respond(
                &mut client,
                b"HTTP/1.1 431 Request Header Fields Too Large\r\n\r\n",
            )
            .await?;
            return Ok(());
        }

        let n = client.read(&mut buf[used..]).await.into_diagnostic()?;
        if n == 0 {
            return Ok(());
        }
        used += n;

        if buf[..used].windows(4).any(|win| win == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buf[..used]);
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    if method != "CONNECT" {
        return handle_forward_proxy(
            method,
            target,
            &buf[..],
            used,
            &mut client,
            opa_engine,
            identity_cache,
            entrypoint_pid,
            package_proxy,
            denial_tx.as_ref(),
        )
        .await;
    }

    let (host, port) = parse_target(target)?;
    let host_lc = host.to_ascii_lowercase();

    if host_lc == INFERENCE_LOCAL_HOST {
        respond(&mut client, b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        let outcome = handle_inference_interception(
            client,
            INFERENCE_LOCAL_HOST,
            port,
            tls_state.as_ref(),
            inference_ctx.as_ref(),
        )
        .await?;
        if let InferenceOutcome::Denied { reason } = outcome {
            info!(action = "deny", reason = %reason, host = INFERENCE_LOCAL_HOST, "Inference interception denied");
        }
        return Ok(());
    }

    let peer_addr = client.peer_addr().into_diagnostic()?;
    let local_addr = client.local_addr().into_diagnostic()?;

    // Evaluate OPA policy with process-identity binding
    let decision = evaluate_opa_tcp(
        peer_addr,
        &opa_engine,
        &identity_cache,
        &entrypoint_pid,
        &host_lc,
        port,
    );

    // Extract action string and matched policy for logging
    let (matched_policy, deny_reason) = match &decision.action {
        NetworkAction::Allow { matched_policy } => (matched_policy.clone(), String::new()),
        NetworkAction::Deny { reason } => (None, reason.clone()),
    };

    // Build log context fields (shared by deny log below and deferred allow log after L7 check)
    let binary_str = decision
        .binary
        .as_ref()
        .map_or_else(|| "-".to_string(), |p| p.display().to_string());
    let pid_str = decision
        .binary_pid
        .map_or_else(|| "-".to_string(), |p| p.to_string());
    let ancestors_str = if decision.ancestors.is_empty() {
        "-".to_string()
    } else {
        decision
            .ancestors
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ")
    };
    let cmdline_str = if decision.cmdline_paths.is_empty() {
        "-".to_string()
    } else {
        decision
            .cmdline_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let policy_str = matched_policy.as_deref().unwrap_or("-");

    // Log denied connections immediately — they never reach L7.
    // Allowed connections are logged after the L7 config check (below)
    // so we can distinguish CONNECT (L4-only) from CONNECT_L7 (L7 follows).
    if matches!(decision.action, NetworkAction::Deny { .. }) {
        info!(
            src_addr = %peer_addr.ip(),
            src_port = peer_addr.port(),
            proxy_addr = %local_addr,
            dst_host = %host_lc,
            dst_port = port,
            binary = %binary_str,
            binary_pid = %pid_str,
            ancestors = %ancestors_str,
            cmdline = %cmdline_str,
            action = "deny",
            engine = "opa",
            policy = "-",
            reason = %deny_reason,
            "CONNECT",
        );
        emit_denial(
            &denial_tx,
            &host_lc,
            port,
            &binary_str,
            &decision,
            &deny_reason,
            "connect",
        );
        respond(&mut client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
        return Ok(());
    }

    let endpoint_settings = match query_endpoint_settings(&opa_engine, &decision, &host_lc, port) {
        Ok(settings) => settings,
        Err(reason) => {
            warn!(
                dst_host = %host_lc,
                dst_port = port,
                reason = %reason,
                "CONNECT blocked: invalid endpoint config"
            );
            emit_denial(
                &denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                &reason,
                "endpoint-config",
            );
            respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        }
    };

    let resolved_addrs =
        match resolve_destination_addrs(&host, port, &endpoint_settings.allowed_ips).await {
            Ok(addrs) => addrs,
            Err(reason) => {
                warn!(
                    dst_host = %host_lc,
                    dst_port = port,
                    reason = %reason,
                    "CONNECT blocked: destination validation failed"
                );
                emit_denial(
                    &denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    &reason,
                    "ssrf",
                );
                respond(&mut client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
                return Ok(());
            }
        };

    let l7_config = endpoint_settings.l7_config.clone();
    let connect_msg = if l7_config.is_some() {
        "CONNECT_L7"
    } else {
        "CONNECT"
    };

    if let Some(l7_config) = l7_config {
        let scoped_secret_injector = if endpoint_settings.secret_injection.is_empty() {
            None
        } else {
            let Some(secret_resolver) = secret_resolver.as_ref() else {
                let reason = "secret injection requires provider env placeholders, but no provider env is configured";
                warn!(dst_host = %host_lc, dst_port = port, reason, "CONNECT blocked");
                emit_denial(
                    &denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    reason,
                    "secret-injection",
                );
                respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            };
            match secret_resolver.scoped_injector(&endpoint_settings.secret_injection) {
                Ok(injector) => injector,
                Err(error) => {
                    let reason = error.to_string();
                    warn!(dst_host = %host_lc, dst_port = port, reason = %reason, "CONNECT blocked");
                    emit_denial(
                        &denial_tx,
                        &host_lc,
                        port,
                        &binary_str,
                        &decision,
                        &reason,
                        "secret-injection",
                    );
                    respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                    return Ok(());
                }
            }
        };

        let route_context = match build_rest_route_context(
            &host,
            port,
            &decision,
            &endpoint_settings,
            &resolved_addrs,
            package_proxy.as_ref(),
            tls_state.as_ref(),
        ) {
            Ok(context) => context,
            Err(reason) => {
                warn!(dst_host = %host_lc, dst_port = port, reason = %reason, "CONNECT blocked");
                emit_denial(
                    &denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    &reason,
                    "package-proxy",
                );
                respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            }
        };

        respond(&mut client, b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        info!(
            src_addr = %peer_addr.ip(),
            src_port = peer_addr.port(),
            proxy_addr = %local_addr,
            dst_host = %host_lc,
            dst_port = port,
            binary = %binary_str,
            binary_pid = %pid_str,
            ancestors = %ancestors_str,
            cmdline = %cmdline_str,
            action = "allow",
            engine = "opa",
            policy = %policy_str,
            egress_via = %route_context.normal_route_name(),
            egress_profile = %route_context.normal_egress_profile(),
            upstream_proxy = %route_context.normal_upstream_proxy(),
            resolved_ips = ?resolved_addrs,
            reason = "",
            connect_msg,
        );

        let tunnel_engine = opa_engine.clone_engine_for_tunnel().unwrap_or_else(|e| {
            warn!(error = %e, "Failed to clone OPA engine for L7, falling back to empty engine");
            regorus::Engine::new()
        });

        let ctx = crate::l7::relay::L7EvalContext {
            host: host_lc.clone(),
            port,
            policy_name: matched_policy.clone().unwrap_or_default(),
            binary_path: decision
                .binary
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ancestors: decision
                .ancestors
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            cmdline_paths: decision
                .cmdline_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            scoped_secret_injector,
            route_context,
        };

        if l7_config.tls == crate::l7::TlsMode::Terminate {
            let Some(ref tls) = tls_state else {
                let reason = "TLS termination requested but TLS state not configured";
                warn!(host = %host_lc, port = port, reason, "TLS L7 relay blocked");
                return Err(miette::miette!("{reason}"));
            };
            let l7_result = async {
                let mut tls_client =
                    crate::l7::tls::tls_terminate_client(client, tls, &host_lc).await?;
                crate::l7::relay::relay_with_inspection(
                    &l7_config,
                    std::sync::Mutex::new(tunnel_engine),
                    &mut tls_client,
                    &ctx,
                )
                .await
            };
            if let Err(e) = l7_result.await {
                if is_benign_relay_error(&e) {
                    debug!(
                        host = %host_lc,
                        port = port,
                        error = %e,
                        "TLS L7 connection closed"
                    );
                } else {
                    warn!(
                        host = %host_lc,
                        port = port,
                        error = %e,
                        "TLS L7 relay error"
                    );
                }
            }
        } else {
            if l7_config.protocol == crate::l7::L7Protocol::Rest {
                let mut peek_buf = [0u8; 8];
                let n = client.peek(&mut peek_buf).await.into_diagnostic()?;
                if n == 0 {
                    return Ok(());
                }
                if !crate::l7::rest::looks_like_http(&peek_buf[..n]) {
                    warn!(
                        host = %host_lc,
                        port = port,
                        policy = %ctx.policy_name,
                        "Expected REST protocol but received non-matching bytes. Connection rejected."
                    );
                    return Err(miette::miette!(
                        "Protocol mismatch: expected HTTP but received non-HTTP bytes"
                    ));
                }
            }
            if let Err(e) = crate::l7::relay::relay_with_inspection(
                &l7_config,
                std::sync::Mutex::new(tunnel_engine),
                &mut client,
                &ctx,
            )
            .await
            {
                if is_benign_relay_error(&e) {
                    debug!(
                        host = %host_lc,
                        port = port,
                        error = %e,
                        "L7 connection closed"
                    );
                } else {
                    warn!(
                        host = %host_lc,
                        port = port,
                        error = %e,
                        "L7 relay error"
                    );
                }
            }
        }
        return Ok(());
    }

    if should_route_via_package_proxy(&endpoint_settings, package_proxy.as_ref(), &decision) {
        let Some(package_proxy) = package_proxy.as_ref() else {
            let reason =
                "package proxy route requested but sandbox package proxy is not configured";
            warn!(dst_host = %host_lc, dst_port = port, reason, "CONNECT blocked");
            emit_denial(
                &denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                reason,
                "package-proxy",
            );
            respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        };
        let egress_profile = match resolve_package_proxy_profile(
            endpoint_settings.egress_profile.as_deref(),
            package_proxy,
        ) {
            Ok(profile) => profile,
            Err(reason) => {
                warn!(dst_host = %host_lc, dst_port = port, reason = %reason, "CONNECT blocked");
                emit_denial(
                    &denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    &reason,
                    "package-proxy",
                );
                respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            }
        };

        let mut upstream = match connect_via_package_proxy(package_proxy, &host, port).await {
            Ok(stream) => stream,
            Err(error) => {
                warn!(
                    dst_host = %host_lc,
                    dst_port = port,
                    upstream_proxy = %package_proxy.upstream_url(),
                    error = %error,
                    "CONNECT upstream package proxy failed"
                );
                respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            }
        };

        respond(&mut client, b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        info!(
            src_addr = %peer_addr.ip(),
            src_port = peer_addr.port(),
            proxy_addr = %local_addr,
            dst_host = %host_lc,
            dst_port = port,
            binary = %binary_str,
            binary_pid = %pid_str,
            ancestors = %ancestors_str,
            cmdline = %cmdline_str,
            action = "allow",
            engine = "opa",
            policy = %policy_str,
            egress_via = "package_proxy",
            egress_profile = %egress_profile,
            upstream_proxy = %package_proxy.upstream_url(),
            resolved_ips = ?resolved_addrs,
            reason = "",
            "CONNECT",
        );
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .into_diagnostic()?;
        return Ok(());
    }

    let mut upstream = match TcpStream::connect(resolved_addrs.as_slice()).await {
        Ok(stream) => stream,
        Err(error) => {
            warn!(dst_host = %host_lc, dst_port = port, error = %error, "CONNECT upstream connect failed");
            respond(&mut client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        }
    };

    respond(&mut client, b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
    info!(
        src_addr = %peer_addr.ip(),
        src_port = peer_addr.port(),
        proxy_addr = %local_addr,
        dst_host = %host_lc,
        dst_port = port,
        binary = %binary_str,
        binary_pid = %pid_str,
        ancestors = %ancestors_str,
        cmdline = %cmdline_str,
        action = "allow",
        engine = "opa",
        policy = %policy_str,
        egress_via = "direct",
        egress_profile = "-",
        upstream_proxy = "-",
        resolved_ips = ?resolved_addrs,
        reason = "",
        connect_msg,
    );

    // L4-only: raw bidirectional copy (existing behavior)
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .into_diagnostic()?;

    Ok(())
}

/// Evaluate OPA policy for a TCP connection with identity binding via /proc/net/tcp.
#[cfg(target_os = "linux")]
fn evaluate_opa_tcp(
    peer_addr: SocketAddr,
    engine: &OpaEngine,
    identity_cache: &BinaryIdentityCache,
    entrypoint_pid: &AtomicU32,
    host: &str,
    port: u16,
) -> ConnectDecision {
    use crate::opa::NetworkInput;
    use std::sync::atomic::Ordering;

    let deny = |reason: String,
                binary: Option<PathBuf>,
                binary_pid: Option<u32>,
                ancestors: Vec<PathBuf>,
                cmdline_paths: Vec<PathBuf>|
     -> ConnectDecision {
        ConnectDecision {
            action: NetworkAction::Deny { reason },
            binary,
            binary_pid,
            ancestors,
            cmdline_paths,
        }
    };

    let pid = entrypoint_pid.load(Ordering::Acquire);
    if pid == 0 {
        return deny(
            "entrypoint process not yet spawned".into(),
            None,
            None,
            vec![],
            vec![],
        );
    }

    let peer_port = peer_addr.port();
    let (bin_path, binary_pid) = match crate::procfs::resolve_tcp_peer_identity(pid, peer_port) {
        Ok(r) => r,
        Err(e) => {
            return deny(
                format!("failed to resolve peer binary: {e}"),
                None,
                None,
                vec![],
                vec![],
            );
        }
    };

    // TOFU verify the immediate binary
    let bin_hash = match identity_cache.verify_or_cache(&bin_path) {
        Ok(h) => h,
        Err(e) => {
            return deny(
                format!("binary integrity check failed: {e}"),
                Some(bin_path),
                Some(binary_pid),
                vec![],
                vec![],
            );
        }
    };

    // Walk the process tree upward to collect ancestor binaries
    let ancestors = crate::procfs::collect_ancestor_binaries(binary_pid, pid);

    // TOFU verify each ancestor binary
    for ancestor in &ancestors {
        if let Err(e) = identity_cache.verify_or_cache(ancestor) {
            return deny(
                format!(
                    "ancestor integrity check failed for {}: {e}",
                    ancestor.display()
                ),
                Some(bin_path),
                Some(binary_pid),
                ancestors.clone(),
                vec![],
            );
        }
    }

    // Collect cmdline paths for script-based binary detection.
    // Excludes exe paths already captured in bin_path/ancestors to avoid duplicates.
    let mut exclude = ancestors.clone();
    exclude.push(bin_path.clone());
    let cmdline_paths = crate::procfs::collect_cmdline_paths(binary_pid, pid, &exclude);

    let input = NetworkInput {
        host: host.to_string(),
        port,
        binary_path: bin_path.clone(),
        binary_sha256: bin_hash,
        ancestors: ancestors.clone(),
        cmdline_paths: cmdline_paths.clone(),
    };

    match engine.evaluate_network_action(&input) {
        Ok(action) => ConnectDecision {
            action,
            binary: Some(bin_path),
            binary_pid: Some(binary_pid),
            ancestors,
            cmdline_paths,
        },
        Err(e) => deny(
            format!("policy evaluation error: {e}"),
            Some(bin_path),
            Some(binary_pid),
            ancestors,
            cmdline_paths,
        ),
    }
}

/// Non-Linux stub: OPA identity binding requires /proc.
#[cfg(not(target_os = "linux"))]
fn evaluate_opa_tcp(
    _peer_addr: SocketAddr,
    _engine: &OpaEngine,
    _identity_cache: &BinaryIdentityCache,
    _entrypoint_pid: &AtomicU32,
    _host: &str,
    _port: u16,
) -> ConnectDecision {
    ConnectDecision {
        action: NetworkAction::Deny {
            reason: "identity binding unavailable on this platform".into(),
        },
        binary: None,
        binary_pid: None,
        ancestors: vec![],
        cmdline_paths: vec![],
    }
}

/// Maximum buffer size for inference request parsing (10 MiB).
const MAX_INFERENCE_BUF: usize = 10 * 1024 * 1024;

/// Initial buffer size for inference request parsing (64 KiB).
const INITIAL_INFERENCE_BUF: usize = 65536;

/// Handle an intercepted connection for inference routing.
///
/// TLS-terminates the client connection, parses HTTP requests, and executes
/// inference API calls locally via `openshell-router`.
/// Non-inference requests are denied with 403.
///
/// Returns [`InferenceOutcome::Routed`] if at least one request was successfully
/// routed, or [`InferenceOutcome::Denied`] with a reason for all denial cases.
async fn handle_inference_interception(
    client: TcpStream,
    host: &str,
    _port: u16,
    tls_state: Option<&Arc<ProxyTlsState>>,
    inference_ctx: Option<&Arc<InferenceContext>>,
) -> Result<InferenceOutcome> {
    use crate::l7::inference::{ParseResult, format_http_response, try_parse_http_request};

    let Some(ctx) = inference_ctx else {
        return Ok(InferenceOutcome::Denied {
            reason: "cluster inference context not configured".to_string(),
        });
    };

    let Some(tls) = tls_state else {
        return Ok(InferenceOutcome::Denied {
            reason: "missing TLS state".to_string(),
        });
    };

    // TLS-terminate the client side (present a cert for the target host)
    let mut tls_client = match crate::l7::tls::tls_terminate_client(client, tls, host).await {
        Ok(c) => c,
        Err(e) => {
            return Ok(InferenceOutcome::Denied {
                reason: format!("TLS handshake failed: {e}"),
            });
        }
    };

    // Read and process HTTP requests from the tunnel.
    // Track whether any request was successfully routed so that a late denial
    // on a keep-alive connection still counts as "routed".
    let mut buf = vec![0u8; INITIAL_INFERENCE_BUF];
    let mut used = 0usize;
    let mut routed_any = false;

    loop {
        let n = match tls_client.read(&mut buf[used..]).await {
            Ok(n) => n,
            Err(e) => {
                if routed_any {
                    break;
                }
                return Ok(InferenceOutcome::Denied {
                    reason: format!("I/O error: {e}"),
                });
            }
        };
        if n == 0 {
            if routed_any {
                break;
            }
            return Ok(InferenceOutcome::Denied {
                reason: "client closed connection".to_string(),
            });
        }
        used += n;

        // Try to parse a complete HTTP request
        match try_parse_http_request(&buf[..used]) {
            ParseResult::Complete(request, consumed) => {
                let was_routed = route_inference_request(&request, ctx, &mut tls_client).await?;
                if was_routed {
                    routed_any = true;
                } else if !routed_any {
                    return Ok(InferenceOutcome::Denied {
                        reason: "connection not allowed by policy".to_string(),
                    });
                }

                // Shift buffer for next request
                buf.copy_within(consumed..used, 0);
                used -= consumed;
            }
            ParseResult::Incomplete => {
                // Need more data — grow buffer if full
                if used == buf.len() {
                    if buf.len() >= MAX_INFERENCE_BUF {
                        let response = format_http_response(413, &[], b"Payload Too Large");
                        write_all(&mut tls_client, &response).await?;
                        if routed_any {
                            break;
                        }
                        return Ok(InferenceOutcome::Denied {
                            reason: "payload too large".to_string(),
                        });
                    }
                    buf.resize((buf.len() * 2).min(MAX_INFERENCE_BUF), 0);
                }
            }
        }
    }

    Ok(InferenceOutcome::Routed)
}

/// Route a parsed inference request locally via the sandbox router, or deny it.
///
/// Returns `Ok(true)` if the request was routed to an inference backend,
/// `Ok(false)` if it was denied as a non-inference request.
async fn route_inference_request(
    request: &crate::l7::inference::ParsedHttpRequest,
    ctx: &InferenceContext,
    tls_client: &mut (impl AsyncWrite + Unpin),
) -> Result<bool> {
    use crate::l7::inference::{detect_inference_pattern, format_http_response};

    let normalized_path = normalize_inference_path(&request.path);

    if let Some(pattern) =
        detect_inference_pattern(&request.method, &normalized_path, &ctx.patterns)
    {
        info!(
            method = %request.method,
            path = %normalized_path,
            protocol = %pattern.protocol,
            kind = %pattern.kind,
            "Intercepted inference request, routing locally"
        );

        // Strip credential + framing/hop-by-hop headers.
        let filtered_headers = sanitize_inference_request_headers(&request.headers);

        let routes = ctx.routes.read().await;

        if routes.is_empty() {
            let body = serde_json::json!({
                "error": "cluster inference is not configured",
                "hint": "run: openshell cluster inference set --help"
            });
            let body_bytes = body.to_string();
            let response = format_http_response(
                503,
                &[("content-type".to_string(), "application/json".to_string())],
                body_bytes.as_bytes(),
            );
            write_all(tls_client, &response).await?;
            return Ok(true);
        }

        match ctx
            .router
            .proxy_with_candidates_streaming(
                &pattern.protocol,
                &request.method,
                &normalized_path,
                filtered_headers,
                bytes::Bytes::from(request.body.clone()),
                &routes,
            )
            .await
        {
            Ok(mut resp) => {
                use crate::l7::inference::{
                    format_chunk, format_chunk_terminator, format_http_response_header,
                };

                let resp_headers = sanitize_inference_response_headers(
                    std::mem::take(&mut resp.headers).into_iter().collect(),
                );

                // Write response headers immediately (chunked TE).
                let header_bytes = format_http_response_header(resp.status, &resp_headers);
                write_all(tls_client, &header_bytes).await?;

                // Stream body chunks as they arrive from the upstream.
                loop {
                    match resp.next_chunk().await {
                        Ok(Some(chunk)) => {
                            let encoded = format_chunk(&chunk);
                            write_all(tls_client, &encoded).await?;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            warn!(error = %e, "error reading upstream response chunk");
                            break;
                        }
                    }
                }

                // Terminate the chunked stream.
                write_all(tls_client, format_chunk_terminator()).await?;
            }
            Err(e) => {
                warn!(error = %e, "inference endpoint detected but upstream service failed");
                let (status, msg) = router_error_to_http(&e);
                let body = serde_json::json!({"error": msg});
                let body_bytes = body.to_string();
                let response = format_http_response(
                    status,
                    &[("content-type".to_string(), "application/json".to_string())],
                    body_bytes.as_bytes(),
                );
                write_all(tls_client, &response).await?;
            }
        }
        Ok(true)
    } else {
        // Not an inference request — deny
        info!(
            method = %request.method,
            path = %normalized_path,
            "connection not allowed by policy"
        );
        let body = serde_json::json!({"error": "connection not allowed by policy"});
        let body_bytes = body.to_string();
        let response = format_http_response(
            403,
            &[("content-type".to_string(), "application/json".to_string())],
            body_bytes.as_bytes(),
        );
        write_all(tls_client, &response).await?;
        Ok(false)
    }
}

fn router_error_to_http(err: &openshell_router::RouterError) -> (u16, String) {
    use openshell_router::RouterError;
    match err {
        RouterError::RouteNotFound(hint) => {
            (400, format!("no route configured for route '{hint}'"))
        }
        RouterError::NoCompatibleRoute(protocol) => (
            400,
            format!("no compatible route for source protocol '{protocol}'"),
        ),
        RouterError::Unauthorized(msg) => (401, msg.clone()),
        RouterError::UpstreamUnavailable(msg) => (503, msg.clone()),
        RouterError::UpstreamProtocol(msg) | RouterError::Internal(msg) => (502, msg.clone()),
    }
}

fn sanitize_inference_request_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| !should_strip_request_header(name))
        .cloned()
        .collect()
}

fn sanitize_inference_response_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, _)| !should_strip_response_header(name))
        .collect()
}

fn should_strip_request_header(name: &str) -> bool {
    let name_lc = name.to_ascii_lowercase();
    matches!(
        name_lc.as_str(),
        "authorization" | "x-api-key" | "host" | "content-length"
    ) || is_hop_by_hop_header(&name_lc)
}

fn should_strip_response_header(name: &str) -> bool {
    let name_lc = name.to_ascii_lowercase();
    matches!(name_lc.as_str(), "content-length") || is_hop_by_hop_header(&name_lc)
}

fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Write all bytes to an async writer.
async fn write_all(writer: &mut (impl AsyncWrite + Unpin), data: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    writer.write_all(data).await.into_diagnostic()?;
    writer.flush().await.into_diagnostic()?;
    Ok(())
}

fn query_endpoint_settings(
    engine: &OpaEngine,
    decision: &ConnectDecision,
    host: &str,
    port: u16,
) -> std::result::Result<EndpointSettings, String> {
    // Only query if action is Allow (not Deny)
    let has_policy = match &decision.action {
        NetworkAction::Allow { matched_policy } => matched_policy.is_some(),
        _ => false,
    };
    if !has_policy {
        return Ok(EndpointSettings::default());
    }

    let input = crate::opa::NetworkInput {
        host: host.to_string(),
        port,
        binary_path: decision.binary.clone().unwrap_or_default(),
        binary_sha256: String::new(),
        ancestors: decision.ancestors.clone(),
        cmdline_paths: decision.cmdline_paths.clone(),
    };

    match engine.query_endpoint_config(&input) {
        Ok(Some(val)) => {
            let egress_via = match endpoint_config_string(&val, "egress_via").as_deref() {
                None | Some("") | Some("direct") => EndpointEgressVia::Direct,
                Some("package_proxy") => EndpointEgressVia::PackageProxy,
                Some(other) => {
                    return Err(format!(
                        "unsupported endpoint egress_via value '{other}' for {host}:{port}"
                    ));
                }
            };
            Ok(EndpointSettings {
                allowed_ips: endpoint_config_strings(&val, "allowed_ips"),
                l7_config: crate::l7::parse_l7_config(&val),
                egress_via,
                egress_profile: endpoint_config_string(&val, "egress_profile"),
                secret_injection: endpoint_config_secret_injection(&val),
            })
        }
        Ok(None) => Ok(EndpointSettings::default()),
        Err(e) => Err(format!("failed to query endpoint config: {e}")),
    }
}

fn endpoint_config_string(value: &regorus::Value, key: &str) -> Option<String> {
    let key = regorus::Value::String(key.into());
    match value {
        regorus::Value::Object(map) => match map.get(&key) {
            Some(regorus::Value::String(s)) if !s.is_empty() => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn endpoint_config_strings(value: &regorus::Value, key: &str) -> Vec<String> {
    let key = regorus::Value::String(key.into());
    match value {
        regorus::Value::Object(map) => match map.get(&key) {
            Some(regorus::Value::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    regorus::Value::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn endpoint_config_bool(value: &regorus::Value, key: &str) -> bool {
    let key = regorus::Value::String(key.into());
    match value {
        regorus::Value::Object(map) => matches!(map.get(&key), Some(regorus::Value::Bool(true))),
        _ => false,
    }
}

fn endpoint_config_secret_injection(value: &regorus::Value) -> Vec<SecretInjectionRule> {
    let key = regorus::Value::String("secret_injection".into());
    match value {
        regorus::Value::Object(map) => match map.get(&key) {
            Some(regorus::Value::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    regorus::Value::Object(_) => Some(SecretInjectionRule {
                        env_var: endpoint_config_string(value, "env_var").unwrap_or_default(),
                        proxy_value: endpoint_config_string(value, "proxy_value")
                            .unwrap_or_default(),
                        match_headers: endpoint_config_strings(value, "match_headers"),
                        match_query: endpoint_config_bool(value, "match_query"),
                        match_body: endpoint_config_bool(value, "match_body"),
                    }),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

async fn resolve_destination_addrs(
    host: &str,
    port: u16,
    allowed_ips: &[String],
) -> std::result::Result<Vec<SocketAddr>, String> {
    if !allowed_ips.is_empty() {
        let nets = parse_allowed_ips(allowed_ips)?;
        resolve_and_check_allowed_ips(host, port, &nets).await
    } else {
        resolve_and_reject_internal(host, port).await
    }
}

fn resolve_package_proxy_profile(
    endpoint_profile: Option<&str>,
    package_proxy: &PackageProxyConfig,
) -> std::result::Result<String, String> {
    match endpoint_profile {
        Some(profile) if profile != package_proxy.profile() => Err(format!(
            "endpoint requests package proxy profile '{profile}' but configured upstream profile is '{}'",
            package_proxy.profile()
        )),
        Some(profile) => Ok(profile.to_string()),
        None => Ok(package_proxy.profile().to_string()),
    }
}

fn build_rest_route_context(
    host: &str,
    port: u16,
    decision: &ConnectDecision,
    endpoint_settings: &EndpointSettings,
    resolved_addrs: &[SocketAddr],
    package_proxy: Option<&PackageProxyConfig>,
    tls_state: Option<&Arc<ProxyTlsState>>,
) -> std::result::Result<RestRouteContext, String> {
    let normal_route = if should_route_via_package_proxy(endpoint_settings, package_proxy, decision)
    {
        let package_proxy = package_proxy.ok_or_else(|| {
            "package proxy route requested but sandbox package proxy is not configured".to_string()
        })?;
        let profile = resolve_package_proxy_profile(
            endpoint_settings.egress_profile.as_deref(),
            package_proxy,
        )?;
        RestNormalRoute::PackageProxy {
            profile,
            upstream_proxy: package_proxy.upstream_url().to_string(),
        }
    } else {
        RestNormalRoute::Direct
    };

    Ok(RestRouteContext {
        host: host.to_string(),
        port,
        tls_mode: endpoint_settings
            .l7_config
            .as_ref()
            .map(|config| config.tls)
            .unwrap_or(crate::l7::TlsMode::Passthrough),
        resolved_addrs: resolved_addrs.to_vec(),
        normal_route,
        package_proxy: package_proxy.cloned(),
        tls_state: tls_state.cloned(),
    })
}

async fn connect_via_package_proxy(
    package_proxy: &PackageProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<BoxedStream> {
    let mut upstream = package_proxy.connect().await?;
    let host_header = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };
    let mut request = format!(
        "CONNECT {host_header} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: openshell-sandbox\r\n"
    );
    if let Some(auth) = package_proxy.authorization() {
        request.push_str("Proxy-Authorization: ");
        request.push_str(auth);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    upstream
        .write_all(request.as_bytes())
        .await
        .into_diagnostic()?;
    upstream.flush().await.into_diagnostic()?;

    let mut buf = vec![0u8; MAX_HEADER_BYTES];
    let mut used = 0usize;
    loop {
        if used == buf.len() {
            return Err(miette::miette!(
                "upstream package proxy response exceeded {MAX_HEADER_BYTES} bytes"
            ));
        }
        let n = upstream.read(&mut buf[used..]).await.into_diagnostic()?;
        if n == 0 {
            return Err(miette::miette!(
                "upstream package proxy closed connection during CONNECT handshake"
            ));
        }
        used += n;
        if buf[..used].windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let response = String::from_utf8_lossy(&buf[..used]);
    let status_line = response.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .parse::<u16>()
        .map_err(|_| {
            miette::miette!("invalid upstream package proxy status line: {status_line}")
        })?;
    if status_code != 200 {
        return Err(miette::miette!(
            "upstream package proxy CONNECT failed with status {status_code}: {status_line}"
        ));
    }

    Ok(upstream)
}

fn rewrite_forward_request_for_upstream_proxy(
    raw: &[u8],
    used: usize,
    package_proxy: &PackageProxyConfig,
) -> Vec<u8> {
    let header_end = raw[..used]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(used, |p| p + 4);
    let header_str = String::from_utf8_lossy(&raw[..header_end]);
    let lines = header_str.split("\r\n").collect::<Vec<_>>();

    let mut output = Vec::with_capacity(header_end + 128);
    let mut has_connection = false;
    let mut has_via = false;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            output.extend_from_slice(line.as_bytes());
            output.extend_from_slice(b"\r\n");
            continue;
        }
        if line.is_empty() {
            break;
        }

        let lower = line.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:")
            || lower.starts_with("proxy-authorization:")
            || lower.starts_with("proxy-authenticate:")
        {
            continue;
        }
        if lower.starts_with("connection:") {
            has_connection = true;
            output.extend_from_slice(b"Connection: close\r\n");
            continue;
        }

        output.extend_from_slice(line.as_bytes());
        output.extend_from_slice(b"\r\n");

        if lower.starts_with("via:") {
            has_via = true;
        }
    }

    if let Some(auth) = package_proxy.authorization() {
        output.extend_from_slice(b"Proxy-Authorization: ");
        output.extend_from_slice(auth.as_bytes());
        output.extend_from_slice(b"\r\n");
    }
    if !has_connection {
        output.extend_from_slice(b"Connection: close\r\n");
    }
    if !has_via {
        output.extend_from_slice(b"Via: 1.1 openshell-sandbox\r\n");
    }
    output.extend_from_slice(b"\r\n");

    if header_end < used {
        output.extend_from_slice(&raw[header_end..used]);
    }

    output
}

/// Check if an IP address is internal (loopback, private RFC1918, or link-local).
///
/// This is a defense-in-depth check to prevent SSRF via the CONNECT proxy.
/// It covers:
/// - IPv4 loopback (127.0.0.0/8), private (10/8, 172.16/12, 192.168/16), link-local (169.254/16)
/// - IPv6 loopback (`::1`), link-local (`fe80::/10`), ULA (`fc00::/7`)
/// - IPv4-mapped IPv6 addresses (`::ffff:x.x.x.x`) are unwrapped and checked as IPv4
fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            // fe80::/10 — IPv6 link-local
            if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // fc00::/7 — IPv6 unique local addresses (ULA)
            if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // Check IPv4-mapped IPv6 (::ffff:x.x.x.x)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            false
        }
    }
}

/// Resolve DNS for a host:port and reject if any resolved address is internal.
///
/// Returns the resolved `SocketAddr` list on success. Returns an error string
/// if any resolved IP is in an internal range or if DNS resolution fails.
async fn resolve_and_reject_internal(
    host: &str,
    port: u16,
) -> std::result::Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS resolution failed for {host}:{port}: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "DNS resolution returned no addresses for {host}:{port}"
        ));
    }

    for addr in &addrs {
        if is_internal_ip(addr.ip()) {
            return Err(format!(
                "{host} resolves to internal address {}, connection rejected",
                addr.ip()
            ));
        }
    }

    Ok(addrs)
}

/// Check if an IP address is always blocked regardless of policy.
///
/// Loopback and link-local addresses are never allowed even when an endpoint
/// has `allowed_ips` configured. This prevents proxy bypass (loopback) and
/// cloud metadata SSRF (link-local 169.254.x.x).
fn is_always_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            // fe80::/10 — IPv6 link-local
            if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // Check IPv4-mapped IPv6 (::ffff:x.x.x.x)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_link_local();
            }
            false
        }
    }
}

/// Resolve DNS and validate resolved addresses against a CIDR/IP allowlist.
///
/// Rejects loopback and link-local unconditionally. For all other resolved
/// addresses, checks that each one matches at least one entry in `allowed_ips`.
/// Entries can be CIDR notation ("10.0.5.0/24") or exact IPs ("10.0.5.20").
///
/// Returns the resolved `SocketAddr` list on success.
async fn resolve_and_check_allowed_ips(
    host: &str,
    port: u16,
    allowed_ips: &[ipnet::IpNet],
) -> std::result::Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS resolution failed for {host}:{port}: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "DNS resolution returned no addresses for {host}:{port}"
        ));
    }

    for addr in &addrs {
        // Always block loopback and link-local
        if is_always_blocked_ip(addr.ip()) {
            return Err(format!(
                "{host} resolves to always-blocked address {}, connection rejected",
                addr.ip()
            ));
        }

        // Check resolved IP against the allowlist
        let ip_allowed = allowed_ips.iter().any(|net| net.contains(&addr.ip()));
        if !ip_allowed {
            return Err(format!(
                "{host} resolves to {} which is not in allowed_ips, connection rejected",
                addr.ip()
            ));
        }
    }

    Ok(addrs)
}

/// Parse CIDR/IP strings into `IpNet` values, rejecting invalid entries and
/// entries that cover loopback or link-local ranges.
///
/// Returns parsed networks on success, or an error describing which entries
/// are invalid.
fn parse_allowed_ips(raw: &[String]) -> std::result::Result<Vec<ipnet::IpNet>, String> {
    let mut nets = Vec::with_capacity(raw.len());
    let mut errors = Vec::new();

    for entry in raw {
        // Try as CIDR first, then as bare IP (convert to /32 or /128)
        let parsed = entry.parse::<ipnet::IpNet>().or_else(|_| {
            entry
                .parse::<IpAddr>()
                .map(|ip| match ip {
                    IpAddr::V4(v4) => ipnet::IpNet::V4(ipnet::Ipv4Net::from(v4)),
                    IpAddr::V6(v6) => ipnet::IpNet::V6(ipnet::Ipv6Net::from(v6)),
                })
                .map_err(|_| ())
        });

        match parsed {
            Ok(n) => nets.push(n),
            Err(_) => errors.push(format!("invalid CIDR/IP in allowed_ips: {entry}")),
        }
    }

    if errors.is_empty() {
        Ok(nets)
    } else {
        Err(errors.join("; "))
    }
}

fn normalize_inference_path(path: &str) -> String {
    if let Some(scheme_idx) = path.find("://") {
        let after_scheme = &path[scheme_idx + 3..];
        if let Some(path_start) = after_scheme.find('/') {
            return after_scheme[path_start..].to_string();
        }
        return "/".to_string();
    }
    path.to_string()
}

/// Extract the hostname from an absolute-form URI used in plain HTTP proxy requests.
///
/// For example, `"http://example.com/path"` yields `"example.com"` and
/// `"http://example.com:8080/path"` yields `"example.com"`. Returns `"unknown"`
/// if the URI cannot be parsed.
#[cfg(test)]
fn extract_host_from_uri(uri: &str) -> String {
    // Absolute-form URIs look like "http://host[:port]/path"
    // Strip the scheme prefix, then extract the authority (host[:port]) before the first '/'.
    let after_scheme = uri.find("://").map(|i| &uri[i + 3..]).unwrap_or(uri);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    // Strip port if present (handle IPv6 bracket notation)
    let host = if authority.starts_with('[') {
        // IPv6: [::1]:port
        authority
            .find(']')
            .map(|i| &authority[..=i])
            .unwrap_or(authority)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    if host.is_empty() {
        "unknown".to_string()
    } else {
        host.to_string()
    }
}

/// Parse an absolute-form proxy request URI into its components.
///
/// For example, `"http://10.86.8.223:8000/screenshot/"` yields
/// `("http", "10.86.8.223", 8000, "/screenshot/")`.
///
/// Handles:
/// - Default port 80 for `http`, 443 for `https`
/// - IPv6 bracket notation (`[::1]`)
/// - Missing path (defaults to `/`)
/// - Query strings (preserved in path)
fn parse_proxy_uri(uri: &str) -> Result<(String, String, u16, String)> {
    // Extract scheme
    let (scheme, rest) = uri
        .split_once("://")
        .ok_or_else(|| miette::miette!("Missing scheme in proxy URI: {uri}"))?;
    let scheme = scheme.to_ascii_lowercase();

    // Split authority from path
    let (authority, path) = if rest.starts_with('[') {
        // IPv6: [::1]:port/path
        let bracket_end = rest
            .find(']')
            .ok_or_else(|| miette::miette!("Unclosed IPv6 bracket in URI: {uri}"))?;
        let after_bracket = &rest[bracket_end + 1..];
        if let Some(slash_pos) = after_bracket.find('/') {
            (
                &rest[..bracket_end + 1 + slash_pos],
                &after_bracket[slash_pos..],
            )
        } else {
            (&rest[..], "/")
        }
    } else if let Some(slash_pos) = rest.find('/') {
        (&rest[..slash_pos], &rest[slash_pos..])
    } else {
        (rest, "/")
    };

    // Parse host and port from authority
    let (host, port) = if authority.starts_with('[') {
        // IPv6: [::1]:port or [::1]
        let bracket_end = authority
            .find(']')
            .ok_or_else(|| miette::miette!("Unclosed IPv6 bracket: {uri}"))?;
        let host = &authority[1..bracket_end]; // strip brackets
        let port_str = &authority[bracket_end + 1..];
        let port = if let Some(port_str) = port_str.strip_prefix(':') {
            port_str
                .parse::<u16>()
                .map_err(|_| miette::miette!("Invalid port in URI: {uri}"))?
        } else {
            match scheme.as_str() {
                "https" => 443,
                _ => 80,
            }
        };
        (host.to_string(), port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| miette::miette!("Invalid port in URI: {uri}"))?;
        (h.to_string(), port)
    } else {
        let port = match scheme.as_str() {
            "https" => 443,
            _ => 80,
        };
        (authority.to_string(), port)
    };

    if host.is_empty() {
        return Err(miette::miette!("Empty host in URI: {uri}"));
    }

    let path = if path.is_empty() { "/" } else { path };

    Ok((scheme, host, port, path.to_string()))
}

/// Rewrite an absolute-form HTTP proxy request to origin-form for upstream.
///
/// Transforms `GET http://host:port/path HTTP/1.1` into `GET /path HTTP/1.1`,
/// strips proxy hop-by-hop headers, injects `Connection: close` and `Via`.
///
/// Returns the rewritten request bytes (headers + any overflow body bytes).
fn rewrite_forward_request(raw: &[u8], used: usize, path: &str) -> Vec<u8> {
    let header_end = raw[..used]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(used, |p| p + 4);

    let header_str = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_str.split("\r\n").collect::<Vec<_>>();

    // Rewrite request line: METHOD absolute-uri HTTP/1.1 → METHOD path HTTP/1.1
    if let Some(first_line) = lines.first_mut() {
        let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
        if parts.len() == 3 {
            let new_line = format!("{} {} {}", parts[0], path, parts[2]);
            *first_line = Box::leak(new_line.into_boxed_str()); // safe: short-lived
        }
    }

    // Rebuild headers, stripping hop-by-hop and adding proxy headers
    let mut output = Vec::with_capacity(header_end + 128);
    let mut has_connection = false;
    let mut has_via = false;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            // Request line — already rewritten
            output.extend_from_slice(line.as_bytes());
            output.extend_from_slice(b"\r\n");
            continue;
        }
        if line.is_empty() {
            // End of headers
            break;
        }

        let lower = line.to_ascii_lowercase();

        // Strip proxy hop-by-hop headers
        if lower.starts_with("proxy-connection:")
            || lower.starts_with("proxy-authorization:")
            || lower.starts_with("proxy-authenticate:")
        {
            continue;
        }

        // Replace Connection header
        if lower.starts_with("connection:") {
            has_connection = true;
            output.extend_from_slice(b"Connection: close\r\n");
            continue;
        }

        output.extend_from_slice(line.as_bytes());
        output.extend_from_slice(b"\r\n");

        if lower.starts_with("via:") {
            has_via = true;
        }
    }

    // Inject missing headers
    if !has_connection {
        output.extend_from_slice(b"Connection: close\r\n");
    }
    if !has_via {
        output.extend_from_slice(b"Via: 1.1 openshell-sandbox\r\n");
    }

    // End of headers
    output.extend_from_slice(b"\r\n");

    // Append any overflow body bytes from the original buffer
    if header_end < used {
        output.extend_from_slice(&raw[header_end..used]);
    }

    output
}

/// Handle a plain HTTP forward proxy request (non-CONNECT).
///
/// Public IPs are allowed through when the endpoint passes OPA evaluation.
/// Private IPs require explicit `allowed_ips` on the endpoint config (SSRF
/// override). Rewrites the absolute-form request to origin-form, connects
/// upstream, and relays the response using `copy_bidirectional` for streaming.
async fn handle_forward_proxy(
    method: &str,
    target_uri: &str,
    buf: &[u8],
    used: usize,
    client: &mut TcpStream,
    opa_engine: Arc<OpaEngine>,
    identity_cache: Arc<BinaryIdentityCache>,
    entrypoint_pid: Arc<AtomicU32>,
    package_proxy: Option<PackageProxyConfig>,
    denial_tx: Option<&mpsc::UnboundedSender<DenialEvent>>,
) -> Result<()> {
    // 1. Parse the absolute-form URI
    let (scheme, host, port, path) = match parse_proxy_uri(target_uri) {
        Ok(parsed) => parsed,
        Err(e) => {
            warn!(target_uri = %target_uri, error = %e, "FORWARD parse error");
            respond(client, b"HTTP/1.1 400 Bad Request\r\n\r\n").await?;
            return Ok(());
        }
    };
    let host_lc = host.to_ascii_lowercase();

    // 2. Reject HTTPS — must use CONNECT for TLS
    if scheme == "https" {
        info!(
            dst_host = %host_lc,
            dst_port = port,
            "FORWARD rejected: HTTPS requires CONNECT"
        );
        respond(
            client,
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 27\r\n\r\nUse CONNECT for HTTPS URLs",
        )
        .await?;
        return Ok(());
    }

    // 3. Evaluate OPA policy (same identity binding as CONNECT)
    let peer_addr = client.peer_addr().into_diagnostic()?;
    let local_addr = client.local_addr().into_diagnostic()?;

    let decision = evaluate_opa_tcp(
        peer_addr,
        &opa_engine,
        &identity_cache,
        &entrypoint_pid,
        &host_lc,
        port,
    );

    // Build log context
    let binary_str = decision
        .binary
        .as_ref()
        .map_or_else(|| "-".to_string(), |p| p.display().to_string());
    let pid_str = decision
        .binary_pid
        .map_or_else(|| "-".to_string(), |p| p.to_string());
    let ancestors_str = if decision.ancestors.is_empty() {
        "-".to_string()
    } else {
        decision
            .ancestors
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ")
    };
    let cmdline_str = if decision.cmdline_paths.is_empty() {
        "-".to_string()
    } else {
        decision
            .cmdline_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };

    // 4. Only proceed on explicit Allow — reject Deny
    let matched_policy = match &decision.action {
        NetworkAction::Allow { matched_policy } => matched_policy.clone(),
        NetworkAction::Deny { reason } => {
            info!(
                src_addr = %peer_addr.ip(),
                src_port = peer_addr.port(),
                proxy_addr = %local_addr,
                dst_host = %host_lc,
                dst_port = port,
                method = %method,
                path = %path,
                binary = %binary_str,
                binary_pid = %pid_str,
                ancestors = %ancestors_str,
                cmdline = %cmdline_str,
                action = "deny",
                engine = "opa",
                policy = "-",
                reason = %reason,
                "FORWARD",
            );
            emit_denial_simple(
                denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                reason,
                "forward",
            );
            respond(client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
            return Ok(());
        }
    };
    let policy_str = matched_policy.as_deref().unwrap_or("-");

    let endpoint_settings = match query_endpoint_settings(&opa_engine, &decision, &host_lc, port) {
        Ok(settings) => settings,
        Err(reason) => {
            warn!(
                dst_host = %host_lc,
                dst_port = port,
                reason = %reason,
                "FORWARD blocked: invalid endpoint config"
            );
            emit_denial_simple(
                denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                &reason,
                "endpoint-config",
            );
            respond(client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        }
    };

    // 4b. Reject if the endpoint has L7 config — the forward proxy path does
    //     not perform per-request method/path inspection, so L7-configured
    //     endpoints must go through the CONNECT tunnel where inspection happens.
    if endpoint_settings.l7_config.is_some() {
        info!(
            dst_host = %host_lc,
            dst_port = port,
            method = %method,
            path = %path,
            binary = %binary_str,
            policy = %policy_str,
            action = "deny",
            reason = "endpoint has L7 rules; use CONNECT",
            "FORWARD",
        );
        emit_denial_simple(
            denial_tx,
            &host_lc,
            port,
            &binary_str,
            &decision,
            "endpoint has L7 rules configured; forward proxy bypasses L7 inspection — use CONNECT",
            "forward-l7-bypass",
        );
        respond(client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
        return Ok(());
    }

    let resolved_addrs =
        match resolve_destination_addrs(&host, port, &endpoint_settings.allowed_ips).await {
            Ok(addrs) => addrs,
            Err(reason) => {
                warn!(
                    dst_host = %host_lc,
                    dst_port = port,
                    reason = %reason,
                    "FORWARD blocked: destination validation failed"
                );
                emit_denial_simple(
                    denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    &reason,
                    "ssrf",
                );
                respond(client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
                return Ok(());
            }
        };

    if should_route_via_package_proxy(&endpoint_settings, package_proxy.as_ref(), &decision) {
        let Some(package_proxy) = package_proxy.as_ref() else {
            let reason =
                "package proxy route requested but sandbox package proxy is not configured";
            emit_denial_simple(
                denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                reason,
                "package-proxy",
            );
            respond(client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        };
        let egress_profile = match resolve_package_proxy_profile(
            endpoint_settings.egress_profile.as_deref(),
            package_proxy,
        ) {
            Ok(profile) => profile,
            Err(reason) => {
                emit_denial_simple(
                    denial_tx,
                    &host_lc,
                    port,
                    &binary_str,
                    &decision,
                    &reason,
                    "package-proxy",
                );
                respond(client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            }
        };
        let mut upstream = match package_proxy.connect().await {
            Ok(stream) => stream,
            Err(error) => {
                warn!(
                    dst_host = %host_lc,
                    dst_port = port,
                    upstream_proxy = %package_proxy.upstream_url(),
                    error = %error,
                    "FORWARD upstream package proxy failed"
                );
                respond(client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
                return Ok(());
            }
        };
        info!(
            src_addr = %peer_addr.ip(),
            src_port = peer_addr.port(),
            proxy_addr = %local_addr,
            dst_host = %host_lc,
            dst_port = port,
            method = %method,
            path = %path,
            binary = %binary_str,
            binary_pid = %pid_str,
            ancestors = %ancestors_str,
            cmdline = %cmdline_str,
            action = "allow",
            engine = "opa",
            policy = %policy_str,
            egress_via = "package_proxy",
            egress_profile = %egress_profile,
            upstream_proxy = %package_proxy.upstream_url(),
            resolved_ips = ?resolved_addrs,
            reason = "",
            "FORWARD",
        );
        let rewritten = rewrite_forward_request_for_upstream_proxy(buf, used, package_proxy);
        if contains_placeholder_bytes(&rewritten) {
            let reason = "forward proxy placeholder rewriting is no longer supported; use CONNECT with protocol: rest and tls: terminate";
            emit_denial_simple(
                denial_tx,
                &host_lc,
                port,
                &binary_str,
                &decision,
                reason,
                "forward-placeholder",
            );
            respond(client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
            return Ok(());
        }
        upstream.write_all(&rewritten).await.into_diagnostic()?;
        let _ = tokio::io::copy_bidirectional(client, &mut upstream)
            .await
            .into_diagnostic()?;
        return Ok(());
    }

    let mut upstream = match TcpStream::connect(resolved_addrs.as_slice()).await {
        Ok(stream) => stream,
        Err(error) => {
            warn!(
                dst_host = %host_lc,
                dst_port = port,
                error = %error,
                "FORWARD upstream connect failed"
            );
            respond(client, b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        }
    };

    info!(
        src_addr = %peer_addr.ip(),
        src_port = peer_addr.port(),
        proxy_addr = %local_addr,
        dst_host = %host_lc,
        dst_port = port,
        method = %method,
        path = %path,
        binary = %binary_str,
        binary_pid = %pid_str,
        ancestors = %ancestors_str,
        cmdline = %cmdline_str,
        action = "allow",
        engine = "opa",
        policy = %policy_str,
        egress_via = "direct",
        egress_profile = "-",
        upstream_proxy = "-",
        resolved_ips = ?resolved_addrs,
        reason = "",
        "FORWARD",
    );

    let rewritten = rewrite_forward_request(buf, used, &path);
    if contains_placeholder_bytes(&rewritten) {
        let reason = "forward proxy placeholder rewriting is no longer supported; use CONNECT with protocol: rest and tls: terminate";
        emit_denial_simple(
            denial_tx,
            &host_lc,
            port,
            &binary_str,
            &decision,
            reason,
            "forward-placeholder",
        );
        respond(client, b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
        return Ok(());
    }
    upstream.write_all(&rewritten).await.into_diagnostic()?;
    let _ = tokio::io::copy_bidirectional(client, &mut upstream)
        .await
        .into_diagnostic()?;
    Ok(())
}

fn parse_target(target: &str) -> Result<(String, u16)> {
    let (host, port_str) = target
        .split_once(':')
        .ok_or_else(|| miette::miette!("CONNECT target missing port: {target}"))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| miette::miette!("Invalid port in CONNECT target: {target}"))?;
    Ok((host.to_string(), port))
}

async fn respond(client: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    client.write_all(bytes).await.into_diagnostic()?;
    Ok(())
}

/// Check if a miette error represents a benign connection close.
///
/// TLS handshake EOF, missing `close_notify`, connection resets, and broken
/// pipes are all normal lifecycle events for proxied connections — not worth
/// a WARN that interrupts the user's terminal.
fn is_benign_relay_error(err: &miette::Report) -> bool {
    const BENIGN: &[&str] = &[
        "close_notify",
        "tls handshake eof",
        "connection reset",
        "broken pipe",
        "unexpected eof",
    ];
    let msg = err.to_string().to_ascii_lowercase();
    BENIGN.iter().any(|pat| msg.contains(pat))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use temp_env::with_vars;

    // -- is_internal_ip: IPv4 --

    #[test]
    fn test_rejects_ipv4_loopback() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))));
    }

    #[test]
    fn test_rejects_ipv4_private_10() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(10, 255, 255, 255))));
    }

    #[test]
    fn test_rejects_ipv4_private_172_16() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255))));
    }

    #[test]
    fn test_rejects_ipv4_private_192_168() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 255, 255
        ))));
    }

    #[test]
    fn test_rejects_ipv4_link_local_metadata() {
        // Cloud metadata endpoint
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
    }

    #[test]
    fn test_allows_ipv4_public() {
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
    }

    #[test]
    fn test_allows_ipv4_non_private_172() {
        // 172.32.0.0 is outside the 172.16/12 private range
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1))));
    }

    // -- is_internal_ip: IPv6 --

    #[test]
    fn test_rejects_ipv6_loopback() {
        assert!(is_internal_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_rejects_ipv6_link_local() {
        // fe80::1
        assert!(is_internal_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn test_rejects_ipv6_unique_local_address() {
        // fdc4:f303:9324::254
        assert!(is_internal_ip(IpAddr::V6(Ipv6Addr::new(
            0xfdc4, 0xf303, 0x9324, 0, 0, 0, 0, 0x0254
        ))));
    }

    #[test]
    fn test_rejects_ipv4_mapped_ipv6_private() {
        // ::ffff:10.0.0.1
        let v6 = Ipv4Addr::new(10, 0, 0, 1).to_ipv6_mapped();
        assert!(is_internal_ip(IpAddr::V6(v6)));
    }

    #[test]
    fn test_rejects_ipv4_mapped_ipv6_loopback() {
        // ::ffff:127.0.0.1
        let v6 = Ipv4Addr::LOCALHOST.to_ipv6_mapped();
        assert!(is_internal_ip(IpAddr::V6(v6)));
    }

    #[test]
    fn test_rejects_ipv4_mapped_ipv6_link_local() {
        // ::ffff:169.254.169.254
        let v6 = Ipv4Addr::new(169, 254, 169, 254).to_ipv6_mapped();
        assert!(is_internal_ip(IpAddr::V6(v6)));
    }

    #[test]
    fn test_allows_ipv6_public() {
        // 2001:4860:4860::8888 (Google DNS)
        assert!(!is_internal_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    #[test]
    fn test_allows_ipv4_mapped_ipv6_public() {
        // ::ffff:8.8.8.8
        let v6 = Ipv4Addr::new(8, 8, 8, 8).to_ipv6_mapped();
        assert!(!is_internal_ip(IpAddr::V6(v6)));
    }

    // -- resolve_and_reject_internal --

    #[tokio::test]
    async fn test_rejects_localhost_resolution() {
        let result = resolve_and_reject_internal("localhost", 80).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("internal address"),
            "expected 'internal address' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_rejects_loopback_ip_literal() {
        let result = resolve_and_reject_internal("127.0.0.1", 443).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("internal address"),
            "expected 'internal address' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_rejects_metadata_ip() {
        let result = resolve_and_reject_internal("169.254.169.254", 80).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("internal address"),
            "expected 'internal address' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_dns_failure_returns_error() {
        let result = resolve_and_reject_internal("this-host-does-not-exist.invalid", 80).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("DNS resolution failed"),
            "expected 'DNS resolution failed' in error: {err}"
        );
    }

    #[test]
    fn sanitize_request_headers_strips_auth_and_framing() {
        let headers = vec![
            ("authorization".to_string(), "Bearer test".to_string()),
            ("x-api-key".to_string(), "secret".to_string()),
            ("transfer-encoding".to_string(), "chunked".to_string()),
            ("content-length".to_string(), "42".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
            ("accept".to_string(), "text/event-stream".to_string()),
        ];

        let kept = sanitize_inference_request_headers(&headers);

        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")),
            "authorization should be stripped"
        );
        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("x-api-key")),
            "x-api-key should be stripped"
        );
        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("transfer-encoding")),
            "transfer-encoding should be stripped"
        );
        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("content-length")),
            "content-length should be stripped"
        );
        assert!(
            kept.iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type")),
            "content-type should be preserved"
        );
        assert!(
            kept.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")),
            "accept should be preserved"
        );
    }

    // -- router_error_to_http --

    #[test]
    fn router_error_route_not_found_maps_to_400() {
        let err = openshell_router::RouterError::RouteNotFound("local".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 400);
        assert!(
            msg.contains("local"),
            "message should contain the hint: {msg}"
        );
    }

    #[test]
    fn router_error_no_compatible_route_maps_to_400() {
        let err = openshell_router::RouterError::NoCompatibleRoute("anthropic_messages".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 400);
        assert!(
            msg.contains("anthropic_messages"),
            "message should contain the protocol: {msg}"
        );
    }

    #[test]
    fn router_error_unauthorized_maps_to_401() {
        let err = openshell_router::RouterError::Unauthorized("bad token".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 401);
        assert_eq!(msg, "bad token");
    }

    #[test]
    fn router_error_upstream_unavailable_maps_to_503() {
        let err = openshell_router::RouterError::UpstreamUnavailable("connection refused".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 503);
        assert_eq!(msg, "connection refused");
    }

    #[test]
    fn router_error_upstream_protocol_maps_to_502() {
        let err = openshell_router::RouterError::UpstreamProtocol("bad gateway".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 502);
        assert_eq!(msg, "bad gateway");
    }

    #[test]
    fn router_error_internal_maps_to_502() {
        let err = openshell_router::RouterError::Internal("unexpected".into());
        let (status, msg) = router_error_to_http(&err);
        assert_eq!(status, 502);
        assert_eq!(msg, "unexpected");
    }

    #[test]
    fn sanitize_response_headers_strips_hop_by_hop() {
        let headers = vec![
            ("transfer-encoding".to_string(), "chunked".to_string()),
            ("content-length".to_string(), "128".to_string()),
            ("connection".to_string(), "keep-alive".to_string()),
            ("content-type".to_string(), "text/event-stream".to_string()),
            ("cache-control".to_string(), "no-cache".to_string()),
        ];

        let kept = sanitize_inference_response_headers(headers);

        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("transfer-encoding")),
            "transfer-encoding should be stripped"
        );
        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("content-length")),
            "content-length should be stripped"
        );
        assert!(
            kept.iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("connection")),
            "connection should be stripped"
        );
        assert!(
            kept.iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type")),
            "content-type should be preserved"
        );
        assert!(
            kept.iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("cache-control")),
            "cache-control should be preserved"
        );
    }

    // -- is_always_blocked_ip --

    #[test]
    fn test_always_blocked_loopback_v4() {
        assert!(is_always_blocked_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            127, 0, 0, 2
        ))));
    }

    #[test]
    fn test_always_blocked_link_local_v4() {
        assert!(is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 0, 1
        ))));
    }

    #[test]
    fn test_always_blocked_loopback_v6() {
        assert!(is_always_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_always_blocked_link_local_v6() {
        assert!(is_always_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn test_always_blocked_ipv4_mapped_v6_loopback() {
        let v6 = Ipv4Addr::LOCALHOST.to_ipv6_mapped();
        assert!(is_always_blocked_ip(IpAddr::V6(v6)));
    }

    #[test]
    fn test_always_blocked_ipv4_mapped_v6_link_local() {
        let v6 = Ipv4Addr::new(169, 254, 169, 254).to_ipv6_mapped();
        assert!(is_always_blocked_ip(IpAddr::V6(v6)));
    }

    #[test]
    fn test_always_blocked_allows_rfc1918() {
        // RFC 1918 addresses should NOT be always-blocked (they're allowed
        // when allowed_ips is configured)
        assert!(!is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            10, 0, 0, 1
        ))));
        assert!(!is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 1
        ))));
        assert!(!is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 0, 1
        ))));
    }

    #[test]
    fn test_always_blocked_allows_public() {
        assert!(!is_always_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_always_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    // -- parse_allowed_ips --

    #[test]
    fn test_parse_cidr_notation() {
        let raw = vec!["10.0.5.0/24".to_string()];
        let nets = parse_allowed_ips(&raw).unwrap();
        assert_eq!(nets.len(), 1);
        assert!(nets[0].contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 5, 1))));
        assert!(!nets[0].contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 6, 1))));
    }

    #[test]
    fn test_parse_exact_ip() {
        let raw = vec!["10.0.5.20".to_string()];
        let nets = parse_allowed_ips(&raw).unwrap();
        assert_eq!(nets.len(), 1);
        assert!(nets[0].contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 5, 20))));
        assert!(!nets[0].contains(&IpAddr::V4(Ipv4Addr::new(10, 0, 5, 21))));
    }

    #[test]
    fn test_parse_multiple_entries() {
        let raw = vec![
            "10.0.0.0/8".to_string(),
            "172.16.0.0/12".to_string(),
            "192.168.1.1".to_string(),
        ];
        let nets = parse_allowed_ips(&raw).unwrap();
        assert_eq!(nets.len(), 3);
    }

    #[test]
    fn test_parse_invalid_entry_errors() {
        let raw = vec!["not-an-ip".to_string()];
        let result = parse_allowed_ips(&raw);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid CIDR/IP"));
    }

    #[test]
    fn test_parse_mixed_valid_invalid_errors() {
        let raw = vec!["10.0.5.0/24".to_string(), "garbage".to_string()];
        let result = parse_allowed_ips(&raw);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_check_allowed_ips_blocks_loopback() {
        let nets = parse_allowed_ips(&["127.0.0.0/8".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("127.0.0.1", 80, &nets).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("always-blocked"),
            "expected 'always-blocked' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_resolve_check_allowed_ips_blocks_metadata() {
        let nets = parse_allowed_ips(&["169.254.0.0/16".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("169.254.169.254", 80, &nets).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("always-blocked"),
            "expected 'always-blocked' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_resolve_check_allowed_ips_rejects_outside_allowlist() {
        // 8.8.8.8 resolves to a public IP which is NOT in 10.0.0.0/8
        let nets = parse_allowed_ips(&["10.0.0.0/8".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("dns.google", 443, &nets).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not in allowed_ips"),
            "expected 'not in allowed_ips' in error: {err}"
        );
    }

    // --- extract_host_from_uri tests ---

    #[test]
    fn test_extract_host_from_http_uri() {
        assert_eq!(
            extract_host_from_uri("http://example.com/path"),
            "example.com"
        );
    }

    #[test]
    fn test_extract_host_from_https_uri() {
        assert_eq!(
            extract_host_from_uri("https://api.openai.com/v1/chat/completions"),
            "api.openai.com"
        );
    }

    #[test]
    fn test_extract_host_from_uri_with_port() {
        assert_eq!(
            extract_host_from_uri("http://example.com:8080/path"),
            "example.com"
        );
    }

    #[test]
    fn test_extract_host_from_uri_ipv6() {
        assert_eq!(extract_host_from_uri("http://[::1]:8080/path"), "[::1]");
    }

    #[test]
    fn test_extract_host_from_uri_no_path() {
        assert_eq!(extract_host_from_uri("http://example.com"), "example.com");
    }

    #[test]
    fn test_extract_host_from_uri_empty() {
        assert_eq!(extract_host_from_uri(""), "unknown");
    }

    #[test]
    fn test_extract_host_from_uri_malformed() {
        // Gracefully handles garbage input
        let result = extract_host_from_uri("not-a-uri");
        assert!(!result.is_empty());
    }

    // --- parse_proxy_uri tests ---

    #[test]
    fn test_parse_proxy_uri_standard() {
        let (scheme, host, port, path) =
            parse_proxy_uri("http://10.86.8.223:8000/screenshot/").unwrap();
        assert_eq!(scheme, "http");
        assert_eq!(host, "10.86.8.223");
        assert_eq!(port, 8000);
        assert_eq!(path, "/screenshot/");
    }

    #[test]
    fn test_parse_proxy_uri_default_port() {
        let (scheme, host, port, path) = parse_proxy_uri("http://example.com/path").unwrap();
        assert_eq!(scheme, "http");
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert_eq!(path, "/path");
    }

    #[test]
    fn test_parse_proxy_uri_https_default_port() {
        let (scheme, host, port, path) =
            parse_proxy_uri("https://api.example.com/v1/chat").unwrap();
        assert_eq!(scheme, "https");
        assert_eq!(host, "api.example.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/v1/chat");
    }

    #[test]
    fn test_parse_proxy_uri_missing_path() {
        let (_, host, port, path) = parse_proxy_uri("http://10.0.0.1:9090").unwrap();
        assert_eq!(host, "10.0.0.1");
        assert_eq!(port, 9090);
        assert_eq!(path, "/");
    }

    #[test]
    fn test_parse_proxy_uri_with_query() {
        let (_, _, _, path) = parse_proxy_uri("http://host:80/api?key=val&foo=bar").unwrap();
        assert_eq!(path, "/api?key=val&foo=bar");
    }

    #[test]
    fn test_parse_proxy_uri_ipv6() {
        let (_, host, port, path) = parse_proxy_uri("http://[::1]:8080/test").unwrap();
        assert_eq!(host, "::1");
        assert_eq!(port, 8080);
        assert_eq!(path, "/test");
    }

    #[test]
    fn test_parse_proxy_uri_ipv6_default_port() {
        let (_, host, port, path) = parse_proxy_uri("http://[fe80::1]/path").unwrap();
        assert_eq!(host, "fe80::1");
        assert_eq!(port, 80);
        assert_eq!(path, "/path");
    }

    #[test]
    fn test_parse_proxy_uri_missing_scheme() {
        let result = parse_proxy_uri("example.com/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_proxy_uri_empty_host() {
        let result = parse_proxy_uri("http:///path");
        assert!(result.is_err());
    }

    // --- rewrite_forward_request tests ---

    #[test]
    fn test_rewrite_get_request() {
        let raw =
            b"GET http://10.0.0.1:8000/api HTTP/1.1\r\nHost: 10.0.0.1:8000\r\nAccept: */*\r\n\r\n";
        let result = rewrite_forward_request(raw, raw.len(), "/api");
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(result_str.contains("Host: 10.0.0.1:8000"));
        assert!(result_str.contains("Connection: close"));
        assert!(result_str.contains("Via: 1.1 openshell-sandbox"));
    }

    #[test]
    fn test_rewrite_strips_proxy_headers() {
        let raw = b"GET http://host/p HTTP/1.1\r\nHost: host\r\nProxy-Authorization: Basic abc\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n";
        let result = rewrite_forward_request(raw, raw.len(), "/p");
        let result_str = String::from_utf8_lossy(&result);
        assert!(
            !result_str
                .to_ascii_lowercase()
                .contains("proxy-authorization")
        );
        assert!(!result_str.to_ascii_lowercase().contains("proxy-connection"));
        assert!(result_str.contains("Accept: */*"));
    }

    #[test]
    fn test_rewrite_replaces_connection_header() {
        let raw = b"GET http://host/p HTTP/1.1\r\nHost: host\r\nConnection: keep-alive\r\n\r\n";
        let result = rewrite_forward_request(raw, raw.len(), "/p");
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("Connection: close"));
        assert!(!result_str.contains("keep-alive"));
    }

    #[test]
    fn test_rewrite_preserves_body_overflow() {
        let raw = b"POST http://host/api HTTP/1.1\r\nHost: host\r\nContent-Length: 13\r\n\r\n{\"key\":\"val\"}";
        let result = rewrite_forward_request(raw, raw.len(), "/api");
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("{\"key\":\"val\"}"));
        assert!(result_str.contains("POST /api HTTP/1.1"));
    }

    #[test]
    fn test_rewrite_preserves_existing_via() {
        let raw = b"GET http://host/p HTTP/1.1\r\nHost: host\r\nVia: 1.0 upstream\r\n\r\n";
        let result = rewrite_forward_request(raw, raw.len(), "/p");
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("Via: 1.0 upstream"));
        // Should not add a second Via header
        assert!(!result_str.contains("Via: 1.1 openshell-sandbox"));
    }

    #[test]
    fn test_rewrite_preserves_placeholder_auth_headers() {
        let raw = b"GET http://host/p HTTP/1.1\r\nHost: host\r\nAuthorization: Bearer openshell:resolve:env:ANTHROPIC_API_KEY\r\n\r\n";
        let result = rewrite_forward_request(raw, raw.len(), "/p");
        let result_str = String::from_utf8_lossy(&result);
        assert!(
            result_str.contains("Authorization: Bearer openshell:resolve:env:ANTHROPIC_API_KEY")
        );
        assert!(contains_placeholder_bytes(&result));
    }

    #[test]
    fn package_proxy_config_parses_env() {
        let auth_file = tempfile::NamedTempFile::new().expect("temp auth file");
        std::fs::write(auth_file.path(), "Bearer proxy-token\n").expect("write auth file");

        with_vars(
            vec![
                (PACKAGE_PROXY_ENABLED_ENV, Some("1")),
                (PACKAGE_PROXY_PROFILE_ENV, Some("socket")),
                (
                    PACKAGE_PROXY_UPSTREAM_URL_ENV,
                    Some("http://proxy.socket.dev:8080"),
                ),
                (
                    PACKAGE_PROXY_AUTHORIZATION_FILE_ENV,
                    auth_file.path().to_str(),
                ),
                (PACKAGE_PROXY_CA_FILE_ENV, None),
            ],
            || {
                let config = PackageProxyConfig::from_env()
                    .expect("config parse")
                    .expect("config should exist");
                assert_eq!(config.profile(), "socket");
                assert_eq!(config.upstream_url(), "http://proxy.socket.dev:8080");
                assert_eq!(config.authorization(), Some("Bearer proxy-token"));
                assert!(config.extra_ca_paths().is_empty());
            },
        );
    }

    #[test]
    fn package_proxy_config_requires_upstream_url_when_enabled() {
        with_vars(
            vec![
                (PACKAGE_PROXY_ENABLED_ENV, Some("1")),
                (PACKAGE_PROXY_UPSTREAM_URL_ENV, None),
                (PACKAGE_PROXY_PROFILE_ENV, None),
                (PACKAGE_PROXY_CA_FILE_ENV, None),
                (PACKAGE_PROXY_AUTHORIZATION_FILE_ENV, None),
            ],
            || {
                let error = PackageProxyConfig::from_env().expect_err("missing URL should fail");
                assert!(error.to_string().contains(PACKAGE_PROXY_UPSTREAM_URL_ENV));
            },
        );
    }

    #[test]
    fn package_proxy_routes_known_package_manager_binaries_when_enabled() {
        let decision = ConnectDecision {
            action: NetworkAction::Allow {
                matched_policy: Some("allow".to_string()),
            },
            binary: Some(PathBuf::from("/usr/bin/npm")),
            binary_pid: Some(1234),
            ancestors: Vec::new(),
            cmdline_paths: Vec::new(),
        };
        let endpoint_settings = EndpointSettings::default();
        let package_proxy = PackageProxyConfig {
            profile: "generic".to_string(),
            upstream_url: "http://proxy.socket.dev:8080".to_string(),
            upstream_host: "proxy.socket.dev".to_string(),
            upstream_port: 8080,
            scheme: PackageProxyScheme::Http,
            authorization: None,
            extra_ca_paths: Vec::new(),
            upstream_tls_config: None,
        };

        assert!(should_route_via_package_proxy(
            &endpoint_settings,
            Some(&package_proxy),
            &decision,
        ));
    }

    #[test]
    fn package_proxy_routes_known_package_manager_cmdline_paths_when_enabled() {
        let decision = ConnectDecision {
            action: NetworkAction::Allow {
                matched_policy: Some("allow".to_string()),
            },
            binary: Some(PathBuf::from("/usr/bin/node")),
            binary_pid: Some(1234),
            ancestors: Vec::new(),
            cmdline_paths: vec![PathBuf::from("/usr/lib/node_modules/npm/bin/npm-cli.js")],
        };
        let endpoint_settings = EndpointSettings::default();
        let package_proxy = PackageProxyConfig {
            profile: "generic".to_string(),
            upstream_url: "http://proxy.socket.dev:8080".to_string(),
            upstream_host: "proxy.socket.dev".to_string(),
            upstream_port: 8080,
            scheme: PackageProxyScheme::Http,
            authorization: None,
            extra_ca_paths: Vec::new(),
            upstream_tls_config: None,
        };

        assert!(should_route_via_package_proxy(
            &endpoint_settings,
            Some(&package_proxy),
            &decision,
        ));
    }

    #[test]
    fn package_proxy_does_not_route_non_package_binaries_without_endpoint_override() {
        let decision = ConnectDecision {
            action: NetworkAction::Allow {
                matched_policy: Some("allow".to_string()),
            },
            binary: Some(PathBuf::from("/usr/bin/curl")),
            binary_pid: Some(1234),
            ancestors: vec![PathBuf::from("/usr/bin/bash")],
            cmdline_paths: Vec::new(),
        };
        let endpoint_settings = EndpointSettings::default();
        let package_proxy = PackageProxyConfig {
            profile: "generic".to_string(),
            upstream_url: "http://proxy.socket.dev:8080".to_string(),
            upstream_host: "proxy.socket.dev".to_string(),
            upstream_port: 8080,
            scheme: PackageProxyScheme::Http,
            authorization: None,
            extra_ca_paths: Vec::new(),
            upstream_tls_config: None,
        };

        assert!(!should_route_via_package_proxy(
            &endpoint_settings,
            Some(&package_proxy),
            &decision,
        ));
    }

    #[test]
    fn rewrite_forward_request_for_upstream_proxy_injects_proxy_authorization() {
        let auth_file = tempfile::NamedTempFile::new().expect("temp auth file");
        std::fs::write(auth_file.path(), "Bearer proxy-token\n").expect("write auth file");
        with_vars(
            vec![
                (PACKAGE_PROXY_ENABLED_ENV, Some("1")),
                (
                    PACKAGE_PROXY_UPSTREAM_URL_ENV,
                    Some("http://proxy.socket.dev:8080"),
                ),
                (
                    PACKAGE_PROXY_AUTHORIZATION_FILE_ENV,
                    auth_file.path().to_str(),
                ),
                (PACKAGE_PROXY_CA_FILE_ENV, None),
                (PACKAGE_PROXY_PROFILE_ENV, None),
            ],
            || {
                let package_proxy = PackageProxyConfig::from_env()
                    .expect("config parse")
                    .expect("config should exist");
                let raw = b"GET http://registry.npmjs.org/pkg HTTP/1.1\r\nHost: registry.npmjs.org\r\nProxy-Authorization: Basic old\r\n\r\n";
                let rewritten =
                    rewrite_forward_request_for_upstream_proxy(raw, raw.len(), &package_proxy);
                let rewritten = String::from_utf8_lossy(&rewritten);
                assert!(rewritten.starts_with("GET http://registry.npmjs.org/pkg HTTP/1.1\r\n"));
                assert!(rewritten.contains("Proxy-Authorization: Bearer proxy-token"));
                assert!(!rewritten.contains("Basic old"));
            },
        );
    }

    // --- Forward proxy SSRF defence tests ---
    //
    // The forward proxy handler uses the same SSRF logic as the CONNECT path:
    //   - No allowed_ips: resolve_and_reject_internal blocks private IPs, allows public.
    //   - With allowed_ips: resolve_and_check_allowed_ips validates against allowlist.
    //
    // These tests document that contract for the forward proxy path specifically.

    #[tokio::test]
    async fn test_forward_public_ip_allowed_without_allowed_ips() {
        // Public IPs (e.g. dns.google -> 8.8.8.8) should pass through
        // resolve_and_reject_internal without needing allowed_ips.
        let result = resolve_and_reject_internal("dns.google", 80).await;
        assert!(
            result.is_ok(),
            "Public IP should be allowed without allowed_ips: {result:?}"
        );
        let addrs = result.unwrap();
        assert!(!addrs.is_empty(), "Should resolve to at least one address");
        // All resolved addresses should be public.
        for addr in &addrs {
            assert!(
                !is_internal_ip(addr.ip()),
                "dns.google should resolve to public IPs, got {}",
                addr.ip()
            );
        }
    }

    #[tokio::test]
    async fn test_forward_private_ip_rejected_without_allowed_ips() {
        // Private IP literals should be rejected by resolve_and_reject_internal.
        let result = resolve_and_reject_internal("10.0.0.1", 80).await;
        assert!(
            result.is_err(),
            "Private IP should be rejected without allowed_ips"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("internal address"),
            "expected 'internal address' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_forward_private_ip_accepted_with_allowed_ips() {
        // Private IP with matching allowed_ips should pass through.
        let nets = parse_allowed_ips(&["10.0.0.0/8".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("10.0.0.1", 80, &nets).await;
        assert!(
            result.is_ok(),
            "Private IP with matching allowed_ips should be accepted: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_forward_private_ip_rejected_with_wrong_allowed_ips() {
        // Private IP not in allowed_ips should be rejected.
        let nets = parse_allowed_ips(&["192.168.0.0/16".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("10.0.0.1", 80, &nets).await;
        assert!(
            result.is_err(),
            "Private IP not in allowed_ips should be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("not in allowed_ips"),
            "expected 'not in allowed_ips' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_forward_loopback_always_blocked_even_with_allowed_ips() {
        // Loopback addresses are always blocked, even if in allowed_ips.
        let nets = parse_allowed_ips(&["127.0.0.0/8".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("127.0.0.1", 80, &nets).await;
        assert!(result.is_err(), "Loopback should be always blocked");
        let err = result.unwrap_err();
        assert!(
            err.contains("always-blocked"),
            "expected 'always-blocked' in error: {err}"
        );
    }

    #[tokio::test]
    async fn test_forward_link_local_always_blocked_even_with_allowed_ips() {
        // Link-local / cloud metadata addresses are always blocked.
        let nets = parse_allowed_ips(&["169.254.0.0/16".to_string()]).unwrap();
        let result = resolve_and_check_allowed_ips("169.254.169.254", 80, &nets).await;
        assert!(result.is_err(), "Link-local should be always blocked");
        let err = result.unwrap_err();
        assert!(
            err.contains("always-blocked"),
            "expected 'always-blocked' in error: {err}"
        );
    }
}
