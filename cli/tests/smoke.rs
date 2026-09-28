//! 全功能冒烟（量化矩阵版）。每个测试声明「功能 → 参数 → 期望」，并在跑完
//! 打印实测的字节/token 收益（`cargo test --test smoke -- --nocapture` 可见）。
//!
//! 覆盖矩阵（功能 → 测试 → 参数 → 收益断言）：
//!   init 接线 / 骨架 / 逃逸口 / fail-open   → m0_wiring_skeleton_escape_failopen
//!   骨架无损（随机行取回）                  → fidelity_skeleton_random_lines
//!   外置无损（全文回读逐字节）              → fidelity_externalize_roundtrip
//!   中文/UTF-8 无损                         → fidelity_chinese_utf8
//!   小/二进制/缺失 passthrough + 大文件外置 → input_boundaries
//!   沙箱 env/解释器/统计任务                → sandbox_stats_and_env
//!   输出压缩注入（ruleset/强化/180k 幂等）   → output_injects
//!   report/doctor/off 幂等                  → report_doctor_off
//!   可靠性：init 两次幂等 / 并发 ledger      → reliability
//!
//! token 口径与 core 一致：est_tokens = ceil(bytes/4)。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

// ---------- helpers ----------

fn setup() -> (PathBuf, PathBuf) {
    let d = std::env::temp_dir().join(format!(
        "tsk-smoke-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let home = d.join("home");
    let project = d.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".tsk/skeletons")).unwrap();
    (home, project)
}

/// Windows 路径进 JSON 前转正斜杠（`:`+`\U` 是非法转义）。
fn p(s: &Path) -> String {
    s.to_string_lossy().replace('\\', "/")
}

fn run(home: &PathBuf, args: &[&str], stdin: &str, cwd: &Path) -> std::process::Output {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_tsk"))
        .args(args)
        .env("TSK_HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut s) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut s, stdin.as_bytes());
    }
    child.wait_with_output().unwrap()
}

fn read_cfg(home: &PathBuf, key: &str) -> Option<String> {
    let s = std::fs::read_to_string(home.join("config.yaml")).ok()?;
    s.lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.trim() == key)
                .and_then(|(_, v)| v.split('#').next())
                .map(|v| v.trim().to_string())
        })
}

fn write_cfg(home: &PathBuf, body: &str) {
    std::fs::write(home.join("config.yaml"), body).unwrap();
}

fn read_home(home: &PathBuf, session: &str, name: &str) -> String {
    std::fs::read_to_string(home.join(format!("{session}/{name}"))).unwrap_or_default()
}




fn parse_ctx(stdout: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(stdout).unwrap();
    v.pointer("/hookSpecificOutput/additionalContext")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string()
}

