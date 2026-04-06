use crate::error::FsError;
use clap::Parser;
use std::process::Command;

const DB_PROVIDER: &str = "openeral-db";
const PRESIGN_URL: &str = "https://app.stringcost.com/v1/presign";

const DEFAULT_CLUSTER_IMAGE: &str = "ghcr.io/sandys/openeral/cluster:latest";
const DEFAULT_REGISTRY_HOST: &str = "ghcr.io";
const DEFAULT_IMAGE_REPO_BASE: &str = "ghcr.io/sandys/openeral";

#[derive(Parser)]
pub struct LaunchArgs {
    /// Gateway name
    #[arg(long, default_value = "openeral")]
    pub gateway: String,

    /// Sandbox name
    #[arg(long, default_value = "openeral-sandbox")]
    pub name: String,

    /// Sandbox image reference
    #[arg(long, env = "OPENERAL_SANDBOX_IMAGE")]
    pub image: String,

    /// Disable StringCost even if STRINGCOST_API_KEY is set
    #[arg(long)]
    pub no_stringcost: bool,

    /// Print commands without executing
    #[arg(long)]
    pub dry_run: bool,
}

fn required_env(key: &str) -> Result<String, FsError> {
    std::env::var(key).map_err(|_| {
        FsError::InvalidArgument(format!(
            "environment variable {key} is required but not set"
        ))
    })
}

/// Run an openshell CLI command. Returns Ok(true) on success, Ok(false) if
/// the command exited non-zero, or Err on spawn failure.
fn openshell(args: &[&str], dry_run: bool) -> Result<bool, FsError> {
    let cmd_str = format!("openshell {}", args.join(" "));
    if dry_run {
        println!("  [dry-run] {cmd_str}");
        return Ok(true);
    }
    tracing::debug!(cmd = %cmd_str, "running openshell");
    let status = Command::new("openshell").args(args).status()?;
    Ok(status.success())
}

