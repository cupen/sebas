//! `xtask` — build-time tooling for sebas.
//!
//! Subcommands:
//! - `update-models` — fetch `https://models.dev/api.json`, parse it
//!   into the `gateway::models::ModelDef` shape, and regenerate the
//!   `MODELS` const body in `gateway/src/models.rs`.
//! - `check-docs` — fail if the tree still cites the removed
//!   superpowers planning corpus (ghost references).
//!
//! This is a one-shot CLI. No async runtime is owned by callers —
//! `update-models` uses reqwest's blocking client under the hood.

mod check_docs;
mod parser;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

const MODELS_DEV_URL: &str = "https://models.dev/api.json";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let subcommand = args.get(1).map(String::as_str).unwrap_or("");
    match subcommand {
        "" | "help" | "--help" | "-h" => {
            print_help();
            if subcommand.is_empty() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        "update-models" => run_update_models(&args[2..]),
        "check-docs" => check_docs::run(&args[2..]),
        other => {
            eprintln!("xtask: unknown subcommand `{}`", other);
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("xtask — build-time tooling for sebas");
    println!();
    println!("USAGE:");
    println!("    xtask <SUBCOMMAND>");
    println!();
    println!("SUBCOMMANDS:");
    println!("    update-models    Fetch https://models.dev/api.json and regenerate");
    println!("                    gateway/src/models.rs::MODELS in place.");
    println!("    check-docs       Fail if the tree cites the removed superpowers");
    println!("                    corpus (paths, dated `spec YYYY-MM-DD`, bare `spec §N`).");
    println!("    help             Print this message.");
}

fn run_update_models(extra_args: &[String]) -> ExitCode {
    // `--help` for subcommand.
    if extra_args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "xtask update-models — fetch {} and regenerate \
             gateway/src/models.rs::MODELS",
            MODELS_DEV_URL
        );
        return ExitCode::SUCCESS;
    }
    if !extra_args.is_empty() {
        eprintln!(
            "xtask update-models: unexpected arguments: {:?}",
            extra_args
        );
        return ExitCode::from(2);
    }

    let models_rs_path = locate_models_rs();

    eprintln!("xtask update-models: fetching {}", MODELS_DEV_URL);
    let body = match fetch_blocking(MODELS_DEV_URL) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("xtask update-models: network fetch failed: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let entries = match parser::parse_api_json(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("xtask update-models: JSON parse failed: {}", e);
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "xtask update-models: parsed {} model entries",
        entries.len()
    );

    let rendered = render::render_models_body(&entries);
    let report = match render::patch_models_rs(&models_rs_path, &rendered) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "xtask update-models: failed to patch {}: {}",
                models_rs_path.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    let utc_now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    println!(
        "Updated {} entries, wrote gateway/src/models.rs (timestamp: {}); file marker: {}",
        report.entries_written, utc_now, report.timestamp_line
    );
    ExitCode::SUCCESS
}

/// Locate `gateway/src/models.rs` relative to the xtask crate's
/// `CARGO_MANIFEST_DIR`. xtask lives at `<repo>/xtask/`, the target
/// lives at `<repo>/gateway/src/models.rs`.
fn locate_models_rs() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest_dir)
        .parent()
        .map(|p| p.join("gateway/src/models.rs"))
        .unwrap_or_else(|| PathBuf::from("gateway/src/models.rs"))
}

fn fetch_blocking(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("sebas-xtask/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("client build: {}", e))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("GET {}: {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} from {}", resp.status(), url));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("read body: {}", e))
}
