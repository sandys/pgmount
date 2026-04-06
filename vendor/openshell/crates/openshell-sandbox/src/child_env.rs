// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

const LOCAL_NO_PROXY: &str = "127.0.0.1,localhost,::1";
const PACKAGE_PROXY_CA_FILE_ENV: &str = "OPENERAL_PACKAGE_PROXY_CA_FILE";

pub(crate) fn proxy_env_vars(proxy_url: &str) -> [(&'static str, String); 9] {
    [
        ("ALL_PROXY", proxy_url.to_owned()),
        ("HTTP_PROXY", proxy_url.to_owned()),
        ("HTTPS_PROXY", proxy_url.to_owned()),
        ("NO_PROXY", LOCAL_NO_PROXY.to_owned()),
        ("http_proxy", proxy_url.to_owned()),
        ("https_proxy", proxy_url.to_owned()),
        ("no_proxy", LOCAL_NO_PROXY.to_owned()),
        ("grpc_proxy", proxy_url.to_owned()),
        // Node.js only honors HTTP(S)_PROXY for built-in fetch/http clients when
        // proxy support is explicitly enabled at process startup.
        ("NODE_USE_ENV_PROXY", "1".to_owned()),
    ]
}

fn merged_bundle_path(
    ca_cert_path: &Path,
    combined_bundle_path: &Path,
) -> std::io::Result<(PathBuf, PathBuf)> {
    let Some(package_proxy_ca_path) =
        std::env::var_os(PACKAGE_PROXY_CA_FILE_ENV).map(PathBuf::from)
    else {
        return Ok((
            ca_cert_path.to_path_buf(),
            combined_bundle_path.to_path_buf(),
        ));
    };

    let package_proxy_ca = std::fs::read_to_string(&package_proxy_ca_path)?;
    if package_proxy_ca.trim().is_empty() {
        return Ok((
            ca_cert_path.to_path_buf(),
            combined_bundle_path.to_path_buf(),
        ));
    }

    let mut node_extra = std::fs::read_to_string(ca_cert_path)?;
    if !node_extra.contains(&package_proxy_ca) {
        if !node_extra.ends_with('\n') {
            node_extra.push('\n');
        }
        node_extra.push_str(&package_proxy_ca);
    }

    let mut combined = std::fs::read_to_string(combined_bundle_path)?;
    if !combined.contains(&package_proxy_ca) {
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&package_proxy_ca);
    }

    let node_extra_path = ca_cert_path
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("node-extra-ca-bundle.pem");
    let combined_path = combined_bundle_path
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("ca-bundle-with-package-proxy.pem");

    std::fs::write(&node_extra_path, node_extra)?;
    std::fs::write(&combined_path, combined)?;

    Ok((node_extra_path, combined_path))
}

