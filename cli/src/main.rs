//! TSK command-line entry. One catch point: any failure inside a hook prints a
//! note to stderr and still answers `{}` with exit 0 (fail-open, spec §10).
//! Business logic lives in `tsk-core`; this file only orchestrates stdio.

use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tsk_core::{Config, events, storage};

#[derive(Parser)]
#[command(name = "tsk", version, about = "TSK — token saving kit")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install: merge TSK hooks into <project>/.claude/settings.json + write ~/.tsk/config.yaml.
    Init {
        /// Project directory. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Turn TSK off: set `enabled: false` and remove TSK's hooks from settings.json.
    Off {
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Handle an agent hook event. Reads the event JSON on stdin.
    Hook { event: String },
    /// Ledger report: total saved, per-tool breakdown, retrieval count.
    Report {
        /// A specific session id. Default: the most recent ("current") session.
        #[arg(long)]
        session: Option<String>,
        /// Report across all sessions (overrides the current-session default).
        #[arg(long)]
        all: bool,
        /// Reset the ledger: delete all session ledger lines + the current marker.
        #[arg(long)]
        clean: bool,
        #[arg(long)]
        cache_hit: Option<PathBuf>,
    },
    /// Self-check: config, writability, hook wiring, PATH.
    Doctor {
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Run `script` in the sandbox against `input`, printing only its stdout.
    Exec {
        script: PathBuf,
        input: PathBuf,
        #[arg(long, default_value = "cli")]
        session: String,
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() {
    let output = match run() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[tsk] fail-open: {e}");
            String::new()
        }
    };
    // Stdout is written last, always complete and valid JSON.
    if output.is_empty() {
        print!("{{}}");
    } else {
        print!("{output}");
    }
}

fn run() -> Result<String, String> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Hook { event } => {
            let ev = events::HookEvent::read(&mut std::io::stdin().lock())
                .map_err(|e| format!("parse stdin: {e}"))?;
            // 记录当前会话，供 `tsk report`（无参数）默认统计本会话。
            storage::write_current_session(&ev.session_id);
            let cfg = Config::load(&storage::tsk_home().join("config.yaml"));
            Ok(match events::Event::parse(&event) {
                Some(events::Event::PreToolUse) => tsk_core::inject::pretooluse(&cfg, &ev).to_string(),
                Some(events::Event::PostToolUse) => tsk_core::inject::posttooluse(&cfg, &ev).to_string(),
                Some(events::Event::SessionStart) => tsk_core::inject::session_start(&cfg, &ev).to_string(),
                Some(events::Event::UserPromptSubmit) => tsk_core::inject::user_prompt(&cfg, &ev).to_string(),
                Some(events::Event::PreCompact) => tsk_core::inject::precompact(&cfg, &ev).to_string(),
                None => return Err(format!("unknown event `{event}`")),
            })
        }
        Cmd::Init { project } => Ok(init(&project).to_string()),
        Cmd::Off { project } => Ok(off(&project).to_string()),
        Cmd::Report { session, all, clean, cache_hit } => Ok(report(session, all, clean, cache_hit).to_string()),
        Cmd::Doctor { project } => Ok(doctor(&project).to_string()),
        Cmd::Exec { script, input, session, args } => Ok(exec(&script, &input, &session, &args)?),
    }
}

/// Install is additive: TSK's hook entries are appended, never replacing anything.
/// Idempotent: config already present keeps the user's settings, hooks are only
/// ensured present (never duplicated), so `tsk off` → `tsk init` re-opens cleanly.
fn init(project: &PathBuf) -> String {
    let cfg_path = storage::tsk_home().join("config.yaml");
    storage::ensure_dir(&storage::tsk_home());
    let mut ok = true;
    if cfg_path.is_file() {
        println!("tsk: config already present at {}", cfg_path.display());
    } else if std::fs::write(&cfg_path, Config::default_yaml()).is_err() {
        ok = false;
    }
    match merge_hooks(project, true) {
        Ok(_) => println!("tsk: installed hooks in {}", project.join(".claude/settings.json").display()),
        Err(e) => {
            println!("tsk: could not wire hooks: {e}");
            ok = false;
        }
    }
    if ok {
        format!("tsk: enabled. Output compression is OFF until you edit {}\n", cfg_path.display())
    } else {
        format!("tsk: installed incompletely (see above)\n")
    }
}

