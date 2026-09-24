//! The decision table. Everything policy-related is a pure function of its inputs —
//! no filesystem, no I/O. The CLI wires the I/O around it. See spec §7.1.
//!
//! v1 is deliberately small: there are only two compression strategies.
//!
//! * `task_type` is not implemented — nothing in v1 routes on it (auto-clarity is
//!   a static declaration in the injected ruleset, not a detected task type).
//! * `Plan::Sandbox` is not implemented — the "aggregate vs. read" classifier is
//!   still undefined (spec §7.1 P-1), so the conservative default is to never
//!   guess: files that would qualify get a skeleton. `tsk exec` remains the
//!   escape hatch for an agent that *knows* it wants a sandboxed analysis.

use crate::config::{EXTERNALIZE_THRESHOLD, ROUTE_THRESHOLD};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Keep head + tail, replace the middle with a retrieval marker.
    Skeleton,
    /// Full text to disk, redirect the `Read` at a pointer + excerpt.
    Externalize,
    /// Run the project's `analyze_<ext>` sandbox script; its stdout enters context.
    /// Only a candidate here — the caller must confirm the script exists (§7.1).
    Sandbox,
}

/// The escape hatch is always on and never configurable (spec §5.3, S2b):
/// an explicit `offset` or `limit` is a retrieval, never something to compress.
pub fn is_escape(tool_input: &Value) -> bool {
    tool_input.get("offset").is_some() || tool_input.get("limit").is_some()
}

/// Aggregate-shaped file extensions (P-1's conservative heuristic, spike §14):
/// these are candidates for sandbox analysis. The sandbox still requires a
/// pre-placed `analyze_<ext>` script; without one the caller falls back to
/// Skeleton (never guess, never fabricate a script).
pub fn is_aggregate_ext(ext: &str) -> bool {
    matches!(ext, "log" | "jsonl" | "csv" | "json" | "tsv")
}

/// Route a `Read` by file size and shape. `None` size means we cannot decide ⇒ skeleton.
pub fn route(orig_bytes: Option<usize>, ext: &str) -> Plan {
    match orig_bytes {
        Some(n) if n > EXTERNALIZE_THRESHOLD => Plan::Externalize,
        Some(n) if n > ROUTE_THRESHOLD && is_aggregate_ext(ext) => Plan::Sandbox,
        _ => Plan::Skeleton,
    }
}

#[cfg(test)]
mod t {
    use super::*;
    use serde_json::json;

    #[test]
    fn escape_hatch_always_wins() {
        for v in [json!({"offset": 10}), json!({"limit": 5}), json!({"offset":0,"limit":3})] {
            assert!(is_escape(&v));
        }
        assert!(!is_escape(&json!({"file_path":"a"})));
        assert!(!is_escape(&json!(null)));
    }

    #[test]
    fn routing_thresholds() {
        let kb = 1024usize;
        // 纯文本（代码/配置）→ 骨架，即使 >50KB。
        assert_eq!(route(Some(51 * kb), "rs"), Plan::Skeleton);
        assert_eq!(route(Some(100 * kb), "rs"), Plan::Skeleton);
        // 聚合型扩展 + >50KB → Sandbox 候选。
        assert_eq!(route(Some(51 * kb), "log"), Plan::Sandbox);
        assert_eq!(route(Some(60 * kb), "jsonl"), Plan::Sandbox);
        // 聚合型但小 → 骨架。
        assert_eq!(route(Some(10 * kb), "log"), Plan::Skeleton);
        // >100KB 恒外置（包括聚合型）。
        assert_eq!(route(Some(101 * kb), "log"), Plan::Externalize);
        assert_eq!(route(None, "log"), Plan::Skeleton); // unknown size ⇒ never guess
    }

    #[test]
    fn aggregate_shape_heuristic() {
        assert!(is_aggregate_ext("log"));
        assert!(is_aggregate_ext("jsonl"));
        assert!(!is_aggregate_ext("rs"));
        assert!(!is_aggregate_ext("md"));
    }
}