pub(crate) fn tls_env_vars(
    ca_cert_path: &Path,
    combined_bundle_path: &Path,
) -> Vec<(&'static str, String)> {
    let (node_extra_ca_path, combined_bundle_path) =
        merged_bundle_path(ca_cert_path, combined_bundle_path).unwrap_or_else(|_| {
            (
                ca_cert_path.to_path_buf(),
                combined_bundle_path.to_path_buf(),
            )
        });
    let node_extra_ca_path = node_extra_ca_path.display().to_string();
    let combined_bundle_path = combined_bundle_path.display().to_string();

    vec![
        ("NODE_EXTRA_CA_CERTS", node_extra_ca_path),
        ("SSL_CERT_FILE", combined_bundle_path.clone()),
        ("REQUESTS_CA_BUNDLE", combined_bundle_path.clone()),
        ("CURL_CA_BUNDLE", combined_bundle_path.clone()),
        ("PIP_CERT", combined_bundle_path.clone()),
        ("YARN_HTTPS_CA_FILE_PATH", combined_bundle_path.clone()),
        ("CARGO_HTTP_CAINFO", combined_bundle_path.clone()),
        ("CARGO_HTTP_PROXY_CAINFO", combined_bundle_path),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::process::Stdio;
    use temp_env::with_var;

    #[test]
    fn apply_proxy_env_includes_node_proxy_opt_in_and_local_bypass() {
        let mut cmd = Command::new("/usr/bin/env");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, value) in proxy_env_vars("http://10.200.0.1:3128") {
            cmd.env(key, value);
        }

        let output = cmd.output().expect("spawn env");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert!(stdout.contains("HTTP_PROXY=http://10.200.0.1:3128"));
        assert!(stdout.contains("NO_PROXY=127.0.0.1,localhost,::1"));
        assert!(stdout.contains("NODE_USE_ENV_PROXY=1"));
        assert!(stdout.contains("no_proxy=127.0.0.1,localhost,::1"));
    }

    #[test]
    fn apply_tls_env_sets_node_and_bundle_paths() {
        let mut cmd = Command::new("/usr/bin/env");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let ca_cert_path = Path::new("/etc/openshell-tls/openshell-ca.pem");
        let combined_bundle_path = Path::new("/etc/openshell-tls/ca-bundle.pem");
        for (key, value) in tls_env_vars(ca_cert_path, combined_bundle_path) {
            cmd.env(key, value);
        }

        let output = cmd.output().expect("spawn env");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert!(stdout.contains("NODE_EXTRA_CA_CERTS=/etc/openshell-tls/openshell-ca.pem"));
        assert!(stdout.contains("SSL_CERT_FILE=/etc/openshell-tls/ca-bundle.pem"));
        assert!(stdout.contains("PIP_CERT=/etc/openshell-tls/ca-bundle.pem"));
        assert!(stdout.contains("CARGO_HTTP_PROXY_CAINFO=/etc/openshell-tls/ca-bundle.pem"));
    }

    #[test]
    fn apply_tls_env_merges_package_proxy_ca_when_configured() {
        let temp = tempfile::tempdir().expect("tempdir");
        let openshell_ca = temp.path().join("openshell-ca.pem");
        let combined_bundle = temp.path().join("ca-bundle.pem");
        let package_proxy_ca = temp.path().join("package-proxy-ca.pem");
        std::fs::write(
            &openshell_ca,
            "-----BEGIN CERTIFICATE-----\nopenshell\n-----END CERTIFICATE-----\n",
        )
        .expect("write openshell ca");
        std::fs::write(&combined_bundle, "-----BEGIN CERTIFICATE-----\nsystem\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nopenshell\n-----END CERTIFICATE-----\n")
            .expect("write combined bundle");
        std::fs::write(
            &package_proxy_ca,
            "-----BEGIN CERTIFICATE-----\npackage-proxy\n-----END CERTIFICATE-----\n",
        )
        .expect("write package proxy ca");

        let stdout = with_var(
            PACKAGE_PROXY_CA_FILE_ENV,
            Some(package_proxy_ca.as_os_str()),
            || {
                let mut cmd = Command::new("/usr/bin/env");
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());

                for (key, value) in tls_env_vars(&openshell_ca, &combined_bundle) {
                    cmd.env(key, value);
                }

                let output = cmd.output().expect("spawn env");
                String::from_utf8(output.stdout).expect("utf8")
            },
        );
        let node_bundle_path = stdout
            .lines()
            .find_map(|line| line.strip_prefix("NODE_EXTRA_CA_CERTS="))
            .expect("node extra ca env");
        let combined_bundle_path = stdout
            .lines()
            .find_map(|line| line.strip_prefix("SSL_CERT_FILE="))
            .expect("combined bundle env");

        let node_bundle = std::fs::read_to_string(node_bundle_path).expect("read node bundle");
        let merged_bundle =
            std::fs::read_to_string(combined_bundle_path).expect("read merged bundle");
        assert!(node_bundle.contains("openshell"));
        assert!(node_bundle.contains("package-proxy"));
        assert!(merged_bundle.contains("system"));
        assert!(merged_bundle.contains("package-proxy"));
    }
}