/// `tsk off` is idempotent: config flipped, hooks stripped, no residue.
fn off(project: &PathBuf) -> String {
    let cfg_path = storage::tsk_home().join("config.yaml");
    let mut cfg = Config::load(&cfg_path);
    cfg.enabled = false;
    let mut out = String::new();
    if storage::ensure_parent_dir(&cfg_path) {
        // 保留 init 写入的注释：只翻转 enabled 行，不整文件重写（to_yaml 会丢注释）。
        let new_cfg = std::fs::read_to_string(&cfg_path)
            .ok()
            .map(|t| disable_enabled_in_yaml(&t))
            .or_else(|| cfg.to_yaml().ok());
        match new_cfg {
            Some(s) => {
                if std::fs::write(&cfg_path, s).is_err() {
                    out.push_str("tsk: could not write config\n");
                } else {
                    out.push_str("tsk: config disabled\n");
                }
            }
            None => out.push_str("tsk: could not serialize config\n"),
        }
    } else {
        out.push_str("tsk: no config file\n");
    }
    match merge_hooks(project, false) {
        Ok("removed") => out.push_str("tsk: hooks removed\n"),
        Ok(_) => out.push_str("tsk: no TSK hooks found\n"),
        Err(e) => out.push_str(&format!("tsk: could not remove hooks: {e}\n")),
    }
    out
}

