//! Where the three compression lines meet the hook protocol: input (skeleton /
//! externalize), output (injected style), history (snapshot + compact advice).
//! See spec §7.

use crate::config::{CompressionLevel, Config, COMPACT_ADVICE_THRESHOLD};
use crate::events::{Event, HookEvent, Output};
use crate::ledger::{self, Entry};
use crate::snapshot;
use crate::storage;
use std::path::Path;

const RULESET: &str = include_str!("assets/ruleset.txt");
const LITE_RULESET: &str = include_str!("assets/ruleset_lite.txt");
const REINFORCE: &str = include_str!("assets/reinforce.txt");

/// Ruleset per compression level: lite is a shorter subset (measured -46% at
/// ~1500 chars, spike S3); full/ultra/auto use the full measured ruleset.
fn ruleset_for(level: CompressionLevel) -> &'static str {
    match level {
        CompressionLevel::Lite => LITE_RULESET,
        _ => RULESET,
    }
}

const PROTECT_MARKERS: &str = "The context window is being compacted. Preserve verbatim every line starting \
   with '[TSK SKELETON of' and every '[TSK ' marker line. These are retrieval anchors; do not summarize or drop them.";

/// PreToolUse(Read): the only rewriting point.
pub fn session_start(cfg: &Config, ev: &HookEvent) -> Output {
    let mut ctx = String::new();
    // 输出压缩：仅 `compression_on` 且非 `auto`（auto 由 UserPrompt 按轮数决定）。
    if cfg.compression_on() && cfg.output_compression != CompressionLevel::Auto {
        let rs = ruleset_for(cfg.output_compression);
        ctx.push_str(rs);
        ctx.push('\n');
        ledger::append(
            &ev.session_id,
            &Entry::Inject(ledger::InjectEntry {
                kind: "inject",
                ts: crate::now_millis(),
                session: ev.session_id.clone(),
                source: "session_start_ruleset".into(),
                bytes: rs.len(),
            }),
        );
    }

    // 历史线 resume：独立于输出压缩开关（无损，应始终工作）。仅受 enabled + source 约束。
    if cfg.enabled && matches!(ev.source.as_deref(), Some("resume") | Some("compact")) {
        let sp = storage::session_dir(&ev.session_id).join("resume-snapshot.json");
        if let Ok(s) = std::fs::read_to_string(&sp) {
            if let Ok(snap) = serde_json::from_str::<snapshot::Snapshot>(&s) {
                let inj = snapshot::assemble(&snap);
                ctx.push_str("\n\nRESUME:\n");
                ctx.push_str(&inj);
                ledger::append(
                    &ev.session_id,
                    &Entry::Inject(ledger::InjectEntry {
                        kind: "inject",
                        ts: crate::now_millis(),
                        session: ev.session_id.clone(),
                        source: "resume_snapshot".into(),
                        bytes: inj.len(),
                    }),
                );
            }
        }
    }
    if ctx.is_empty() {
        return Output::Passthrough;
    }
    Output::Context(Event::SessionStart, ctx)
}

/// UserPromptSubmit: per-turn reinforcement, then the /compact advice.
/// In `auto` mode, output compression only starts once the session has enough
/// turns for the fixed ruleset cost to pay back (break-even ≈ 3 turns); short
/// sessions pay nothing.
pub fn user_prompt(cfg: &Config, ev: &HookEvent) -> Output {
    let mut ctx = String::new();

    if cfg.compression_on() {
        if cfg.output_compression == CompressionLevel::Auto {
            let turns = prompt_turns(&ev.session_id);
            log_prompt_turn(&ev.session_id);
            if turns >= 2 {
                // 第 3 轮起注入：ruleset 只注入一次，之后每轮 reinforce。
                if !ruleset_injected(&ev.session_id) {
                    ctx.push_str(ruleset_for(cfg.output_compression));
                    ledger::append(
                        &ev.session_id,
                        &Entry::Inject(ledger::InjectEntry {
                            kind: "inject",
                            ts: crate::now_millis(),
                            session: ev.session_id.clone(),
                            source: "auto_ruleset".into(),
                            bytes: RULESET.len(),
                        }),
                    );
                }
                ctx.push_str(REINFORCE);
                ledger::append(
                    &ev.session_id,
                    &Entry::Inject(ledger::InjectEntry {
                        kind: "inject",
                        ts: crate::now_millis(),
                        session: ev.session_id.clone(),
                        source: "per_turn_reinforce".into(),
                        bytes: REINFORCE.len(),
                    }),
                );
            }
        } else {
            ctx.push_str(REINFORCE);
            ledger::append(
                &ev.session_id,
                &Entry::Inject(ledger::InjectEntry {
                    kind: "inject",
                    ts: crate::now_millis(),
                    session: ev.session_id.clone(),
                    source: "per_turn_reinforce".into(),
                    bytes: REINFORCE.len(),
                }),
            );
        }
    }

    if let Some(tp) = ev.transcript_path.as_deref().map(Path::new) {
        let used = last_assistant_usage(tp).unwrap_or(0) as usize;
        if used > COMPACT_ADVICE_THRESHOLD && !already_advised(&ev.session_id) {
            if !ctx.is_empty() {
                ctx.push('\n');
            }
            let advice = "Context is above 180k tokens; consider running /compact.";
            ctx.push_str(advice);
            log_event(&ev.session_id, "compact_advice: context above 180k, /compact suggested");
            ledger::append(
                &ev.session_id,
                &Entry::Inject(ledger::InjectEntry {
                    kind: "inject",
                    ts: crate::now_millis(),
                    session: ev.session_id.clone(),
                    source: "compact_advice".into(),
                    bytes: advice.len(),
                }),
            );
        }
    }

    if ctx.is_empty() {
        Output::Passthrough
    } else {
        Output::Context(Event::UserPromptSubmit, ctx)
    }
}

