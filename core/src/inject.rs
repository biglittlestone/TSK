//! Where the three compression lines meet the hook protocol: input (skeleton /
//! externalize), output (injected style), history (snapshot + compact advice).
//! See spec §7.

use crate::compress::{externalize, skeleton};
use crate::config::{CompressionLevel, Config, COMPACT_ADVICE_THRESHOLD};
use crate::events::{Event, HookEvent, Output};
use crate::ledger::{self, Entry};
use crate::policy;
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
pub fn pretooluse(_cfg: &Config, ev: &HookEvent) -> Output {
    let input = &ev.tool_input;
    let Some(orig) = input.get("file_path").and_then(|v| v.as_str()) else {
        return Output::Passthrough
    };
    let orig_path = Path::new(orig);
    // Escape hatch: an explicit offset/limit is a retrieval, never a compression.
    if policy::is_escape(input) {
        let bytes = file_bytes(orig_path);
        ledger::append(
            &ev.session_id,
            &Entry::Retrieval(ledger::RetrievalEntry {
                kind: "retrieval",
                ts: crate::now_millis(),
                session: ev.session_id.clone(),
                tool: "Read".into(),
                reason: "escape_hatch_offset_limit".into(),
                orig_bytes: bytes,
            }),
        );
        let orig = orig_path.to_string_lossy();
        log_event(&ev.session_id, &format!("retrieval: escaped via offset/limit, {} bytes ({orig})", bytes.unwrap_or(0)));
        return Output::Passthrough;
    }

    // 外置全文回读 = 显式检索：ext/ 目录下的文件 Read 恒 passthrough。
    // 否则外置文件（通常 >100KB）被再次 externalize → 指针→再压缩→死循环。
    if orig_path.starts_with(storage::ext_dir(&ev.session_id)) {
        return Output::Passthrough;
    }

    let Some(text) = std::fs::read_to_string(orig_path).ok() else {
        return Output::Passthrough
    };
    let bytes = text.len();
    if bytes == 0 {
        return Output::Passthrough
    }
    let project = Path::new(&ev.cwd);
    let rel = if orig_path.starts_with(project) {
        orig_path.strip_prefix(project).unwrap_or(orig_path).to_string_lossy().into_owned()
    } else {
        orig.to_string()
    };
    let ext = orig_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // >100KB 无条件外置（ext 路径无行数要求——单行 200KB 的 minified JSON 也应外置）。
    // 行数门控只约束骨架路径（n ≥ MIN_LINES），不拦外部化/沙箱。
    if bytes > crate::config::EXTERNALIZE_THRESHOLD {
        return plan_externalize(ev, bytes, &text);
    }
    // ≤100KB：骨架需 ≥8 行；聚合型大文件仍可走沙箱候选。
    if skeleton::skeleton(&text, &rel).is_none() {
        if !(policy::is_aggregate_ext(ext) && bytes > crate::config::ROUTE_THRESHOLD) {
            return Output::Passthrough
        }
    }

    // [沙箱 opt-in] 项目若显式预置 .tsk/sandbox/analyze_<ext> 脚本 → 优先用其摘要替代 read
    //（这是项目方自己的选择，不是引擎猜测：不看大小/聚合启发）。摘要必须严格小于原文，否则回退。
    if let Some(out) = try_sandbox_plan(project, ext, orig_path, ev, bytes) {
        return out
    }

    match policy::route(Some(bytes), ext) {
        policy::Plan::Externalize => plan_externalize(ev, bytes, &text),
        policy::Plan::Sandbox | policy::Plan::Skeleton => {
            // 沙箱已在上方尝试过（脚本存在才成立）；无脚本/失败一律回退骨架（保守，§7.1 禁猜测）。
            plan_skeleton(project, orig_path, ev, bytes, &text, &rel)
        }
    }
}

/// Externalize and rewrite the Read target to the pointer file.
fn plan_externalize(ev: &HookEvent, bytes: usize, text: &str) -> Output {
    let Some(x) = externalize::externalize(&ev.session_id, text) else {
        return Output::Passthrough
    };
    ledger::append(
        &ev.session_id,
        &Entry::Externalize(ledger::ExternalizeEntry {
            kind: "externalize",
            ts: crate::now_millis(),
            session: ev.session_id.clone(),
            tool: "Read".into(),
            orig_bytes: bytes,
            new_bytes: x.pointer_bytes,
        }),
    );
    log_event(
        &ev.session_id,
        &format!(
            "externalize: {} -> {} bytes, pointer {}",
            bytes,
            x.pointer_bytes,
            x.pointer.display()
        ),
    );
    rewrite(&ev.tool_input.clone(), &x.pointer)
}

