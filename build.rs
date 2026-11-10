//! Embeds build information read by `asaas version`.
//!
//! The CLI version comes from $ASAAS_VERSION at build time (the Makefile
//! passes `git describe`), defaulting to "dev".

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ASAAS_VERSION");

    let version = std::env::var("ASAAS_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=ASAAS_CLI_VERSION={version}");

    // "rustc 1.99.1 (58cf8f9c3 2026-10-13)" -> "1.99.1"
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rust_version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ASAAS_CLI_RUST_VERSION={rust_version}");
}
