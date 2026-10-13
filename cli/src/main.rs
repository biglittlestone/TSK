//! TSK command-line entry. One catch point: any failure inside a hook prints a
//! note to stderr and still answers `{}` with exit 0 (fail-open, spec §10).
//! Business logic lives in `tsk-core`; this file only orchestrates stdio.

use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tsk_core::{Config, events, storage};

mod report;

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
    /// 按命令类型压缩一个工具输出（rtk 式）。raw 走 stdin，输出紧凑文本；未识别则空。
    Compress {
        /// 原始命令（用于决定压缩策略），如 "git status --short"
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Search TSK's stored/externalized content (Think-in-Code 辅助). Returns exact-hit snippets.
    Search {
        /// Query terms (joined by space).
        #[arg(trailing_var_arg = true)]
        query: Vec<String>,
        /// Restrict to one session id (default: all sessions).
        #[arg(long)]
        session: Option<String>,
        /// Max hits (default 5).
        #[arg(long, default_value_t = 5)]
        top: usize,
    },
    /// Analyze a file in the sandbox (Think-in-Code): run `<lang> script`/inline summarizer over `file`,
    /// print only stdout (the computed result) into context.
    Analyze {
        file: PathBuf,
        /// Analysis script path; if omitted, use a built-in summary.
        #[arg(short, long)]
        script: Option<PathBuf>,
        #[arg(long, default_value = "cli")]
        session: String,
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
                Some(events::Event::SessionStart) => tsk_core::inject::session_start(&cfg, &ev).to_string(),
                Some(events::Event::UserPromptSubmit) => tsk_core::inject::user_prompt(&cfg, &ev).to_string(),
                Some(events::Event::PreCompact) => tsk_core::inject::precompact(&cfg, &ev).to_string(),
                None => return Err(format!("unknown event `{event}`")),
            })
        }
        Cmd::Init { project } => Ok(init(&project).to_string()),
        Cmd::Off { project } => Ok(off(&project).to_string()),
        Cmd::Report { session, all, clean, cache_hit } => Ok(report::report(session, all, clean, cache_hit).to_string()),
        Cmd::Doctor { project } => Ok(doctor(&project).to_string()),
        Cmd::Exec { script, input, session, args } => Ok(exec(&script, &input, &session, &args)?),
        Cmd::Search { query, session, top } => Ok(search_cmd(&query.join(" "), session.as_deref(), top)),
        Cmd::Analyze { file, script, session } => Ok(analyze_cmd(&file, script.as_deref(), &session)?),
        Cmd::Compress { command } => {
            let cmd = command.join(" ");
            let raw = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
            Ok(tsk_core::compact::compact_by_command(&cmd, &raw).unwrap_or_default())
        }
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

fn search_cmd(query: &str, session: Option<&str>, top: usize) -> String {
    use tsk_core::search;
    let hits = search::search(query, top, session);
    if hits.is_empty() {
        return "tsk search: no hits.\n".to_string();
    }
    let mut out = format!("[tsk search] {query} — {} hit(s):\n", hits.len());
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("#{} [{:.2}]\n{}\n({} B)\n--- exact snippet ---\n{}\n\n", i + 1, h.score as f64 / 100.0, h.path.display(), h.bytes, h.snippet.trim()));
    }
    out
}

/// `tsk analyze <file>`：Think-in-Code —— 在沙箱里跑分析脚本（或缺省内置摘要），只输出 stdout。
/// 内置摘要：对文本文件取标题 + ## 标题 + 每节首要点（与 deveco 内联摘要一致），cap 1.2KB。
fn analyze_cmd(file: &std::path::Path, script: Option<&std::path::Path>, session: &str) -> Result<String, String> {
    let file = file.canonicalize().map_err(|e| format!("cannot resolve {file:?}: {e}"))?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    if let Some(sc) = script {
        let run = tsk_core::sandbox::run(sc, &[], &file).map_err(|e| format!("sandbox failed: {e}"))?;
        return Ok(if run.stdout.trim().is_empty() { "(sandbox produced no stdout)".to_string() } else { run.stdout });
    }
    // 内置摘要
    let mut title = "";
    let mut sections: Vec<String> = Vec::new();
    let mut cur = "";
    let mut size = 0usize; let cap = 1200usize;
    for l in text.lines() {
        if l.starts_with("# ") && title.is_empty() { title = l[2..].trim(); continue; }
        if l.starts_with("##") { cur = l.trim_start_matches('#').trim(); sections.push(format!("\n### {cur}")); size += cur.len(); continue; }
        if !cur.is_empty() && (l.trim().starts_with("-") || l.trim().starts_with("*") || l.trim().starts_with(char::is_numeric)) {
            sections.push(format!("  · {}", l.trim().chars().take(100).collect::<String>())); size += 104;
            // numbers: 用首字符数字也行，简化：只取 dash/* 开头
            let _ = size;
        }
        if size > cap { break; }
    }
    let _ = session;
    Ok(format!("[tsk analyze of {}]\n# {}\n{}\n[原文: {} 字节]\n", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), title, sections.join("\n"), text.len()))
}
