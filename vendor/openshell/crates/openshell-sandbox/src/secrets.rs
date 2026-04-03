// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use miette::{Result, miette};

const PLACEHOLDER_PREFIX: &str = "openshell:resolve:env:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecretInjectionRule {
    pub env_var: String,
    pub proxy_value: String,
    pub match_headers: Vec<String>,
    pub match_query: bool,
    pub match_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecretSwap {
    pub env_var: String,
    pub locations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedHttpRequest {
    pub bytes: Vec<u8>,
    pub swaps: Vec<SecretSwap>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SecretResolver {
    by_env_var: HashMap<String, SecretEntry>,
}

#[derive(Debug, Clone)]
struct SecretEntry {
    placeholder: String,
    real_value: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ScopedSecretInjector {
    rules: Vec<ResolvedSecretRule>,
}

#[derive(Debug, Clone)]
struct ResolvedSecretRule {
    env_var: String,
    proxy_value: String,
    real_value: String,
    match_headers: Option<HashSet<String>>,
    match_query: bool,
}

impl SecretResolver {
    pub(crate) fn from_provider_env(
        provider_env: HashMap<String, String>,
    ) -> (HashMap<String, String>, Option<Self>) {
        if provider_env.is_empty() {
            return (HashMap::new(), None);
        }

        let mut child_env = HashMap::with_capacity(provider_env.len());
        let mut by_env_var = HashMap::with_capacity(provider_env.len());

        for (key, value) in provider_env {
            let placeholder = placeholder_for_env_key(&key);
            child_env.insert(key.clone(), placeholder.clone());
            by_env_var.insert(
                key,
                SecretEntry {
                    placeholder,
                    real_value: value,
                },
            );
        }

        (child_env, Some(Self { by_env_var }))
    }

    pub(crate) fn scoped_injector(
        &self,
        rules: &[SecretInjectionRule],
    ) -> Result<Option<ScopedSecretInjector>> {
        if rules.is_empty() {
            return Ok(None);
        }

        let mut resolved = Vec::with_capacity(rules.len());
        for rule in rules {
            let entry = self.by_env_var.get(&rule.env_var).ok_or_else(|| {
                miette!(
                    "secret injection env_var '{}' is not available in provider env",
                    rule.env_var
                )
            })?;
            let proxy_value = if rule.proxy_value.is_empty() {
                entry.placeholder.clone()
            } else {
                rule.proxy_value.clone()
            };
            let match_headers = if rule.match_headers.is_empty() {
                None
            } else {
                Some(
                    rule.match_headers
                        .iter()
                        .map(|name| name.to_ascii_lowercase())
                        .collect(),
                )
            };
            resolved.push(ResolvedSecretRule {
                env_var: rule.env_var.clone(),
                proxy_value,
                real_value: entry.real_value.clone(),
                match_headers,
                match_query: rule.match_query,
            });
        }

        Ok(Some(ScopedSecretInjector { rules: resolved }))
    }
}

impl ScopedSecretInjector {
    pub(crate) fn rewrite_http_request(&self, raw: &[u8]) -> Result<PreparedHttpRequest> {
        let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4) else {
            if contains_placeholder_bytes(raw) {
                return Err(miette!(
                    "request contains placeholder secrets outside an authorized injection path"
                ));
            }
            return Ok(PreparedHttpRequest {
                bytes: raw.to_vec(),
                swaps: Vec::new(),
            });
        };

        let header_str = String::from_utf8_lossy(&raw[..header_end]);
        let mut lines = header_str.split("\r\n");
        let Some(request_line) = lines.next() else {
            if contains_placeholder_bytes(raw) {
                return Err(miette!(
                    "request contains placeholder secrets outside an authorized injection path"
                ));
            }
            return Ok(PreparedHttpRequest {
                bytes: raw.to_vec(),
                swaps: Vec::new(),
            });
        };

        let mut recorded = BTreeMap::<String, BTreeSet<String>>::new();
        let rewritten_request_line = self.rewrite_request_line(request_line, &mut recorded)?;

        let mut output = Vec::with_capacity(raw.len());
        output.extend_from_slice(rewritten_request_line.as_bytes());
        output.extend_from_slice(b"\r\n");

        for line in lines {
            if line.is_empty() {
                break;
            }

            let rewritten = self.rewrite_header_line(line, &mut recorded);
            output.extend_from_slice(rewritten.as_bytes());
            output.extend_from_slice(b"\r\n");
        }

        output.extend_from_slice(b"\r\n");
        output.extend_from_slice(&raw[header_end..]);

        if contains_placeholder_bytes(&output) {
            return Err(miette!(
                "request contains placeholder secrets outside an authorized injection path"
            ));
        }

        Ok(PreparedHttpRequest {
            bytes: output,
            swaps: recorded
                .into_iter()
                .map(|(env_var, locations)| SecretSwap {
                    env_var,
                    locations: locations.into_iter().collect(),
                })
                .collect(),
        })
    }

    fn rewrite_request_line(
        &self,
        request_line: &str,
        recorded: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> Result<String> {
        let mut parts = request_line.splitn(3, ' ');
        let Some(method) = parts.next() else {
            return Ok(request_line.to_string());
        };
        let Some(target) = parts.next() else {
            return Ok(request_line.to_string());
        };
        let Some(version) = parts.next() else {
            return Ok(request_line.to_string());
        };

        let rewritten_target = self.rewrite_target(target, recorded)?;
        Ok(format!("{method} {rewritten_target} {version}"))
    }

    fn rewrite_target(
        &self,
        target: &str,
        recorded: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> Result<String> {
        let Some((path, raw_query)) = target.split_once('?') else {
            return Ok(target.to_string());
        };

        let mut params: Vec<(String, String)> = url::form_urlencoded::parse(raw_query.as_bytes())
            .into_owned()
            .collect();
        let mut changed = false;

        for (key, value) in &mut params {
            let mut updated = value.clone();
            for rule in &self.rules {
                if !rule.match_query {
                    continue;
                }
                if updated.contains(&rule.proxy_value) {
                    updated = updated.replace(&rule.proxy_value, &rule.real_value);
                    record_swap(recorded, &rule.env_var, format!("query:{key}"));
                    changed = true;
                }
            }
            *value = updated;
        }

        if !changed {
            return Ok(target.to_string());
        }

        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in params {
            serializer.append_pair(&key, &value);
        }
        let encoded = serializer.finish();
        Ok(format!("{path}?{encoded}"))
    }

    fn rewrite_header_line(
        &self,
        line: &str,
        recorded: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> String {
        let Some((name, value)) = line.split_once(':') else {
            return line.to_string();
        };
        let name_lc = name.trim().to_ascii_lowercase();
        let mut updated = value.trim().to_string();
        let mut changed = false;

        for rule in &self.rules {
            let applies = rule
                .match_headers
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&name_lc));
            if !applies {
                continue;
            }

            let (rewritten, did_change) =
                replace_in_header(name.trim(), &updated, &rule.proxy_value, &rule.real_value);
            if did_change {
                updated = rewritten;
                changed = true;
                record_swap(recorded, &rule.env_var, format!("header:{name_lc}"));
            }
        }

        if changed {
            format!("{name}: {updated}")
        } else {
            line.to_string()
        }
    }
}

pub(crate) fn placeholder_for_env_key(key: &str) -> String {
    format!("{PLACEHOLDER_PREFIX}{key}")
}

pub(crate) fn contains_placeholder_bytes(bytes: &[u8]) -> bool {
    bytes
        .windows(PLACEHOLDER_PREFIX.len())
        .any(|window| window == PLACEHOLDER_PREFIX.as_bytes())
}

fn record_swap(recorded: &mut BTreeMap<String, BTreeSet<String>>, env_var: &str, location: String) {
    recorded
        .entry(env_var.to_string())
        .or_default()
        .insert(location);
}

fn replace_in_header(
    header_name: &str,
    value: &str,
    proxy_value: &str,
    real_value: &str,
) -> (String, bool) {
    if header_name.eq_ignore_ascii_case("Authorization")
        && let Some(decoded) = decode_basic_auth(value)
        && decoded.contains(proxy_value)
    {
        let replaced = decoded.replace(proxy_value, real_value);
        return (
            format!(
                "Basic {}",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, replaced)
            ),
            true,
        );
    }

    if value.contains(proxy_value) {
        (value.replace(proxy_value, real_value), true)
    } else {
        (value.to_string(), false)
    }
}

fn decode_basic_auth(value: &str) -> Option<String> {
    let payload = value.strip_prefix("Basic ")?;
    let decoded =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload).ok()?;
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver_with(values: &[(&str, &str)]) -> SecretResolver {
        let provider_env = values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        let (_, resolver) = SecretResolver::from_provider_env(provider_env);
        resolver.expect("resolver")
    }

    #[test]
    fn provider_env_is_replaced_with_placeholders() {
        let (child_env, resolver) = SecretResolver::from_provider_env(
            [("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())]
                .into_iter()
                .collect(),
        );

        assert_eq!(
            child_env.get("ANTHROPIC_API_KEY"),
            Some(&"openshell:resolve:env:ANTHROPIC_API_KEY".to_string())
        );
        assert!(resolver.is_some());
    }

    #[test]
    fn scoped_injector_requires_declared_env_var() {
        let resolver = resolver_with(&[("ANTHROPIC_API_KEY", "sk-test")]);
        let error = resolver
            .scoped_injector(&[SecretInjectionRule {
                env_var: "MISSING".to_string(),
                proxy_value: String::new(),
                match_headers: Vec::new(),
                match_query: false,
                match_body: false,
            }])
            .expect_err("missing env var should fail");
        assert!(error.to_string().contains("MISSING"));
    }

    #[test]
    fn scoped_injector_rewrites_authorized_headers() {
        let resolver = resolver_with(&[
            ("ANTHROPIC_API_KEY", "sk-real"),
            ("CUSTOM_TOKEN", "tok-real"),
        ]);
        let injector = resolver
            .scoped_injector(&[
                SecretInjectionRule {
                    env_var: "ANTHROPIC_API_KEY".to_string(),
                    proxy_value: String::new(),
                    match_headers: vec!["Authorization".to_string()],
                    match_query: false,
                    match_body: false,
                },
                SecretInjectionRule {
                    env_var: "CUSTOM_TOKEN".to_string(),
                    proxy_value: String::new(),
                    match_headers: vec!["x-api-key".to_string()],
                    match_query: false,
                    match_body: false,
                },
            ])
            .expect("injector build")
            .expect("injector");

        let raw = b"GET /v1/messages HTTP/1.1\r\nAuthorization: Bearer openshell:resolve:env:ANTHROPIC_API_KEY\r\nx-api-key: openshell:resolve:env:CUSTOM_TOKEN\r\nHost: example.com\r\n\r\n";
        let prepared = injector.rewrite_http_request(raw).expect("rewrite");
        let rewritten = String::from_utf8(prepared.bytes).expect("utf8");

        assert!(rewritten.contains("Authorization: Bearer sk-real\r\n"));
        assert!(rewritten.contains("x-api-key: tok-real\r\n"));
        assert!(!rewritten.contains("openshell:resolve:env:"));
        assert_eq!(prepared.swaps.len(), 2);
    }

    #[test]
    fn scoped_injector_rewrites_query_parameters() {
        let resolver = resolver_with(&[("SERVICE_TOKEN", "tok-real")]);
        let injector = resolver
            .scoped_injector(&[SecretInjectionRule {
                env_var: "SERVICE_TOKEN".to_string(),
                proxy_value: String::new(),
                match_headers: Vec::new(),
                match_query: true,
                match_body: false,
            }])
            .expect("injector build")
            .expect("injector");

        let raw = b"GET /download?token=openshell%3Aresolve%3Aenv%3ASERVICE_TOKEN HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let prepared = injector.rewrite_http_request(raw).expect("rewrite");
        let rewritten = String::from_utf8(prepared.bytes).expect("utf8");

        assert!(rewritten.starts_with("GET /download?token=tok-real HTTP/1.1\r\n"));
        assert_eq!(
            prepared.swaps,
            vec![SecretSwap {
                env_var: "SERVICE_TOKEN".to_string(),
                locations: vec!["query:token".to_string()],
            }]
        );
    }

    #[test]
    fn scoped_injector_denies_unauthorized_placeholders() {
        let resolver = resolver_with(&[("ANTHROPIC_API_KEY", "sk-real")]);
        let injector = resolver
            .scoped_injector(&[SecretInjectionRule {
                env_var: "ANTHROPIC_API_KEY".to_string(),
                proxy_value: String::new(),
                match_headers: vec!["Authorization".to_string()],
                match_query: false,
                match_body: false,
            }])
            .expect("injector build")
            .expect("injector");

        let raw = b"GET /v1/messages HTTP/1.1\r\nx-api-key: openshell:resolve:env:ANTHROPIC_API_KEY\r\nHost: example.com\r\n\r\n";
        let error = injector
            .rewrite_http_request(raw)
            .expect_err("unauthorized placeholder should fail");
        assert!(error.to_string().contains("placeholder secrets"));
    }

    #[test]
    fn scoped_injector_rewrites_basic_auth_headers() {
        let resolver = resolver_with(&[("SERVICE_TOKEN", "real-token")]);
        let injector = resolver
            .scoped_injector(&[SecretInjectionRule {
                env_var: "SERVICE_TOKEN".to_string(),
                proxy_value: String::new(),
                match_headers: vec!["Authorization".to_string()],
                match_query: false,
                match_body: false,
            }])
            .expect("injector build")
            .expect("injector");

        let basic = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "user:openshell:resolve:env:SERVICE_TOKEN",
        );
        let raw = format!(
            "GET /v1/messages HTTP/1.1\r\nAuthorization: Basic {basic}\r\nHost: example.com\r\n\r\n"
        );
        let prepared = injector
            .rewrite_http_request(raw.as_bytes())
            .expect("rewrite");
        let rewritten = String::from_utf8(prepared.bytes).expect("utf8");
        let auth = rewritten
            .lines()
            .find(|line| line.starts_with("Authorization: "))
            .expect("auth line");
        let decoded = decode_basic_auth(auth.trim_start_matches("Authorization: "))
            .expect("decode basic auth");
        assert_eq!(decoded, "user:real-token");
    }

    #[test]
    fn contains_placeholder_bytes_detects_leaks() {
        assert!(contains_placeholder_bytes(
            b"Authorization: Bearer openshell:resolve:env:ANTHROPIC_API_KEY"
        ));
        assert!(!contains_placeholder_bytes(
            b"Authorization: Bearer sk-test"
        ));
    }

    #[test]
    fn scoped_injector_without_rules_denies_placeholders_by_final_scan() {
        let injector = ScopedSecretInjector { rules: Vec::new() };
        let error = injector
            .rewrite_http_request(
                b"GET / HTTP/1.1\r\nAuthorization: Bearer openshell:resolve:env:ANTHROPIC_API_KEY\r\n\r\n",
            )
            .expect_err("placeholder should fail");
        assert!(error.to_string().contains("placeholder secrets"));
    }
}
