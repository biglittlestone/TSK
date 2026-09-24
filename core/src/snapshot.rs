//! PreCompact snapshot + the SessionStart resume injection it feeds.
//!
//! See spec §7.4 (extraction) and §8.3 (assembly with the hard-truncation pass).
//! Everything here reads the transcript and never calls a model — any parse
//! failure degrades to an empty field and the hook keeps going (fail-open).

use crate::config::{
    INJECT_BUDGET, INJECT_P1_MAX_CHARS, INJECT_P2_N, INJECT_P3_N, SNAPSHOT_MAX,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What a snapshot carries. Ordered by priority: P1 role, P2 decisions,
/// P3 skills, P4 intent. P1 is never truncated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub session: String,
    pub role: String,
    pub decisions: Vec<String>,
    pub skills: Vec<String>,
    pub intent: String,
}

/// Read the transcript JSONL and extract role / decisions / skills / intent.
pub fn build(path: &Path, session: &str) -> Snapshot {
    let lines = std::fs::read_to_string(path).unwrap_or_default();
    let recs: Vec<serde_json::Value> =
        lines.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();

    let mut users: Vec<String> = Vec::new();
    let mut assistants: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();

    for r in &recs {
        match r.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "user" => {
                if let Some(t) = user_text(r) {
                    users.push(t);
                }
            }
            "assistant" => {
                let txt = assistant_text(r);
                collect_skills(r, &mut skills);
                if !txt.is_empty() {
                    assistants.push(txt);
                }
            }
            _ => {}
        }
    }

    let role = first_user_or_paths(&users, &recs);
    let decisions = decisions(&assistants);
    let intent = users
        .last()
        .map(|s| s.chars().take(200).collect())
        .unwrap_or_default();

    Snapshot {
        v: 1,
        session: session.to_string(),
        role,
        decisions,
        skills: last_n(&dedup(skills), INJECT_P3_N),
        intent,
    }
}

/// The last `n` items, in original order.
fn last_n<T: Clone>(v: &[T], n: usize) -> Vec<T> {
    v.iter().rev().take(n).cloned().collect::<Vec<_>>().into_iter().rev().collect()
}

/// First user message, truncated — or, when it is too thin, the paths most
/// frequently touched in the session.
fn first_user_or_paths(users: &[String], recs: &[serde_json::Value]) -> String {
    if let Some(first) = users.first() {
        let s: String = first.chars().take(INJECT_P1_MAX_CHARS).collect();
        if s.trim().len() >= 20 {
            return s;
        }
    }
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in recs {
        if let Some(p) = r.pointer("/tool_input/file_path").and_then(|v| v.as_str()) {
            *counts.entry(p.to_string()).or_default() += 1;
        }
    }
    if counts.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let paths: Vec<String> = pairs.into_iter().map(|(p, _)| p).take(6).collect();
    format!("Working on: {}", paths.join(", "))
}