/// `bytes/4` ceil — same estimate as core (est tokens).
fn est_tok(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// Flip `enabled: true` → `enabled: false` in the config text, keeping the comment
/// lines so `tsk off` doesn't strip the level documentation `tsk init` wrote.
fn disable_enabled_in_yaml(txt: &str) -> String {
    txt.split('\n')
        .map(|l| {
            if l.contains("enabled: true") {
                l.replace("enabled: true", "enabled: false")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn report(session: Option<String>, all: bool, clean: bool, cache_hit_path: Option<PathBuf>) -> String {
    if let Some(p) = cache_hit_path {
        return cache_hit(&p);
    }
    if clean {
        return clear_ledgers();
    }
    if let Some(sid) = session {
        let entries = tsk_core::ledger::read_all(&storage::session_dir(&sid).join("ledger.jsonl"));
        return format!("session {sid}\n{}", summarize(&entries));
    }
    if all {
        return report_all();
    }
    // 默认：当前会话（最后一钩子记录的 session）。无记录则退回全量。
    match storage::read_current_session() {
        Some(sid) => {
            let entries = tsk_core::ledger::read_all(&storage::session_dir(&sid).join("ledger.jsonl"));
            format!("current session {sid}\n{}", summarize(&entries))
        }
        None => report_all(),
    }
}

fn report_all() -> String {
    let mut total: usize = 0;
    let mut cost: usize = 0;
    let mut tools: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut sessions = 0usize;
    if let Ok(rd) = std::fs::read_dir(storage::tsk_home()) {
        for e in rd.flatten() {
            let p = e.path().join("ledger.jsonl");
            if !p.is_file() {
                continue;
            }
            sessions += 1;
            for x in tsk_core::ledger::read_all(&p) {
                total += saved_of(&x);
                cost += cost_of(&x);
                *tools.entry(tool_of(&x).unwrap_or("?").to_string()).or_insert(0) += saved_of(&x);
            }
        }
    }
    let net = total.saturating_sub(cost);
    let mut s = format!(
        "saved: {total} bytes (≈ {} est tokens) across {sessions} session(s)\n\
         inject cost: {cost} bytes (≈ {} est tokens)\n\
         net: {net} bytes\n",
        est_tok(total),
        est_tok(cost)
    );
    for (k, v) in &tools {
        s.push_str(&format!("  {k}: {v}\n"));
    }
    s
}

/// `report --clean`: remove every ledger + the current-session marker (reset stats).
fn clear_ledgers() -> String {
    let mut n = 0usize;
    if let Ok(rd) = std::fs::read_dir(storage::tsk_home()) {
        for e in rd.flatten() {
            let p = e.path().join("ledger.jsonl");
            if p.is_file() {
                if std::fs::remove_file(&p).is_ok() {
                    n += 1;
                }
            }
        }
    }
    let _ = std::fs::remove_file(storage::current_session_file());
    format!("tsk: cleared {n} ledger file(s) + current-session marker\n")
}

/// Estimated-token cost of one entry (injection bytes for `inject`, else 0).
fn cost_of(e: &tsk_core::ledger::Entry) -> usize {
    match e {
        tsk_core::ledger::Entry::Inject(x) => x.bytes,
        _ => 0,
    }
}

fn doctor(project: &PathBuf) -> String {
    let mut ok = true;
    let lines = vec![
        check("config present", Config::load(&storage::tsk_home().join("config.yaml")).enabled),
        check("~/.tsk writable", storage::tsk_home().exists() || std::fs::create_dir_all(&storage::tsk_home()).is_ok()),
        check(
            "settings.json wired",
            serde_json::from_str::<Value>(&std::fs::read_to_string(project.join(".claude/settings.json")).unwrap_or_default())
                .map(|v| has_tsk_hook(&v))
                .unwrap_or(false)
                // 插件方式：hooks 声明在插件的 plugin.json 里、自动合并，无需项目 settings.json。
                || plugin_installed(),
        ),
        check("tsk in PATH", tsk_on_path()),
    ];
    for (label, good) in &lines {
        if !good {
            ok = false;
        }
        println!("[{}] {label}", if *good { "ok" } else { "FAIL" });
    }
    // 返回非空状态行，避免 main 的 fail-open 兜底在 stdout 尾缀 `{}`。
    format!("doctor: {}\n", if ok { "all checks passed" } else { "some checks failed" })
}

/// True when the `tsk` executable resolves from any PATH element
/// (`std::env::split_paths` handles both `:` and `;` separators correctly).
fn tsk_on_path() -> bool {
    let bin = if cfg!(windows) { "tsk.exe" } else { "tsk" };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// True when the TSK Claude skills-dir plugin is installed — its hooks come from
/// the plugin's own plugin.json (auto-merged), so a project settings.json is not
/// required in that mode.
fn plugin_installed() -> bool {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    home.map(|h| {
        Path::new(&h)
            .join(".claude")
            .join("skills")
            .join("tsk")
            .join(".claude-plugin")
            .join("plugin.json")
            .is_file()
    })
    .unwrap_or(false)
}

fn exec(script: &PathBuf, input: &PathBuf, session: &str, args: &Vec<String>) -> Result<String, String> {
    // 记账语义：orig_bytes = 不沙箱时进 context 的原始数据量（输入文件大小），
    // new_bytes = 实际结论大小。沙箱收益 = 输入数据 - 结论。
    let input_bytes = std::fs::metadata(input).map(|m| m.len() as usize).unwrap_or(0);
    let (out_len, stdout) = match tsk_core::sandbox::run(script, args, input) {
        Ok(r) => (r.stdout.len(), r.stdout),
        Err(e) => {
            if let Some(r) = e.downcast_ref::<tsk_core::sandbox::Refusal>() {
                return Err(format!("sandbox: input is {} bytes, above the {} byte cap", r.input_bytes, tsk_core::config::SANDBOX_HARD_CAP));
            }
            return Err(format!("sandbox: failed to run script: {e}"));
        }
    };
    let mut shown = stdout;
    if shown.len() > tsk_core::config::SANDBOX_MAX_OUTPUT {
        let dir = storage::ext_dir(session);
        storage::ensure_dir(&dir);
        let h = storage::sha1_hex(shown.as_bytes());
        let full = dir.join(format!("{h}.txt"));
        let _ = std::fs::write(&full, &shown);
        let keep = shown.split('\n').take(200).collect::<Vec<_>>().join("\n");
        shown = format!(
            "{keep}\n[TSK: output truncated; {} bytes total. Full text: {}/{}.txt]\n",
            out_len,
            full.parent().unwrap_or(Path::new(".")).display(),
            h
        );
    }
    tsk_core::ledger::append(
        session,
        &tsk_core::ledger::Entry::Sandbox(tsk_core::ledger::SandboxEntry {
            kind: "sandbox",
            ts: tsk_core::now_millis(),
            session: session.to_string(),
            tool: "Bash".into(),
            orig_bytes: input_bytes,
            new_bytes: shown.len(),
            saved: input_bytes.saturating_sub(shown.len()),
        }),
    );
    tsk_core::inject::log_event(
        session,
        &format!("sandbox: {} -> {} bytes (script {})", input_bytes, shown.len(), script.display()),
    );
    Ok(shown)
}

fn check(label: &'static str, good: bool) -> (&'static str, bool) {
    (label, good)
}

fn has_tsk_hook(v: &Value) -> bool {
    serde_json::to_string(v)
        .map(|s| s.contains("tsk hook"))
        .unwrap_or(false)
}

/// Merge (add) or strip (remove) TSK's hook entries. Refuses to overwrite a
/// settings.json it cannot parse.
fn merge_hooks(project: &PathBuf, add: bool) -> Result<&'static str, String> {
    let p = project.join(".claude/settings.json");
    let mut obj: Value = if p.exists() {
        let s = std::fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        serde_json::from_str(&s).map_err(|e| format!("{} is not valid JSON, refusing to overwrite: {e}", p.display()))?
    } else {
        Value::Object(Map::new())
    };

    if add {
        add_tsk_hooks(&mut obj);
        write_hooks(&p, &obj)?;
        Ok("wired")
    } else {
        let n = strip_tsk_hooks(&mut obj);
        write_hooks(&p, &obj)?;
        Ok(if n > 0 { "removed" } else { "kept" })
    }
}

fn add_tsk_hooks(obj: &mut Value) {
    let Some(o) = obj.as_object_mut() else { return };
    let ho = o.entry("hooks").or_insert(Value::Object(Map::new()));
    let Some(ho) = ho.as_object_mut() else { return };
    let tsk = serde_json::json!({
        "PreToolUse": [{ "matcher": "Read", "hooks": [{ "type": "command", "command": "tsk hook PreToolUse" }] }],
        "PostToolUse": [{ "matcher": "Read", "hooks": [{ "type": "command", "command": "tsk hook PostToolUse" }] }],
        "SessionStart": [{ "hooks": [{ "type": "command", "command": "tsk hook SessionStart" }] }],
        "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "tsk hook UserPromptSubmit" }] }],
        "PreCompact": [{ "hooks": [{ "type": "command", "command": "tsk hook PreCompact" }] }],
    });
    if let Some(existing) = tsk.as_object() {
        for (ev, entries) in existing {
            let slot = ho.entry(ev.clone()).or_insert(Value::Array(Vec::new()));
            if let Some(arr) = slot.as_array_mut() {
                // 幂等：已有 TSK hook 条目则不重复追加。
                let already = arr
                    .iter()
                    .any(|e| serde_json::to_string(e).map(|s| s.contains("tsk hook")).unwrap_or(false));
                if !already {
                    arr.extend(entries.as_array().cloned().unwrap_or_default());
                }
            }
        }
    }
}

/// Returns the number of TSK hook entries removed.
fn strip_tsk_hooks(obj: &mut Value) -> usize {
    let Some(o) = obj.as_object_mut() else { return 0 };
    let Some(ho) = o.get_mut("hooks").and_then(|v| v.as_object_mut()) else { return 0 };
    let mut removed = 0usize;
    for (_ev, arr) in ho.iter_mut() {
        if let Some(list) = arr.as_array_mut() {
            let before = list.len();
            list.retain(|e| !serde_json::to_string(e).map(|s| s.contains("tsk hook")).unwrap_or(false));
            removed += before - list.len();
        }
    }
    ho.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false));
    if ho.is_empty() {
        o.remove("hooks");
    }
    removed
}

fn write_hooks(p: &PathBuf, o: &Value) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_string_pretty(o).unwrap_or_default()).map_err(|e| e.to_string())
}

fn saved_of(e: &tsk_core::ledger::Entry) -> usize {
    match e {
        tsk_core::ledger::Entry::Rewrite(x) => x.saved,
        tsk_core::ledger::Entry::Sandbox(x) => x.saved,
        tsk_core::ledger::Entry::Externalize(x) => x.orig_bytes.saturating_sub(x.new_bytes),
        _ => 0,
    }
}

fn tool_of(e: &tsk_core::ledger::Entry) -> Option<&str> {
    match e {
        tsk_core::ledger::Entry::Rewrite(x) => Some(&x.tool),
        tsk_core::ledger::Entry::Retrieval(x) => Some(&x.tool),
        tsk_core::ledger::Entry::Externalize(x) => Some(&x.tool),
        tsk_core::ledger::Entry::Sandbox(x) => Some(&x.tool),
        _ => None,
    }
}

/// Original bytes of files that were compressed (skeleton / externalize) — the
/// amount that would re-enter context if the model later read the full text.
fn orig_of(e: &tsk_core::ledger::Entry) -> usize {
    match e {
        tsk_core::ledger::Entry::Rewrite(x) => x.orig_bytes,
        tsk_core::ledger::Entry::Externalize(x) => x.orig_bytes,
        _ => 0,
    }
}

fn summarize(entries: &[tsk_core::ledger::Entry]) -> String {
    let mut total = 0usize;
    let mut cost = 0usize;
    let mut orig_full = 0usize; // 被压缩文件的原文总量（骨架/外置）
    let mut tools: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut retrievals = 0usize;
    for e in entries {
        total += saved_of(e);
        cost += cost_of(e);
        orig_full += orig_of(e);
        if let Some(t) = tool_of(e) {
            *tools.entry(t.to_string()).or_insert(0) += saved_of(e);
        }
        if matches!(e, tsk_core::ledger::Entry::Retrieval(_)) {
            retrievals += 1;
        }
    }
    let net = total.saturating_sub(cost);
    let mut s = format!(
        "saved       : {} bytes ≈ {} est tokens\n\
         inject cost : {} bytes ≈ {} est tokens\n\
         net         : {} bytes\n\
         retrievals  : {}\n",
        fmt_int(total),
        est_tok(total),
        fmt_int(cost),
        est_tok(cost),
        fmt_int(net),
        retrievals
    );
    let full_tok = est_tok(orig_full) as i64;
    let pess_net = est_tok(total) as i64 - full_tok - est_tok(cost) as i64;
    s.push_str(&format!(
        "压缩原文合计: {} bytes ≈ {} est tokens\n\
          （悲观上限：若全部回读原文全文, 额外输入 ≈ {} est tok; 净省 ≈ {} est tok）\n\
         —— 骨架/外置的节省以“不回读全文”为前提; 回读即付该额外输入\n",
        fmt_int(orig_full),
        est_tok(orig_full),
        full_tok,
        pess_net
    ));
    let maxk = tools.keys().map(|k| k.len()).max().unwrap_or(4);
    for (k, v) in &tools {
        let pct = if total > 0 { *v * 100 / total } else { 0 };
        s.push_str(&format!(
            "  {k:<pad$}: {val} bytes ({pct}%)\n",
            k = k,
            val = fmt_int(*v),
            pct = pct,
            pad = maxk
        ));
    }
    s
}

/// Thousands separator for readability: 199700 -> 199,700.
fn fmt_int(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// CI metric, iron law 6: cache reads / total input, from a transcript JSONL.
fn cache_hit(path: &PathBuf) -> String {
    let lines = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return format!("cache-hit: cannot read {}: {e}\n", path.display()),
    };
    let mut turns = 0u64;
    let mut reads = 0u64;
    let mut input = 0u64;
    for line in lines.lines() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(u) = v.pointer("/message/usage") else { continue };
        let i = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        let c = u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        turns += 1;
        reads += c;
        input += i;
    }
    if input == 0 {
        return format!("cache-hit: no usage data in {}\n", path.display());
    }
    format!("cache-hit: {}/{} turns, {:.1}% cache read\n", turns, turns, reads as f64 * 100.0 / input as f64)
}
