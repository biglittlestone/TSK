//! One-way ledger. Append-only, single writer, no read-back loop. See spec §6.3,
//! iron law 7. A write failure never blocks the hook.

use serde::{Deserialize, Serialize};
use std::path::Path;

// serde's `default` attribute takes a function path, so each kind string
// needs a tiny getter. (The variant structs carry `kind`, but `Entry`'s
// internal tag already owns the JSON field — skipped in both directions to
// avoid a duplicate key in the serialized line.)
fn rewrite() -> &'static str {
    "rewrite"
}
fn retrieval() -> &'static str {
    "retrieval"
}
fn externalize() -> &'static str {
    "externalize"
}
fn inject() -> &'static str {
    "inject"
}
fn sandbox() -> &'static str {
    "sandbox"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteEntry {
    #[serde(skip_serializing, skip_deserializing, default = "rewrite")]
    pub kind: &'static str,
    pub ts: u64,
    pub session: String,
    pub tool: String,
    pub tu_id: String,
    pub strategy: String,
    pub orig_bytes: usize,
    pub new_bytes: usize,
    pub saved: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalEntry {
    #[serde(skip_serializing, skip_deserializing, default = "retrieval")]
    pub kind: &'static str,
    pub ts: u64,
    pub session: String,
    pub tool: String,
    /// Why the escape hatch was taken (e.g. `escape_hatch_offset_limit`).
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalizeEntry {
    #[serde(skip_serializing, skip_deserializing, default = "externalize")]
    pub kind: &'static str,
    pub ts: u64,
    pub session: String,
    pub tool: String,
    pub orig_bytes: usize,
    pub new_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectEntry {
    #[serde(skip_serializing, skip_deserializing, default = "inject")]
    pub kind: &'static str,
    pub ts: u64,
    pub session: String,
    pub source: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxEntry {
    #[serde(skip_serializing, skip_deserializing, default = "sandbox")]
    pub kind: &'static str,
    pub ts: u64,
    pub session: String,
    pub tool: String,
    pub orig_bytes: usize,
    pub new_bytes: usize,
    pub saved: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Rewrite(RewriteEntry),
    Retrieval(RetrievalEntry),
    Externalize(ExternalizeEntry),
    Inject(InjectEntry),
    Sandbox(SandboxEntry),
}

/// Append one line to `~/.tsk/<session>/ledger.jsonl`. Ignores errors by design.
pub fn append(session_id: &str, entry: &Entry) {
    let dir = crate::storage::session_dir(session_id);
    crate::storage::ensure_dir(&dir);
    let line = match serde_json::to_string(entry) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("ledger.jsonl"))
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes()));
}

/// Read back every parseable entry. Used by `tsk report`; unreadable lines are skipped.
pub fn read_all(path: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