/// Decision-shaped lines in the assistant's own text, most recent first, 200 chars each.
fn decisions(assistants: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for msg in assistants.iter().rev() {
        for line in msg.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_lowercase();
            let is_decision = lower.starts_with("decision:")
                || lower.starts_with("决定:")
                || lower.starts_with("决定：")
                || ["we will use", "we will write", "chosen", "采用", "选定"]
                    .iter()
                    .any(|k| lower.contains(k));
            if is_decision {
                let s: String = t.chars().take(200).collect();
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out.into_iter().take(INJECT_P2_N).collect()
}

/// Tool names actually used + a language word inferred from paths touched.
fn collect_skills(r: &serde_json::Value, skills: &mut Vec<String>) {
    for b in r.pointer("/message/content").and_then(|c| c.as_array()).into_iter().flatten() {
        if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
            if let Some(n) = b.get("name").and_then(|n| n.as_str()) {
                skills.push(n.to_string());
                if let Some(p) = b.pointer("/input/file_path").and_then(|v| v.as_str()) {
                    if let Some(e) = std::path::Path::new(p).extension().and_then(|e| e.to_str()) {
                        push_lang(skills, &ext_to_lang(e));
                    }
                }
            }
        }
    }
}

fn push_lang(v: &mut Vec<String>, l: &str) {
    if !l.is_empty() && !v.contains(&l.to_string()) {
        v.push(l.to_string());
    }
}

fn ext_to_lang(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" | "mjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "sh" | "bash" | "zsh" => "shell",
        "toml" | "ini" => "config",
        "md" => "markdown",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "html" | "css" => "web",
        _ => "",
    }
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(v.len());
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

fn user_text(r: &serde_json::Value) -> Option<String> {
    let c = r.get("message").and_then(|m| m.get("content"))?;
    if let Some(s) = c.as_str() {
        return Some(s.to_string());
    }
    let arr = c.as_array()?;
    for b in arr {
        if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
            continue; // not a human turn
        }
        if let Some(s) = b.get("text").and_then(|t| t.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

fn assistant_text(r: &serde_json::Value) -> String {
    let Some(arr) = r.pointer("/message/content").and_then(|c| c.as_array()) else {
        return String::new();
    };
    arr.iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serialize the snapshot, capped at `SNAPSHOT_MAX`. Dropping happens in
/// priority order P4 → P3 → P2 (same order as the assembler).
pub fn write_snapshot(path: &Path, snap: &Snapshot) {
    let mut s = snap.clone();
    loop {
        let bytes = serde_json::to_string(&s).map(|v| v.len()).unwrap_or(SNAPSHOT_MAX + 1);
        if bytes <= SNAPSHOT_MAX
            || (s.intent.is_empty() && s.skills.is_empty() && s.decisions.is_empty())
        {
            break;
        }
        if !s.intent.is_empty() {
            s.intent.clear();
        } else if !s.skills.is_empty() {
            s.skills.pop();
        } else if !s.decisions.is_empty() {
            s.decisions.pop();
        } else {
            break;
        }
    }
    let _ = std::fs::write(path, serde_json::to_string(&s).unwrap_or_default());
}

/// Assemble the resume injection. See §8.3 — the context-mode 5→3 fallback plus
/// TSK's hard-truncation pass, which the original lacks (spike CS-4).
pub fn assemble(snap: &Snapshot) -> String {
    assemble_with(snap, INJECT_BUDGET)
}

/// The assembly loop with an injectable budget. Testable without global state.
fn assemble_with(snap: &Snapshot, budget: usize) -> String {
    let est = |s: &str| crate::est_tokens(s);
    let fmt = |v: &[String]| v.join("; ");

    let p1: String = snap.role.chars().take(INJECT_P1_MAX_CHARS).collect();
    let mut dec: Vec<String> = last_n(&snap.decisions, INJECT_P2_N);
    let mut skl: Vec<String> = last_n(&snap.skills, INJECT_P3_N);
    let mut p4: String = snap.intent.clone();

    let total = |dec: &[String], skl: &[String], p4: &str| {
        est(&p1) + est(&fmt(dec)) + est(&fmt(skl)) + est(p4)
    };

    // context-mode's original: over budget, keep only the 3 most recent decisions.
    if total(&dec, &skl, &p4) > budget {
        while dec.len() > 3 {
            dec.pop();
        }
    }

    // TSK hard-truncation pass: P4 first, then P3 one at a time, then P2.
    // P1 (~400 chars ≈ 100 tok) is never touched, so the loop always terminates.
    while total(&dec, &skl, &p4) > budget {
        let dropped = if !p4.is_empty() {
            p4.clear();
            true
        } else if skl.len() > 1 {
            skl.pop();
            true
        } else if dec.len() > 1 {
            dec.pop();
            true
        } else {
            false
        };
        if !dropped {
            break;
        }
    }

    format!(
        "ROLE: {p1}\nDECISIONS: {}\nSKILLS: {}\nINTENT: {p4}",
        fmt(&dec),
        fmt(&skl)
    )
}