fn est_tok(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// 打印量化收益并断言不低于 `min_ratio`。返回 (saved_bytes, ratio)。
fn quant(feature: &str, params: &str, orig: usize, compressed: usize, min_ratio: f64) -> (usize, f64) {
    let saved = orig.saturating_sub(compressed);
    let ratio = if orig == 0 { 0.0 } else { saved as f64 / orig as f64 };
    eprintln!(
        "  [量化] {feature} | {params} | {orig}B(est {}) -> {compressed}B(est {}) | saved {saved}B | -{:.1}%",
        est_tok(orig),
        est_tok(compressed),
        ratio * 100.0
    );
    assert!(ratio >= min_ratio, "{feature}: 收益 {:.1}% 低于期望 {:.1}%", ratio * 100.0, min_ratio * 100.0);
    (saved, ratio)
}



// =====================================================================
// 1. 协议接线 + 骨架（量化收益）+ 逃逸口 + fail-open
// =====================================================================
#[test]
fn sandbox_stats_and_env() {
    eprintln!("== sandbox: env 过滤 / 解释器 / 统计任务 ==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-sx-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let input = sx.join("input.txt");
    std::fs::write(&input, b"line 1\nline 2\nline 3\n").unwrap();
    let script = |name: &str, body: &str| {
        let s = sx.join(name);
        std::fs::write(&s, body).unwrap();
        s
    };

    std::env::set_var("NODE_OPTIONS", "--require=/tmp/not-real.js");
    std::env::set_var("PYTHONSTARTUP", "/tmp/not-real.py");
    std::env::set_var("GIT_CONFIG_KEY_0", "hook.allow");
    let sh = script("probe.sh", "#!/bin/sh\nprintf 'N=%s\\n' \"$NODE_OPTIONS\"\nprintf 'P=%s\\n' \"$PYTHONSTARTUP\"\nprintf 'G=%s\\n' \"$GIT_CONFIG_KEY_0\"\n");
    let out = run(&home, &["exec", p(&sh).as_str(), p(&input).as_str()], "", &sx);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "N=\nP=\nG=\n", "env 未过滤");

    let py = script("probe.py", "import os\nprint('pyok NODE_OPTIONS=' + os.environ.get('NODE_OPTIONS',''))\n");
    let out = run(&home, &["exec", p(&py).as_str(), p(&input).as_str()], "", &sx);
    assert!(String::from_utf8_lossy(&out.stdout).trim_end().ends_with("NODE_OPTIONS="));

    let js = script("probe.js", "const fs=require('fs');\nconst n=fs.readFileSync(process.argv[1],'utf8').trim().split('\\n').length;\nconsole.log(n+' lines');\n");
    let out = run(&home, &["exec", p(&js).as_str(), p(&input).as_str()], "", &sx);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3 lines");

    if cfg!(windows) {
        let ps = script("probe.ps1", "Get-Content -LiteralPath $args[0] | Measure-Object -Line | Select-Object -ExpandProperty Lines\n");
        let out = run(&home, &["exec", p(&ps).as_str(), p(&input).as_str()], "", &sx);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3");
    }

    // 统计任务：500 行日志 → 只回 6 字节结论（CS-1 形态）。
    let log = sx.join("access.log");
    let log_body: String = (1..=500)
        .map(|i| {
            let code = if i % 100 == 0 { 500 } else if i % 7 == 0 { 404 } else { 200 };
            format!(r#"192.168.1.{i} - - [15/Sep/2026:10:00:01 +0000] "GET /api/u HTTP/1.1" {code} 5120 "ref" "UA" 0.0{i}"#)
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&log, &log_body).unwrap();
    let stats = script("stats.py", "import sys\nlines=open(sys.argv[1]).read().splitlines()\nerr=sum(1 for l in lines if ' 404 ' in l or ' 500 ' in l)\nprint(f'total={len(lines)} errors={err}')\n");
    let out = run(&home, &["exec", p(&stats).as_str(), p(&log).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.trim(), "total=500 errors=76", "统计结论");
    assert!(!s.contains("192.168."), "原始行不得进 context");
    let _ = quant("sandbox分析", "500行nginx日志 -> 6字节结论", log_body.len(), s.len(), 0.95);

    let led = read_home(&home, "cli", "ledger.jsonl");
    assert!(led.contains("\"kind\":\"sandbox\""), "sandbox ledger");

    std::env::remove_var("NODE_OPTIONS");
    std::env::remove_var("PYTHONSTARTUP");
    std::env::remove_var("GIT_CONFIG_KEY_0");
    for f in [sh, py, js] {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&log);
}

// =====================================================================
// 7. 输出压缩：ruleset / 每轮强化 / 180k 建议（幂等）
// =====================================================================
#[test]
fn output_injects() {
    eprintln!("== output: 输出压缩注入 ==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "s7";

    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#);
    let out = run(&home, &["hook", "SessionStart"], &ev, &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    for needle in [
        "TSK OUTPUT MODE ACTIVE", "HARD RULES", "VIOLATION CONTRACT",
        "POSITIVE example", "NEGATIVE example", "AUTO-CLARITY EXCEPTION", "STYLE SELF-CHECK",
    ] {
        assert!(ctx.contains(needle), "ruleset 缺 {needle}");
    }
    // 注入成本可见：ruleset 必须 ≥3500 字符（S3 稀释阈值），est tokens 打印。
    eprintln!("  [量化] SessionStart 注入成本: {} 字符 ≈ est {} tok（输出压缩的输入侧成本）", ctx.chars().count(), est_tok(ctx.len()));
    assert!(ctx.chars().count() >= 3500);

    let ev = format!(r#"{{"session_id":"{sid}","prompt":"why","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);
    let out = run(&home, &["hook", "UserPromptSubmit"], &ev, &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(ctx.contains("Enforce this reply"), "每轮强化注入：{ctx}");

    // 180k 建议：>180k 注入一次，幂等（第二次不重复）。
    let t = project.join("transcript.jsonl");
    std::fs::write(
        &t,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":200000,"cache_read_input_tokens":10000}}}
"#,
    )
    .unwrap();
    let ev = format!(r#"{{"session_id":"{sid}","prompt":"again","cwd":"/","transcript_path":"{}","hook_event_name":"UserPromptSubmit"}}"#, p(&t));
    let out = run(&home, &["hook", "UserPromptSubmit"], &ev, &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(ctx.contains("/compact"), "180k 建议缺：{ctx}");
    let out = run(&home, &["hook", "UserPromptSubmit"], &ev, &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(!ctx.contains("/compact"), "重复注入：{ctx}");
    let log = read_home(&home, sid, "events.log");
    assert_eq!(log.matches("compact_advice").count(), 1, "events.log 只有一行建议");

    // off → 不注入。
    write_cfg(&home, "enabled: true\noutput_compression: off\n");
    let out = run(&home, &["hook", "SessionStart"], &format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
}

// =====================================================================
// 8. report / doctor / off 幂等
// =====================================================================
#[test]
fn report_doctor_off() {
    eprintln!("== report / doctor / off ==");
    let (home, project) = setup();
    let _ = run(&home, &["init"], "", &project);

    let sdir = home.join("sess-r");
    std::fs::create_dir_all(&sdir).unwrap();
    std::fs::write(
        sdir.join("ledger.jsonl"),
        r#"{"kind":"rewrite","ts":1,"session":"sess-r","tool":"Read","tu_id":"t1","strategy":"skeleton","orig_bytes":1000,"new_bytes":100,"saved":900}
{"kind":"retrieval","ts":2,"session":"sess-r","tool":"Read","reason":"escape_hatch_offset_limit"}
{"kind":"externalize","ts":3,"session":"sess-r","tool":"Read","orig_bytes":200000,"new_bytes":1200}
"#,
    )
    .unwrap();
    let out = run(&home, &["report", "--session", "sess-r"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    eprintln!("  [量化] report: {}（saved=rewrite 900 + externalize 198800）", s.trim());
    assert!((s.contains("saved") && s.contains("199,700")));
    assert!((s.contains("retrievals") && s.contains("1")));
    assert!((s.contains("Read") && s.contains("199,700")));

    let out = run(&home, &["doctor"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("settings.json wired"));
    assert!(s.contains("[ok]"));

    let out = run(&home, &["off"], "", &project);
    assert!(String::from_utf8_lossy(&out.stdout).contains("config disabled"));
    assert_eq!(read_cfg(&home, "enabled").as_deref(), Some("false"));
    // Bug E3 回归：`off` 只翻转 enabled 行，不得丢弃 init 写入的档位注释。
    let disabled_cfg = std::fs::read_to_string(home.join("config.yaml")).unwrap();
    assert!(disabled_cfg.contains("output_compression:"), "off 后仍保留档位键");
    assert!(disabled_cfg.contains("lossless"), "off 后保留档位注释（无损说明）");
    let settings = std::fs::read_to_string(project.join(".claude/settings.json")).unwrap();
    assert!(!settings.contains("tsk hook"), "off 后 hooks 摘除");
    let out = run(&home, &["off"], "", &project);
    assert_eq!(out.status.code(), Some(0), "off 幂等");
}

// =====================================================================
// 9. 可靠性：init 两次幂等（不重复 hook）+ 并发 ledger 不坏
// =====================================================================
#[test]
fn bash_passthrough_and_exec() {
    eprintln!("== bash: 事件恒 passthrough + tsk exec 命令收益 ==");
    let (home, project) = setup();

    // 澄清：v1 PreToolUse 只 matcher Read（spec §3），Bash 事件恒不改写——
    // 这是防二次权限弹窗（S1）的设计决定，不是遗漏。任何 Bash 命令都返回 {}。
    for cmd in [
        "ls -la",
        "cat bigfile.txt",
        "echo hello | grep -c e",
        "node -e \"console.log(1)\"",
    ] {
        let stdin = format!(
            r#"{{"session_id":"b1","cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#,
            p(&project),
            cmd.replace('"', "\\\"")
        );
        let out = run(&home, &["hook", "PreToolUse"], &stdin, &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "Bash 事件 {cmd} 恒 passthrough");
    }
    // 复杂命令（管道/内联解释器）也不被改写、不触发任何压缩路径。
    let out = run(&home, &["hook", "PreToolUse"], &format!(
        r#"{{"session_id":"b1","cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"wc -l file | tail -1 && echo done"}}}}"#,
        p(&project)
    ), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");

    // 沙箱执行 bash 命令（含管道）的收益：读 500 行日志只回 2 行结论。
    let sx = std::env::temp_dir().join(format!("tsk-bx-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let log = sx.join("access.log");
    let body: String = (1..=500)
        .map(|i| format!("192.168.0.{i} GET /api/{i} status {}", if i % 50 == 0 { 500 } else { 200 }))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&log, &body).unwrap();
    let sh = sx.join("sum.sh");
    std::fs::write(
        &sh,
        "#!/bin/sh\ngrep -c 'status 500' \"$1\"\ngrep -c 'GET /api/' \"$1\"\n",
    )
    .unwrap();
    let out = run(&home, &["exec", p(&sh).as_str(), p(&log).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.trim(), "10\n500", "bash 管道统计结论");
    let _ = quant("bash沙箱命令", "500行日志->2行结论", body.len(), s.len(), 0.95);
    for f in [sh, log] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 12. 沙箱输出三档：小 / 中 / 大（>64KB 截断 + 外置）
// =====================================================================
#[test]
fn sandbox_output_sizes() {
    eprintln!("== sandbox: 输出三档（小/中/大>64KB 截断外置）==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-oy-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let input = sx.join("in.txt");
    std::fs::write(&input, "x\n").unwrap();

    // 小输出：原样透传。
    let small = sx.join("small.sh");
    std::fs::write(&small, "#!/bin/sh\necho 3 lines\n").unwrap();
    let out = run(&home, &["exec", p(&small).as_str(), p(&input).as_str()], "", &sx);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3 lines");

    // 中输出（几百字节）：原样。
    let med = sx.join("med.py");
    std::fs::write(&med, "print('\\n'.join(f'r{i}' for i in range(100)))\n").unwrap();
    let out = run(&home, &["exec", p(&med).as_str(), p(&input).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.lines().count(), 100, "中输出原样");

    // 大输出（>64KB）：截断 + 全文外置 + ledger 记 saved。
    let big = sx.join("big.py");
    std::fs::write(&big, "print('\\n'.join(f'row {i} with padding to exceed 64k' for i in range(5000)))\n").unwrap();
    let out = run(&home, &["exec", p(&big).as_str(), p(&input).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("TSK: output truncated"), "大输出须提示截断，前 200 字符: {}", &s[..200.min(s.len())]);
    // 全文已外置。
    let ext = home.join("cli/ext");
    let fulls: Vec<_> = std::fs::read_dir(&ext).unwrap().flatten().filter(|e| !e.file_name().to_string_lossy().contains(".pointer")).collect();
    assert_eq!(fulls.len(), 1, "ext/ 一个全文文件");
    let full_len = std::fs::metadata(fulls[0].path()).unwrap().len();
    assert!(full_len > 64 * 1024, "全文外置完整: {full_len}");
    let _ = quant("sandbox大输出", "100KB stdout", full_len as usize, s.len(), 0.9);
    for f in [small, med, big, input] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 13. 输出压缩三档注入：ruleset / 每轮强化 / resume 快照
// =====================================================================
#[test]
fn output_compression_lengths() {
    eprintln!("== output: 三档注入（ruleset / reinforce / resume 快照）==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "o3";

    // 档 1：SessionStart 全量 ruleset（≥3500 字符）。
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#);
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &ev, &project).stdout));
    assert!(ctx.chars().count() >= 3500);
    eprintln!("  [量化] 注入-1 ruleset: {} 字符 ≈ est {} tok", ctx.chars().count(), est_tok(ctx.len()));

    // 档 2：UserPromptSubmit 每轮强化（~200 字符）。
    let ev = format!(r#"{{"session_id":"{sid}","prompt":"q","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "UserPromptSubmit"], &ev, &project).stdout));
    assert!(ctx.contains("Enforce this reply"));
    eprintln!("  [量化] 注入-2 reinforce: {} 字符 ≈ est {} tok", ctx.chars().count(), est_tok(ctx.len()));

    // 档 3：SessionStart(source=resume) + 快照 → RESUME 组装注入（≤500 tok 预算）。
    let t = project.join("transcript.jsonl");
    std::fs::write(
        &t,
        r#"{"type":"user","message":{"content":"refactor the database module and add tests for it"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Decision: use SQLite with WAL mode for the store."}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/db.rs"}}]}}
{"type":"user","message":{"content":"now verify the migration path"}}
"#,
    )
    .unwrap();
    let pcev = format!(r#"{{"session_id":"{sid}","cwd":"/","transcript_path":"{}","trigger":"manual","hook_event_name":"PreCompact"}}"#, p(&t));
    let _ = run(&home, &["hook", "PreCompact"], &pcev, &project);
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"resume","hook_event_name":"SessionStart"}}"#);
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &ev, &project).stdout));
    assert!(ctx.contains("RESUME:"), "resume 须注入快照组装");
    assert!(ctx.contains("ROLE:"), "快照组装含 P1 role");
    let resume_part = ctx.split("RESUME:").nth(1).unwrap_or("");
    assert!(est_tok(resume_part.len()) <= 500, "RESUME 注入 ≤500 tok 预算，实为 est {}", est_tok(resume_part.len()));
    eprintln!("  [量化] 注入-3 resume 快照: {} 字符 ≈ est {} tok（预算 500）", resume_part.chars().count(), est_tok(resume_part.len()));
}

// =====================================================================
// 14. 逃逸口矩阵：offset/limit 的各种形态恒 passthrough + 记账
// =====================================================================
#[test]
fn output_compression_net_gain() {
    eprintln!("== output: 整体收益（注入开销 vs 输出节省）==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "og";

    // 实测注入开销：SessionStart ruleset + 每轮 reinforce。
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#);
    let ruleset_tok = est_tok(parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &ev, &project).stdout)).len());
    let ev = format!(r#"{{"session_id":"{sid}","prompt":"q","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);
    let reinforce_tok = est_tok(parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "UserPromptSubmit"], &ev, &project).stdout)).len());
    eprintln!("  注入开销: ruleset est {ruleset_tok} tok + 每轮 reinforce est {reinforce_tok} tok");

    // 输出节省：真实 claude A/B 实测（同一 prompt "explain closures"，TSK off vs full）：
    // off 输出 814 tok / full 254 tok → 每轮省 560 tok。见 README「输出压缩收益」节。
    let out_saving_per_turn = 814usize - 254;

    for n in [1usize, 2, 3, 5, 10] {
        let inject_cost = ruleset_tok + n * reinforce_tok;
        let saved_out = n * out_saving_per_turn;
        let net = saved_out as isize - inject_cost as isize;
        eprintln!(
            "  [量化] {n} 轮: 注入 {inject_cost} tok vs 输出节省 {saved_out} tok → 整体 {net:+} tok（{}）",
            if net >= 0 { "净赚" } else { "净亏 → 短会话应关闭输出压缩" }
        );
    }
    // break-even：560N > 967 + 88N → N > 967/472 ≈ 2.05 → 3 轮起净赚。
    let break_even = ruleset_tok.div_ceil(out_saving_per_turn.saturating_sub(reinforce_tok));
    eprintln!("  [量化] break-even ≈ {break_even} 轮：≤2 轮整体净亏（收益低→关闭输出压缩），≥{break_even} 轮净赚");
    assert!(
        ruleset_tok + reinforce_tok > out_saving_per_turn,
        "单轮（ruleset+1×reinforce）必须净亏——如实量化短会话收益为负"
    );
    assert!(
        ruleset_tok + 10 * reinforce_tok < 10 * out_saving_per_turn,
        "10 轮必须净赚"
    );
}

// =====================================================================
// 19. 输出压缩 auto 档：短会话不注入（省固定成本），≥3 轮起注入
// =====================================================================
#[test]
fn output_compression_auto() {
    eprintln!("== output: auto 档（break-even ≈3 轮，短会话省固定成本）==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: auto\n");
    let sid = "au";

    // SessionStart 在 auto 下不注入 ruleset（省 967 tok 固定成本）。
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#);
    let out = run(&home, &["hook", "SessionStart"], &ev, &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "auto: SessionStart 不注入");

    let turn = |home: &PathBuf| {
        let ev = format!(r#"{{"session_id":"{sid}","prompt":"q","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);
        String::from_utf8_lossy(&run(home, &["hook", "UserPromptSubmit"], &ev, &project).stdout).into_owned()
    };
    // 第 1、2 轮：不注入（短会话零成本）。
    for i in 1..=2 {
        assert_eq!(turn(&home).trim(), "{}", "auto 第 {i} 轮不注入");
    }
    // 第 3 轮起：注入 ruleset + reinforce。
    let ctx = parse_ctx(&turn(&home));
    assert!(ctx.contains("TSK OUTPUT MODE ACTIVE"), "auto 第 3 轮注入 ruleset");
    assert!(ctx.contains("Enforce this reply"), "auto 第 3 轮注入 reinforce");
    // 第 4 轮：仅 reinforce（ruleset 只注入一次）。
    let ctx = parse_ctx(&turn(&home));
    assert!(!ctx.contains("HARD RULES"), "ruleset 只注入一次");
    assert!(ctx.contains("Enforce this reply"), "第 4 轮 reinforce");

    let led = read_home(&home, sid, "ledger.jsonl");
    let injects = led.matches("\"kind\":\"inject\"").count();
    eprintln!("  [量化] auto 4 轮: 注入记账 {injects} 行（1 ruleset + 2 reinforce）；前 2 轮零注入成本（full 档 2 轮净亏 -23 tok）");
    assert_eq!(injects, 3, "auto 4 轮应只有 1 ruleset + 2 reinforce 记账");
}

// =====================================================================
// 20. PostToolUse 观测口：恒 passthrough + events.log 记录
// =====================================================================
#[test]
fn sandbox_examples() {
    eprintln!("== sandbox: 示例脚本 analyze_log.py 可用 ==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-ex-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let log = sx.join("access.log");
    let body: String = (1..=500)
        .map(|i| {
            let code = if i % 100 == 0 { 500 } else if i % 7 == 0 { 404 } else { 200 };
            let lat = if i % 50 == 0 { 2.5 } else { 0.0 + (i % 10) as f64 / 100.0 };
            format!(r#"192.168.1.{i} - - [15/Sep/2026:10:00:01 +0000] "GET /api/u HTTP/1.1" {code} 5120 "ref" "UA" {lat}"#)
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&log, &body).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("examples/sandbox/analyze_log.py");
    assert!(script.exists(), "示例脚本存在: {}", script.display());
    let out = run(&home, &["exec", p(&script).as_str(), p(&log).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Total requests: 500"), "示例输出:\n{s}");
    assert!(s.contains("Slow requests (>1s): 10"), "慢请求 10 个（50 的倍数）:\n{s}");
    assert!(!s.contains("192.168."), "原始行不进 context");
    let _ = std::fs::remove_file(&log);
}

// =====================================================================
// 22. P-1 沙箱自动路由：analyze_<ext> 脚本存在才沙箱，缺脚本回退骨架
// =====================================================================
#[test]
fn output_compression_lite() {
    eprintln!("== output: lite 档（短 ruleset 省输入）==");
    let (home, project) = setup();
    let ev = r#"{"session_id":"li","cwd":"/","source":"startup","hook_event_name":"SessionStart"}"#;
    write_cfg(&home, "enabled: true\noutput_compression: lite\n");
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], ev, &project).stdout));
    assert!(ctx.contains("(level: lite)"), "lite 标记");
    assert!(ctx.contains("pleasantries"), "lite 核心规则");
    assert!(ctx.contains("AUTO-CLARITY EXCEPTION"), "lite 保留安全例外");
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let full = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], ev, &project).stdout));
    assert!(ctx.len() < full.len(), "lite({}) 须短于 full({})", ctx.len(), full.len());
    eprintln!("  [量化] lite {} 字符 est {} tok vs full {} 字符 est {} tok（每次会话省 {} tok 注入）",
        ctx.len(), est_tok(ctx.len()), full.len(), est_tok(full.len()), est_tok(full.len()).saturating_sub(est_tok(ctx.len())));
}

// =====================================================================
// 24. 沙箱网络隔离：TSK_NET_ISOLATE=1 不破坏现有执行
//     （Linux 用 unshare -n 需 CAP_SYS_ADMIN；Windows 无 seccomp，no-op）
// =====================================================================
#[test]
fn sandbox_net_isolate() {
    eprintln!("== sandbox: 网络隔离开关不破坏执行（平台分派）==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-net-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let input = sx.join("in.txt");
    std::fs::write(&input, "x\n").unwrap();
    let sh = sx.join("probe.sh");
    std::fs::write(&sh, "#!/bin/sh\necho net-probe-ok\n").unwrap();
    // 设置隔离开关（Linux 生效、Windows no-op），断言沙箱照常执行。
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_tsk"))
        .args(["exec", p(&sh).as_str(), p(&input).as_str()])
        .env("TSK_HOME", &home)
        .env("TSK_NET_ISOLATE", "1")
        .current_dir(&sx)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut s) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut s, b"");
    }
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("net-probe-ok"), "隔离开关下沙箱照常执行: stdout={s:?} stderr={:?}", String::from_utf8_lossy(&out.stderr));
    for f in [sh, input] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 25. cache-hit 汇总计算（--cache-hit 多轮聚合，供 CI 基线校准）
// =====================================================================
#[test]
fn cache_hit_summary() {
    eprintln!("== cache: 多轮 usage 汇总（--cache-hit）==");
    let (home, project) = setup();
    let t = project.join("t.jsonl");
    let mut s = String::new();
    for i in 0..4 {
        let read = if i % 2 == 0 { 400 } else { 100 };
        s += &format!(r#"{{"type":"assistant","message":{{"usage":{{"input_tokens":1000,"cache_read_input_tokens":{read}}}}}}}"#);
        s += "\n";
    }
    std::fs::write(&t, s).unwrap();
    let out = run(&home, &["report", "--cache-hit", p(&t).as_str()], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("4 turns"), "轮数");
    assert!(s.contains("25.0%"), "cache read 比例应 25.0%:\n{s}");
    eprintln!("  [量化] cache-hit: {}（真实会话在此校准 CI 阈值）", s.trim());
}

// =====================================================================
// 26. /compact 闭环：compact 前 transcript → PreCompact 快照 → resume 注入
// =====================================================================
#[test]
fn compact_roundtrip() {
    eprintln!("== compact: 沙盒闭环（PreCompact 快照 → resume 注入）==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "cp";

    // 模拟一个真实多轮会话的 transcript（compact 前一刻的状态）。
    let t = project.join("transcript.jsonl");
    std::fs::write(
        &t,
        r#"{"type":"user","message":{"content":"refactor the database module and add tests"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Decision: use SQLite with WAL mode.\nLet me look at the schema."}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/db.rs"}}]}}
{"type":"user","message":{"content":"also add a migration"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"we will use a versioned migration table"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/migrations/mod.rs"}}]}}
{"type":"user","message":{"content":"now verify the migration path"}}
"#,
    )
    .unwrap();

    // ① compact 时刻：PreCompact hook 被调用 → 快照落盘 + 锚点保护。
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","transcript_path":"{}","trigger":"manual","hook_event_name":"PreCompact"}}"#, p(&t));
    let out = run(&home, &["hook", "PreCompact"], &ev, &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(ctx.contains("[TSK SKELETON of"), "锚点保护");
    assert!(ctx.contains("do not summarize or drop"), "锚点保护明细");
    let snap_path = home.join(format!("{sid}/resume-snapshot.json"));
    assert!(snap_path.exists(), "快照落盘");
    let snap: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&snap_path).unwrap()).unwrap();
    let snap_bytes = std::fs::metadata(&snap_path).unwrap().len() as usize;
    assert!(snap_bytes < 2 * 1024, "快照 <2KB: {snap_bytes}B");
    assert!(snap["role"].as_str().unwrap().contains("refactor the database module"));
    assert!(snap["decisions"].as_array().unwrap().len() >= 2, "提取 ≥2 条决策");
    assert!(snap["skills"].as_array().unwrap().iter().any(|s| s == "rust"), "扩展名推断技能 rust");
    assert!(snap["intent"].as_str().unwrap().contains("verify the migration"));

    // ② compact 后：SessionStart(source=resume) → RESUME 快照组装注入（≤500 tok）。
    let ev = format!(r#"{{"session_id":"{sid}","cwd":"/","source":"resume","hook_event_name":"SessionStart"}}"#);
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &ev, &project).stdout));
    assert!(ctx.contains("RESUME:"), "resume 注入");
    assert!(ctx.contains("ROLE:"), "P1 role");
    assert!(ctx.contains("DECISIONS:"), "P2 decisions");
    let resume_part = ctx.split("RESUME:").nth(1).unwrap_or("");
    let resume_tok = est_tok(resume_part.len());
    assert!(resume_tok <= 500, "resume 注入 ≤500 tok 预算: {resume_tok}");
    // 无损：decisions/skills 原文保留。
    assert!(resume_part.contains("SQLite with WAL"), "决策原文");
    assert!(resume_part.contains("rust"), "技能词");
    eprintln!("  [量化] compact 闭环: 快照 {snap_bytes}B(<2KB) | resume 注入 est {resume_tok} tok(≤500) | 锚点+快照+resume 全链路通过");
}


// =====================================================================
// 27. 沙箱超时杀进程树（10s 硬顶）
// =====================================================================
#[test]
fn sandbox_timeout_kills() {
    eprintln!("== sandbox: 超时杀进程树（10s 硬顶）==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-tmo-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let input = sx.join("in.txt");
    std::fs::write(&input, "x\n").unwrap();
    // 用原生进程（python）而非 msys bash：Windows 的 taskkill /T 对原生进程树可靠，
    // 对 msys fork 模拟的进程树不可靠（实测 30s 完整跑完）。
    let sh = sx.join("slow.py");
    std::fs::write(&sh, "import time\ntime.sleep(30)\n").unwrap();
    let start = std::time::Instant::now();
    let out = run(&home, &["exec", p(&sh).as_str(), p(&input).as_str()], "", &sx);
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs() < 20, "超时应在 ~10s 杀进程树，实为 {elapsed:?}");
    // cli 对空输出包装成 `{}`（fail-open），沙箱脚本自身无输出。
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "被杀进程无脚本输出");
    eprintln!("  [量化] sandbox超时: 10s 硬顶生效，{elapsed:?} 内杀原生进程树");
    for f in [sh, input] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 28. 沙箱 stderr 隔离：只 stdout 进 context
// =====================================================================
#[test]
fn sandbox_stderr_isolated() {
    eprintln!("== sandbox: stderr 隔离（信息不出 context）==");
    let (home, _project) = setup();
    let sx = std::env::temp_dir().join(format!("tsk-err-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let input = sx.join("in.txt");
    std::fs::write(&input, "x\n").unwrap();
    let sh = sx.join("mix.sh");
    std::fs::write(&sh, "#!/bin/sh\necho STDOUT-LINE\necho STDERR-LINE 1>&2\n").unwrap();
    let out = run(&home, &["exec", p(&sh).as_str(), p(&input).as_str()], "", &sx);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("STDOUT-LINE"), "stdout 进 context: {s}");
    assert!(!s.contains("STDERR-LINE"), "stderr 不得进 context: {s}");
    eprintln!("  [量化] stderr 隔离: 只有 stdout({}B) 进 context", s.len());
    for f in [sh, input] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 29. 骨架边界：CRLF / 空行 / 超长行（含收益）
// =====================================================================
#[test]
fn auto_resume_interaction() {
    eprintln!("== resume: 独立于输出压缩档位（auto/off 档也应注入快照）==");
    let (home, project) = setup();
    let compact = |home: &PathBuf, sid: &str| {
        let t = project.join(sid);
        // 每行一个 JSON（真实 transcript 是 JSONL，多行；挤在同一行会导致 build 只解析第一条）。
        std::fs::write(&t, "{\"type\":\"user\",\"message\":{\"content\":\"refactor the database module\"}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Decision: use WAL\"}]}}\n").unwrap();
        let _ = run(home, &["hook", "PreCompact"], &format!(r#"{{"session_id":"{sid}","cwd":"/","transcript_path":"{}","trigger":"manual","hook_event_name":"PreCompact"}}"#, p(&t)), &project);
        assert!(home.join(format!("{sid}/resume-snapshot.json")).exists(), "快照落盘");
        let _ = t;
    };

    // auto 档：resume 注入快照 RESUME，但不注入 ruleset（ruleset 由轮数决定）。
    write_cfg(&home, "enabled: true\noutput_compression: auto\n");
    compact(&home, "ar-auto");
    let out = run(&home, &["hook", "SessionStart"], &format!(r#"{{"session_id":"ar-auto","cwd":"/","source":"resume","hook_event_name":"SessionStart"}}"#), &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(ctx.contains("RESUME:"), "auto 档 resume 须注入快照（历史线无损，独立于输出档位）");
    assert!(!ctx.contains("TSK OUTPUT MODE ACTIVE"), "auto 档不注入 ruleset（由轮数决定）");

    // off 档（默认！）：resume 仍须注入——这是 Bug A 的回归点（原实现开箱即用时
    // resume 从未工作）。仅受 enabled 约束。
    write_cfg(&home, "enabled: true\noutput_compression: off\n");
    compact(&home, "ar-off");
    let out = run(&home, &["hook", "SessionStart"], &format!(r#"{{"session_id":"ar-off","cwd":"/","source":"resume","hook_event_name":"SessionStart"}}"#), &project);
    let ctx = parse_ctx(&String::from_utf8_lossy(&out.stdout));
    assert!(ctx.contains("RESUME:"), "off 档 resume 须注入快照（输出压缩关闭也不应影响历史恢复）");
    assert!(ctx.contains("use WAL"), "决策原文保留（该会话决策为 'Decision: use WAL'）");

    eprintln!("  [量化] resume: auto 档注入 RESUME 且省 ruleset 成本；off 档仍注入（历史线无损独立）");
}

// ---------- 新增功能冒烟：search / analyze (Think-in-Code) ----------

/// `tsk search <q>`：对落盘 ext 全文按相关性返回「精确命中行窗」，原文保留。
#[test]
fn smoke_search_returns_exact_snippet() {
    let (home, project) = setup();
    let ext = home.join("s1/ext");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("calc.txt"), "function onNumberInput(d){return d}\nconst CODE = 200 OK\nERROR 404 file not found\nvar calc = 3\n收益 error handling 示例\n").unwrap();

    let out = run(&home, &["search", "error"], "", &project);
    assert_eq!(out.status.code(), Some(0), "search should exit 0; stderr={}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("≪ERROR≫ 404 file not found"), "snippet must keep exact hit line + highlight, got: {s}");
    assert!(s.contains("calc"), "should include neighbors, got: {s}");
}

/// `tsk analyze <file>`：Think-in-Code —— 沙箱/内置摘要把计算结果送上下文，不读全文。
#[test]
fn smoke_analyze_computes_summary() {
    let (home, project) = setup();
    let md = project.join("spec.md");
    std::fs::write(&md, "# 测试方案\n\n## 用户故事1\n做计算器，四则运算。\n- 优先级 P1\n\n## 约束\n- 无网络、无存储\n").unwrap();

    let out = run(&home, &["analyze", &p(&md)], "", &project);
    assert_eq!(out.status.code(), Some(0), "analyze should exit 0; stderr={}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("测试方案"), "should keep title, got: {s}");
    assert!(s.contains("用户故事1"), "should keep section heading, got: {s}");
}