/// Run an openshell command and capture stdout.
fn openshell_output(args: &[&str]) -> Result<String, FsError> {
    let output = Command::new("openshell")
        .args(args)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Call StringCost presign API via curl and return the base URL.
fn presign_stringcost(
    stringcost_key: &str,
    anthropic_key: &str,
    dry_run: bool,
) -> Result<String, FsError> {
    if dry_run {
        println!("  [dry-run] curl -s -X POST {PRESIGN_URL} (presign Anthropic key)");
        return Ok("https://proxy.stringcost.com/stringcost-proxy/t/DRY_RUN_TOKEN".into());
    }

    let body = serde_json::json!({
        "provider": "anthropic",
        "client_api_key": anthropic_key,
        "path": ["/v1/messages"],
        "expires_in": -1,
        "max_uses": -1,
        "tags": ["openeral"],
        "metadata": {"source": "openeral"}
    });

    let output = Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", &format!("Authorization: Bearer {stringcost_key}"),
            "-H", "Content-Type: application/json",
            "-d", &body.to_string(),
            PRESIGN_URL,
        ])
        .output()?;

    if !output.status.success() {
        return Err(FsError::InternalError(
            "StringCost presign request failed".into(),
        ));
    }

    let resp: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        FsError::InternalError(format!(
            "failed to parse StringCost response: {e}\nraw: {}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })?;

    let url = resp["url"]
        .as_str()
        .ok_or_else(|| {
            FsError::InternalError(format!(
                "StringCost response missing 'url' field: {resp}"
            ))
        })?;

    // Strip the path suffix to get the base URL
    // e.g. https://proxy.stringcost.com/stringcost-proxy/t/TOKEN/v1/messages
    //   -> https://proxy.stringcost.com/stringcost-proxy/t/TOKEN
    let base = url
        .find("/v1/")
        .map(|i| &url[..i])
        .unwrap_or(url);

    Ok(base.to_string())
}

/// Check whether a provider already exists on the gateway.
fn provider_exists(gateway: &str, name: &str) -> bool {
    openshell_output(&["provider", "get", "--gateway", gateway, "--name", name])
        .map(|out| !out.is_empty() && !out.contains("not found"))
        .unwrap_or(false)
}

fn step(n: u8, total: u8, msg: &str) {
    println!("[{n}/{total}] {msg}");
}

fn set_default_env(key: &str, value: &str) {
    if std::env::var(key).map(|v| v.is_empty()).unwrap_or(true) {
        std::env::set_var(key, value);
    }
}

pub async fn execute(args: LaunchArgs) -> Result<(), FsError> {
    // Set OpenShell image defaults so users don't have to.
    set_default_env("OPENSHELL_CLUSTER_IMAGE", DEFAULT_CLUSTER_IMAGE);
    set_default_env("OPENSHELL_REGISTRY_HOST", DEFAULT_REGISTRY_HOST);
    set_default_env("IMAGE_REPO_BASE", DEFAULT_IMAGE_REPO_BASE);

    let anthropic_key = required_env("ANTHROPIC_API_KEY")?;
    let _database_url = required_env("DATABASE_URL")?;

    let use_stringcost = !args.no_stringcost
        && std::env::var("STRINGCOST_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
    let stringcost_key = if use_stringcost {
        Some(required_env("STRINGCOST_API_KEY")?)
    } else {
        None
    };

    // StringCost uses direct routing (no inference routing needed).
    // The presigned URL becomes ANTHROPIC_BASE_URL and the OpenShell proxy
    // rewrites the x-api-key placeholder header to the real key automatically.
    // Claude Code picks its own model — no model override.
    let stringcost_base_url = if let Some(sc_key) = &stringcost_key {
        let total: u8 = 4;
        step(1, total, "Starting gateway...");
        start_gateway(&args)?;

        step(2, total, "Creating database provider...");
        ensure_db_provider(&args)?;

        step(3, total, "Presigning Anthropic key via StringCost...");
        let base_url = presign_stringcost(sc_key, &anthropic_key, args.dry_run)?;
        println!("    base_url = {base_url}");

        step(4, total, "Launching sandbox...");
        Some(base_url)
    } else {
        let total: u8 = 3;
        step(1, total, "Starting gateway...");
        start_gateway(&args)?;

        step(2, total, "Creating database provider...");
        ensure_db_provider(&args)?;

        step(3, total, "Launching sandbox...");
        None
    };

    // Build sandbox command
    let mut sandbox_args: Vec<&str> = vec![
        "sandbox", "create",
        "--gateway", &args.gateway,
        "--name", &args.name,
        "--from", &args.image,
        "--provider", DB_PROVIDER,
        "--provider", "claude",
        "--auto-providers",
        "--",
    ];

    let base_url_env;
    if let Some(ref url) = stringcost_base_url {
        base_url_env = format!("ANTHROPIC_BASE_URL={url}");
        sandbox_args.extend_from_slice(&[
            "env", &base_url_env,
            "HOME=/home/agent", "claude",
        ]);
    } else {
        sandbox_args.extend_from_slice(&["env", "HOME=/home/agent", "claude"]);
    }

    openshell(&sandbox_args, args.dry_run)?;

    if args.dry_run {
        println!("\n[dry-run] All commands printed. Re-run without --dry-run to execute.");
    }

    Ok(())
}

fn start_gateway(args: &LaunchArgs) -> Result<(), FsError> {
    if !openshell(
        &["gateway", "start", "--name", &args.gateway],
        args.dry_run,
    )? {
        println!("    (gateway already running, reusing)");
    }
    Ok(())
}

fn ensure_db_provider(args: &LaunchArgs) -> Result<(), FsError> {
    if !args.dry_run && provider_exists(&args.gateway, DB_PROVIDER) {
        println!("    (provider {DB_PROVIDER} already exists, skipping)");
    } else {
        openshell(
            &[
                "provider", "create",
                "--gateway", &args.gateway,
                "--name", DB_PROVIDER,
                "--type", "generic",
                "--credential", "DATABASE_URL",
            ],
            args.dry_run,
        )?;
    }
    Ok(())
}