/// PreCompact: build the snapshot, then ask the compactor to keep the anchors.
pub fn precompact(cfg: &Config, ev: &HookEvent) -> Output {
    if !cfg.enabled {
        return Output::Passthrough;
    }
    let Some(tp) = ev.transcript_path.as_deref().map(Path::new) else {
        return Output::Passthrough
    };
    let snap = snapshot::build(tp, &ev.session_id);
    storage::ensure_dir(&storage::session_dir(&ev.session_id));
    snapshot::write_snapshot(&storage::session_dir(&ev.session_id).join("resume-snapshot.json"), &snap);
    let evtext = format!("role: {}\nintent: {}", snap.role.chars().take(120).collect::<String>(), snap.intent.chars().take(120).collect::<String>());
    log_event(&ev.session_id, &evtext);
    ledger::append(
        &ev.session_id,
        &Entry::Inject(ledger::InjectEntry {
            kind: "inject",
            ts: crate::now_millis(),
            session: ev.session_id.clone(),
            source: "precompact_anchor_protection".into(),
            bytes: PROTECT_MARKERS.len(),
        }),
    );
    Output::Context(Event::PreCompact, PROTECT_MARKERS.to_string())
}

/// Sum of the last assistant turn's input tokens (incl. cache reads). `None` ⇒ fail-open.
pub fn last_assistant_usage(transcript: &Path) -> Option<u64> {
    let lines = std::fs::read_to_string(transcript).ok()?;
    for line in lines.lines().rev() {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let u = v.pointer("/message/usage")?;
        let input = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        let cached = u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        let cache_write = u.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        return Some(input.saturating_add(cached).saturating_add(cache_write));
    }
    None
}

fn already_advised(session: &str) -> bool {
    let p = storage::session_dir(session).join("events.log");
    std::fs::read_to_string(p)
        .map(|s| s.lines().any(|l| l.contains("compact_advice")))
        .unwrap_or(false)
}

/// One line per event, `[ts] category: content` — grep-friendly (spike CS-2).
/// `already_advised` is the matching read on the same file.
/// Number of user turns so far (`auto` output compression uses this to defer
/// injection until the fixed ruleset cost pays back). Counts events.log rows.
fn prompt_turns(session: &str) -> usize {
    let p = storage::session_dir(session).join("events.log");
    std::fs::read_to_string(p)
        .map(|s| s.matches("prompt_turn").count())
        .unwrap_or(0)
}

fn log_prompt_turn(session: &str) {
    log_event(session, "prompt_turn: user message submitted");
}

/// True once a ruleset was injected this session (auto mode injects at most once).
fn ruleset_injected(session: &str) -> bool {
    let p = storage::session_dir(session).join("ledger.jsonl");
    std::fs::read_to_string(p)
        .map(|s| s.contains("auto_ruleset") || s.contains("session_start_ruleset"))
        .unwrap_or(false)
}

/// One line per event, `[ts] category: content` — grep-friendly (spike CS-2).
/// Public so the CLI can record its own decisions (e.g. `tsk exec`) into the
/// same session log.
pub fn log_event(session: &str, body: &str) {
    let dir = storage::session_dir(session);
    storage::ensure_dir(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join("events.log"))
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("[{ts}] {body}\n").as_bytes()));
}

#[cfg(test)]
mod t {
    use super::*;
    use crate::config::RULESET_MIN_CHARS;

    #[test]
    fn ruleset_meets_the_measured_length_floor() {
        // Below RULESET_MIN_CHARS the injection is diluted by models (spike S3).
        assert!(
            RULESET.chars().count() >= RULESET_MIN_CHARS,
            "ruleset is {} chars, needs {}",
            RULESET.chars().count(),
            RULESET_MIN_CHARS
        );
    }

    #[test]
    fn reinforcement_is_the_short_reemit() {
        assert!(REINFORCE.chars().count() <= 400);
        assert!(REINFORCE.contains("no pleasantries"));
    }

    #[test]
    fn marker_protection_text_is_self_describing() {
        assert!(PROTECT_MARKERS.contains("[TSK SKELETON of"));
        assert!(PROTECT_MARKERS.contains("do not summarize or drop"));
    }
}
