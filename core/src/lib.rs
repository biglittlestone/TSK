//! TSK core: policy, compression, sandbox, injection, ledger, snapshot.
//!
//! Pure library. No CLI concerns here (no clap, no stdout/stderr).
//! Errors propagate as `Result`; the CLI `main` is the single catch point.

pub mod config;
pub mod compress;
pub mod events;
pub mod inject;
pub mod ledger;
pub mod policy;
pub mod sandbox;
pub mod snapshot;
pub mod storage;

pub use config::{Config, CompressionLevel};

/// Milliseconds since the Unix epoch. Used by the ledger (field `ts`).
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Token estimate: `ceil(bytes / 4)`. Same convention as spike CS-4.
pub fn est_tokens(s: &str) -> usize {
    s.len().div_ceil(4)
}