/// Skeleton path of the decision table. Shared by the plain route and the
/// sandbox-fallback route (§7.1: sandbox 失败 → 骨架 → 原文件 passthrough).
fn plan_skeleton(project: &Path, orig_path: &Path, ev: &HookEvent, bytes: usize, text: &str, rel: &str) -> Output {
    let sk = skeleton::skeleton(text, rel).unwrap_or_default();
    let sk_dir = storage::skeleton_dir(project);
    storage::ensure_dir(&sk_dir);
    // tokenizer-count gate（文档 §11.3）：骨架不得大于原文，否则回退原值。
    if sk.len() >= bytes {
        return Output::Passthrough;
    }
    let path = storage::derive_skeleton_path(project, orig_path);
    if std::fs::write(&path, &sk).is_err() {
        return Output::Passthrough;
    }
    ledger::append(
        &ev.session_id,
        &Entry::Rewrite(ledger::RewriteEntry {
            kind: "rewrite",
            ts: crate::now_millis(),
            session: ev.session_id.clone(),
            tool: "Read".into(),
            tu_id: ev.tool_use_id.clone().unwrap_or_default(),
            strategy: "skeleton".into(),
            orig_bytes: bytes,
            new_bytes: sk.len(),
            saved: bytes.saturating_sub(sk.len()),
        }),
    );
    rewrite(&ev.tool_input.clone(), &path)
}

/// Run the project's pre-placed `analyze_<ext>` script in the sandbox; its stdout
/// (the conclusions) is written to `ext/<sha1>.summary.txt` and becomes the Read
/// target. `None` means "cannot sandbox — fall back to skeleton".
fn try_sandbox_plan(project: &Path, ext: &str, orig_path: &Path, ev: &HookEvent, bytes: usize) -> Option<Output> {
    // 约定：`<project>/.tsk/sandbox/analyze_<ext>.{sh,py,js}`（spec §7.1）。
    let mut base = project.join(".tsk/sandbox").join(format!("analyze_{ext}"));
    // 也允许全局沙箱脚本目录（env TSK_SANDBOX_DIR / analyze_<ext>），避免在模型可见工作区放 TSK 产物。
    if !base.exists() {
        if let Ok(dir) = std::env::var("TSK_SANDBOX_DIR") {
            let g = std::path::Path::new(&dir).join(format!("analyze_{ext}"));
            for e in ["sh", "py", "js"] {
                let cand = g.with_extension(e);
                if cand.exists() { base = cand; break; }
            }
        }
    }
    let script = ["sh", "py", "js"]
        .iter()
        .map(|e| base.with_extension(e))
        .find(|p| p.exists())?;
    let sb = crate::sandbox::run(&script, &[], orig_path).ok()?;
    if sb.stdout.trim().is_empty() {
        return None;
    }
    let dir = storage::ext_dir(&ev.session_id);
    storage::ensure_dir(&dir);
    // 恪守 SANDBOX_MAX_OUTPUT（同 `tsk exec`）：超限时全文外置，summary 只进截断+提示。
    let mut summary_txt = sb.stdout.clone();
    if summary_txt.len() > crate::config::SANDBOX_MAX_OUTPUT {
        let h_full = storage::sha1_hex(summary_txt.as_bytes());
        let full_path = dir.join(format!("{h_full}.txt"));
        let _ = std::fs::write(&full_path, &summary_txt);
        let keep = summary_txt.split('\n').take(200).collect::<Vec<_>>().join("\n");
        summary_txt = format!(
            "{keep}\n[TSK: output truncated; {} bytes total. Full text: {}]\n",
            sb.stdout.len(),
            full_path.display()
        );
    }
    // 守护：沙箱摘要不得大于原文，否则不采用（回退骨架/原文件）。
    if summary_txt.len() >= bytes {
        return None;
    }
    let h = storage::sha1_hex(summary_txt.as_bytes());
    let summary = dir.join(format!("{h}.summary.txt"));
    if std::fs::write(&summary, &summary_txt).is_err() {
        return None;
    }
    ledger::append(
        &ev.session_id,
        &Entry::Sandbox(ledger::SandboxEntry {
            kind: "sandbox",
            ts: crate::now_millis(),
            session: ev.session_id.clone(),
            tool: "Read".into(),
            orig_bytes: bytes,
            new_bytes: summary_txt.len(),
            saved: bytes.saturating_sub(summary_txt.len()),
        }),
    );
    log_event(
        &ev.session_id,
        &format!("sandbox: analyze_{ext} -> {} -> {} bytes", bytes, summary_txt.len()),
    );
    Some(rewrite(&ev.tool_input.clone(), &summary))
}

fn rewrite(input: &serde_json::Value, new_path: &Path) -> Output {
    let mut v = input.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.insert("file_path".into(), serde_json::Value::String(new_path.to_string_lossy().into_owned()));
    }
    Output::UpdatedInput(v)
}

/// Size of a file, if we can read its metadata. Used only for ledger accuracy.
fn file_bytes(p: &Path) -> Option<usize> {
    std::fs::metadata(p).ok().map(|m| m.len() as usize)
}

/// PostToolUse: v1 never rewrites a tool result (spike S2), so this is always a
/// passthrough. Wired as the v1.5 observation port: each call is recorded in
/// events.log so future cross-hook accounting can study tool completion.
pub fn posttooluse(_cfg: &Config, ev: &HookEvent) -> Output {
    log_event(
        &ev.session_id,
        &format!(
            "posttooluse: tool={} tu_id={}",
            ev.tool_name.as_deref().unwrap_or("?"),
            ev.tool_use_id.as_deref().unwrap_or("?")
        ),
    );
    Output::Passthrough
}

/// SessionStart: the long ruleset, plus a resume injection when returning.
/// In `auto` mode the ruleset (fixed cost ~967 tok) is *not* injected here —
/// whether output compression pays back depends on session length, which is
/// decided per-turn in `user_prompt` (break-even ≈ 3 turns).
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
