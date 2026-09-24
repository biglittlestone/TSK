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

fn pretool_ev(session: &str, cwd: &Path, tool_input: &str) -> String {
    format!(
        r#"{{"session_id":"{session}","cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{tool_input},"tool_use_id":"tu-{session}"}}"#,
        p(cwd)
    )
}

fn read_input(fp: &Path) -> String {
    format!(r#"{{"file_path":"{}"}}"#, p(fp))
}

fn file_lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
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

/// 断言 hook 输出为改写到给定路径、该路径存在且可读。
fn assert_rewrite_to(stdout: &str, expect: &str) -> PathBuf {
    let v: serde_json::Value = serde_json::from_str(stdout).unwrap();
    let updated = v.pointer("/hookSpecificOutput/updatedInput").unwrap();
    assert_eq!(updated.as_object().unwrap().keys().count(), 1, "只改 file_path");
    let path = PathBuf::from(updated["file_path"].as_str().unwrap());
    assert!(path.exists(), "改写的路径存在: {}", path.display());
    if !expect.is_empty() {
        assert_eq!(updated["file_path"].as_str().unwrap(), expect, "改写的路径");
    }
    path
}

/// 生成骨架并返回 (stdout, 骨架文件, 原文件)。
fn make_skeleton(home: &PathBuf, project: &Path, session: &str, content: &str) -> (String, PathBuf, PathBuf) {
    let file = project.join(format!("{session}.txt"));
    std::fs::write(&file, content).unwrap();
    let out = run(home, &["hook", "PreToolUse"], &pretool_ev(session, project, &read_input(&file)), project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("hookSpecificOutput"), "stdout: {stdout} stderr: {}", String::from_utf8_lossy(&out.stderr));
    let sk = assert_rewrite_to(&stdout, "");
    (stdout, sk, file)
}

// =====================================================================
// 1. 协议接线 + 骨架（量化收益）+ 逃逸口 + fail-open
// =====================================================================
#[test]
fn m0_wiring_skeleton_escape_failopen() {
    eprintln!("== m0: init 接线 / 骨架 / 逃逸口 / fail-open ==");
    let (home, project) = setup();

    let out = run(&home, &["init"], "", &project);
    assert_eq!(out.status.code(), Some(0));
    let settings = std::fs::read_to_string(project.join(".claude/settings.json")).unwrap();
    for ev in ["PreToolUse", "PostToolUse", "SessionStart", "UserPromptSubmit", "PreCompact"] {
        assert!(settings.contains(&format!("tsk hook {ev}")), "wiring {ev}");
    }
    assert_eq!(read_cfg(&home, "enabled").as_deref(), Some("true"));
    assert_eq!(read_cfg(&home, "output_compression").as_deref(), Some("off"));

    // 骨架：200 行 → 只保留 head 5 + tail 3。量化收益：≤50% 原文。
    let content = file_lines(200);
    let (stdout, sk, file) = make_skeleton(&home, &project, "s1", &content);
    let sk_bytes = std::fs::read_to_string(&sk).unwrap();
    let orig_bytes = std::fs::metadata(&file).unwrap().len() as usize;
    let lines: Vec<&str> = sk_bytes.lines().collect();
    assert!(lines[0].starts_with("[TSK SKELETON of s1.txt | 200 lines total"));
    assert!(lines[1].starts_with("     1\t") && lines[1].ends_with("line 1"));
    assert!(lines[6].starts_with("[TSK omitted lines 6..197."));
    assert!(lines[6].contains("Read s1.txt with offset/limit"));
    assert!(lines[9].starts_with("   200\t"));
    assert_eq!(lines.len(), 10);
    let _ = quant("skeleton", "200行 -> head5+tail3", orig_bytes, sk_bytes.len(), 0.5);
    let _ = stdout;

    // 逃逸口：offset/limit → passthrough + ledger retrieval。
    let stdin = pretool_ev("s1", &project, &format!(r#"{{"file_path":"{}","offset":50,"limit":10}}"#, p(&file)));
    let out = run(&home, &["hook", "PreToolUse"], &stdin, &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
    let led = read_home(&home, "s1", "ledger.jsonl");
    assert!(led.contains("\"kind\":\"rewrite\""));
    assert!(led.contains("\"saved\":"));
    assert!(led.contains("\"kind\":\"retrieval\""));
    assert!(led.contains("escape_hatch_offset_limit"));

    // fail-open：坏 JSON / 未知事件 / 空输入 → `{}`、exit 0、stderr 报错。
    for (args, stdin) in [
        (["hook", "PreToolUse"], "not json"),
        (["hook", "NoSuchEvent"], "{}"),
        (["hook", "PreCompact"], ""),
    ] {
        let out = run(&home, &args, stdin, &project);
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("fail-open"), "{args:?}");
    }
}

// =====================================================================
// 2. 无损：骨架 + 逃逸口 → 原文件任意行可重建
// =====================================================================
#[test]
fn fidelity_skeleton_random_lines() {
    eprintln!("== fidelity: 骨架后任意行可经 offset/limit 无损取回 ==");
    let (home, project) = setup();
    let content = file_lines(200);
    let (_stdout, sk, file) = make_skeleton(&home, &project, "s2", &content);
    let sk_text = std::fs::read_to_string(&sk).unwrap();

    // 骨架保留的 head/tail 行必须与原文逐字节一致。
    let orig_lines: Vec<&str> = content.lines().collect();
    let sk_lines: Vec<&str> = sk_text.lines().collect();
    for keep in [1usize, 5, 198, 200] {
        let sk_idx = if keep <= 5 { keep } else { 7 + (keep - 198) };
        let sk_line = sk_lines[sk_idx].split('\t').nth(1).unwrap();
        assert_eq!(sk_line, orig_lines[keep - 1], "head/tail 行 {keep} 必须原文");
    }

    // 中间行（6..197）：offset/limit 读原文件取回 == 原文（确定性伪随机 5 个行号）。
    for n in [10usize, 50, 100, 150, 197] {
        let stdin = pretool_ev("s2", &project, &format!(r#"{{"file_path":"{}","offset":{},"limit":1}}"#, p(&file), n));
        let out = run(&home, &["hook", "PreToolUse"], &stdin, &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "逃逸口须 passthrough");
        // 取回动作 = 原文件 offset/limit 切片；逃逸口放行后该读可执行。
        let slice = content.lines().nth(n - 1).unwrap();
        assert_eq!(slice, orig_lines[n - 1], "行 {n} 无损");
        assert!(sk_text.contains(&format!("[TSK omitted lines 6..197.")));
    }
}

// =====================================================================
// 3. 无损：externalize 全文回读 == 原始（逐字节）
// =====================================================================
#[test]
fn fidelity_externalize_roundtrip() {
    eprintln!("== fidelity: externalize 全文逐字节 == 原始 ==");
    let (home, project) = setup();
    let content: String = (1..=4000usize)
        .map(|i| format!("log {i} request /api/{i} status 200 latency 45ms\n"))
        .collect();
    assert!(content.len() > 100 * 1024);
    let (_stdout, pointer) = {
        let file = project.join("big.log");
        std::fs::write(&file, &content).unwrap();
        let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s3", &project, &read_input(&file)), &project);
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let pointer = assert_rewrite_to(&stdout, "");
        (stdout, pointer)
    };

    // 指针第一行给出 full 路径，且该文件内容 == 原始。
    let ptr_text = std::fs::read_to_string(&pointer).unwrap();
    let full = ptr_text.lines().next().unwrap().split("full: ").nth(1).unwrap().trim_end_matches(']');
    let full_path = Path::new(full);
    assert!(full_path.exists(), "full 路径存在: {full}");
    assert_eq!(
        std::fs::read(full_path).unwrap(),
        content.as_bytes(),
        "外置全文必须与原始逐字节相等"
    );
    let _ = quant("externalize", "4000行日志", content.len(), ptr_text.len(), 0.98);

    // 回读 ext 全文 = 显式检索：passthrough，不再 externalize（防死循环）。
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s3", &project, &read_input(full_path)), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "ext 全文回读应 passthrough");
}

// =====================================================================
// 4. 无损：中文 / UTF-8
// =====================================================================
#[test]
fn fidelity_chinese_utf8() {
    eprintln!("== fidelity: 中文内容骨架保留 + 取回一致 ==");
    let (home, project) = setup();
    let content = (1..=20).map(|i| format!("第{i}行：修复登录接口的边界条件\n")).collect::<String>();
    let (_stdout, sk, file) = make_skeleton(&home, &project, "s4", &content);
    let sk_text = std::fs::read_to_string(&sk).unwrap();

    // 骨架 head/tail 必须保留中文行（TAB 分隔的原文逐字节一致）。
    let orig: Vec<&str> = content.lines().collect();
    let sk_lines: Vec<&str> = sk_text.lines().collect();
    for keep in [1usize, 5, 18, 20] {
        let sk_idx = if keep <= 5 { keep } else { 7 + (keep - 18) };
        let got = sk_lines[sk_idx].split('\t').nth(1).unwrap();
        assert_eq!(got, orig[keep - 1], "中文行 {keep}");
    }
    assert!(sk_text.starts_with("[TSK SKELETON of s4.txt | 20 lines total"));

    // 中间行逃逸口取回 == 原文。
    let stdin = pretool_ev("s4", &project, &format!(r#"{{"file_path":"{}","offset":10,"limit":1}}"#, p(&file)));
    let out = run(&home, &["hook", "PreToolUse"], &stdin, &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
    assert_eq!(content.lines().nth(9).unwrap(), orig[9], "第 10 行无损");
}

// =====================================================================
// 5. 输入压缩边界：小 / 大(externalize) / 二进制 / 缺失
// =====================================================================
#[test]
fn input_boundaries() {
    eprintln!("== input_boundaries: 小/大/二进制/缺失 ==");
    let (home, project) = setup();

    let small = project.join("small.txt");
    std::fs::write(&small, "a\nb\nc\n").unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s5", &project, &read_input(&small)), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "小文件(<8行) passthrough");
    assert_eq!(read_home(&home, "s5", "ledger.jsonl"), "", "小文件不记账");

    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s5", &project, &read_input(&project.join("missing.txt"))), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "缺失文件 passthrough");

    let bin = project.join("bin.dat");
    std::fs::write(&bin, b"line1\xFF\xFE\nline2\x80\nline3\x00\xFF\n").unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s5", &project, &read_input(&bin)), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "二进制 passthrough");

    // 大文件 → externalize，量化收益 ≥98%。
    let big = project.join("big.log");
    let content: String = (1..=4000usize).map(|i| format!("log {i} request /api/{i} status 200 latency 45ms\n")).collect();
    std::fs::write(&big, &content).unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("s5", &project, &read_input(&big)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let pointer = assert_rewrite_to(&stdout, "");
    let ptr_text = std::fs::read_to_string(&pointer).unwrap();
    let _ = quant("externalize", "4000行/120KB+ 日志", content.len(), ptr_text.len(), 0.98);
    assert!(ptr_text.starts_with("[TSK EXTERNALIZED output of Read | original"));
    assert!(ptr_text.contains("[TSK excerpt: head 1KB follows"));
    let led = read_home(&home, "s5", "ledger.jsonl");
    assert!(led.contains("\"kind\":\"externalize\""));
    assert!(led.contains(&format!("\"orig_bytes\":{}", content.len())));
}

// =====================================================================
// 6. 沙箱：env 过滤 / 解释器 / 统计任务（复杂→结论）
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
fn reliability() {
    eprintln!("== reliability: init 幂等 / 并发 ledger ==");
    let (home, project) = setup();

    // init 两次：TSK hook 条目不得重复（否则 claude 会双重执行 hook）。
    run(&home, &["init"], "", &project);
    run(&home, &["init"], "", &project);
    let settings = std::fs::read_to_string(project.join(".claude/settings.json")).unwrap();
    for ev in ["PreToolUse", "SessionStart", "PreCompact"] {
        let needle = format!("tsk hook {ev}");
        assert_eq!(settings.matches(&needle).count(), 1, "{needle} 必须只出现一次:\n{settings}");
    }

    // 一键关闭 → 重开闭环：off 摘除 hooks 后，init 必须能重新接上。
    run(&home, &["off"], "", &project);
    let settings = std::fs::read_to_string(project.join(".claude/settings.json")).unwrap();
    assert!(!settings.contains("tsk hook"), "off 后 hooks 摘除");
    run(&home, &["init"], "", &project);
    let settings = std::fs::read_to_string(project.join(".claude/settings.json")).unwrap();
    for ev in ["PreToolUse", "SessionStart", "PreCompact"] {
        let needle = format!("tsk hook {ev}");
        assert_eq!(settings.matches(&needle).count(), 1, "off→init 后 {needle} 恢复且不重复");
    }

    // 并发写 ledger：8 个进程同时触发 retrieval，每行都是合法 JSON。
    let file = project.join("bigfile.txt");
    std::fs::write(&file, file_lines(100)).unwrap();
    let mut children = Vec::new();
    for i in 0..8 {
        let stdin = pretool_ev(&format!("conc{i}"), &project, &format!(r#"{{"file_path":"{}","offset":1,"limit":1}}"#, p(&file)));
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_tsk"))
            .args(["hook", "PreToolUse"])
            .env("TSK_HOME", &home)
            .current_dir(&project)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        if let Some(mut s) = child.stdin.take() {
            let _ = std::io::Write::write_all(&mut s, stdin.as_bytes());
        }
        children.push(child);
    }
    for c in children {
        c.wait_with_output().unwrap();
    }
    let mut rows = 0usize;
    let mut bad = 0usize;
    for i in 0..8 {
        let led = read_home(&home, &format!("conc{i}"), "ledger.jsonl");
        for line in led.lines().filter(|l| !l.is_empty()) {
            rows += 1;
            if serde_json::from_str::<serde_json::Value>(line).is_err() {
                bad += 1;
            }
        }
    }
    eprintln!("  [量化] 并发 ledger: {rows} 行, 坏行 {bad}");
    assert_eq!(rows, 8, "每个并发进程各写 1 行");
    assert_eq!(bad, 0, "并发下每行必须仍是合法 JSON");
}

// =====================================================================
// 10. skeleton 多长度：8(边界不压)/9/20/99/200/999 行
// =====================================================================
#[test]
fn skeleton_various_lengths() {
    eprintln!("== skeleton: 多长度（8 边界 / 9 / 20 / 99 / 200 / 999 行）==");
    let (home, project) = setup();

    // 8 行：head5+tail3 会触底 → 不压缩（MIN_LINES 边界）。
    let f8 = project.join("n8.txt");
    std::fs::write(&f8, file_lines(8)).unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("L8", &project, &read_input(&f8)), &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "8 行不压");

    // 9 / 20 行：骨架生成但 overhead（header+marker+行号）> 原文 → tokenizer-count
    // gate（§11.3）回退为原值，恒 passthrough。这是防负收益的设计。
    for n in [9usize, 20] {
        let f = project.join(format!("n{n}.txt"));
        std::fs::write(&f, file_lines(n)).unwrap();
        let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(&format!("L{n}"), &project, &read_input(&f)), &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "{n} 行骨架膨胀须 gate 回退");
    }

    // 99 / 200 / 999：内容超过骨架 overhead → 压缩，列宽 2→3 位仍 6 列对齐。
    for n in [99usize, 200, 999] {
        let (_, sk, file) = make_skeleton(&home, &project, &format!("L{n}"), &file_lines(n));
        let t = std::fs::read_to_string(&sk).unwrap();
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines[0].starts_with(&format!("[TSK SKELETON of L{n}.txt | {n} lines total")));
        let tail_first = lines[7].split('\t').next().unwrap().trim();
        assert_eq!(tail_first, format!("{}", n - 2), "尾行起始号 {n}-2");
        let last = lines[9].split('\t').next().unwrap().trim();
        assert_eq!(last, format!("{n}"), "尾行号 {n}");
        assert_eq!(lines.len(), 10, "恒为 head5+marker+tail3");
        let orig = std::fs::metadata(&file).unwrap().len() as usize;
        let _ = quant("skeleton", &format!("{n} 行"), orig, t.len(), 0.4);
    }
}

// =====================================================================
// 11. Bash：v1 不改写 Bash 事件（恒 passthrough）+ 沙箱执行 bash 命令收益
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
fn escape_hatch_matrix() {
    eprintln!("== escape: offset/limit 形态矩阵 ==");
    let (home, project) = setup();
    let file = project.join("bigfile.txt");
    std::fs::write(&file, file_lines(200)).unwrap();
    let fp = p(&file);

    // (tool_input 片段, 说明)。逃逸口恒开（spec §5.3 / S2b），不可配置。
    let cases: &[(&str, &str)] = &[
        (r#"{"file_path":"FP","offset":50}"#, "仅 offset"),
        (r#"{"file_path":"FP","limit":10}"#, "仅 limit"),
        (r#"{"file_path":"FP","offset":0,"limit":10}"#, "offset=0"),
        (r#"{"file_path":"FP","limit":0}"#, "limit=0"),
        (r#"{"file_path":"FP","limit":999999}"#, "limit 超大"),
        (r#"{"file_path":"FP","offset":-1}"#, "offset 负值"),
    ];
    for (ti, why) in cases {
        let ti = ti.replace("FP", &fp);
        let stdin = pretool_ev("e1", &project, &ti);
        let out = run(&home, &["hook", "PreToolUse"], &stdin, &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "{why} 须 passthrough");
        // 每次都记一条 retrieval。
        let led = read_home(&home, "e1", "ledger.jsonl");
        assert!(led.contains("escape_hatch_offset_limit"), "{why} 记 retrieval:\n{led}");
        // 失败场景：文件不存在 + offset；二进制 + offset —— 逃逸口仍放行。
        let bad = pretool_ev("e1", &project, &format!(r#"{{"file_path":"{}","offset":10,"limit":1}}"#, p(&project.join("nope.txt"))));
        let out = run(&home, &["hook", "PreToolUse"], &bad, &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "缺失文件+offset 恒 passthrough");
        let bin = project.join("b.dat");
        std::fs::write(&bin, b"x\xFF\xFE\n").unwrap();
        let bad = pretool_ev("e1", &project, &format!(r#"{{"file_path":"{}","offset":1,"limit":1}}"#, p(&bin)));
        let out = run(&home, &["hook", "PreToolUse"], &bad, &project);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "二进制+offset 恒 passthrough");
    }
}

// =====================================================================
// 15. 多轮会话：5 轮混合任务（骨架/外置/沙箱/短文本/快照），累计 token 成本与节省
// =====================================================================
#[test]
fn multi_turn_session() {
    eprintln!("== multi_turn: 30 轮混合任务（骨架/外置/沙箱/逃逸口/PreCompact）==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "mt";
    let start = |source: &str| format!(r#"{{"session_id":"{sid}","cwd":"/","source":"{source}","hook_event_name":"SessionStart"}}"#);
    let prompt = |text: &str| format!(r#"{{"session_id":"{sid}","prompt":"{text}","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);

    // 轮 0：SessionStart 注入 ruleset。
    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &start("startup"), &project).stdout));
    assert!(ctx.contains("TSK OUTPUT MODE ACTIVE"), "轮0 ruleset");

    // 跨轮复用的沙箱脚本 + 外置大日志。
    let sx = std::env::temp_dir().join(format!("tsk-mt-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let log = sx.join("a.log");
    std::fs::write(&log, (1..=500).map(|i| format!("line {i} err={}", if i % 7 == 0 { 1 } else { 0 })).collect::<Vec<_>>().join("\n")).unwrap();
    let sh = sx.join("t.sh");
    std::fs::write(&sh, "#!/bin/sh\ngrep -c err=1 \"$1\"\n").unwrap();
    let big = project.join("server.log");
    let big_body: String = (1..=4000usize).map(|i| format!("log {i} request /api/{i} status 200 latency 45ms\n")).collect();
    std::fs::write(&big, &big_body).unwrap();

    let mut n_rewrite = 0usize;
    let mut n_externalize = 0usize;
    let mut n_sandbox = 0usize;
    let mut n_retrieval = 0usize;
    for i in 0..30 {
        // 每轮一条用户消息 → 输出压缩每轮强化必须注入、不漂移。
        let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "UserPromptSubmit"], &prompt(&format!("第 {i} 轮任务")), &project).stdout));
        assert!(ctx.contains("Enforce this reply"), "轮 {i} 强化注入");

        match i % 4 {
            0 => {
                let f = project.join(format!("mod{i}.rs"));
                std::fs::write(&f, file_lines(200 + i * 7)).unwrap();
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&f)), &project);
                assert!(String::from_utf8_lossy(&out.stdout).contains("hookSpecificOutput"), "轮{i} 骨架");
                n_rewrite += 1;
            }
            1 => {
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&big)), &project);
                assert!(String::from_utf8_lossy(&out.stdout).contains("hookSpecificOutput"), "轮{i} 外置");
                n_externalize += 1;
            }
            2 => {
                let out = run(&home, &["exec", p(&sh).as_str(), p(&log).as_str()], "", &sx);
                assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "71", "轮{i} 沙箱结论");
                n_sandbox += 1;
            }
            _ => {
                let f = project.join("mod0.rs");
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &format!(r#"{{"file_path":"{}","offset":50,"limit":1}}"#, p(&f))), &project);
                assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "轮{i} 逃逸口");
                n_retrieval += 1;
            }
        }
        if i == 14 || i == 29 {
            let t = project.join("transcript.jsonl");
            std::fs::write(&t, r#"{"type":"user","message":{"content":"refactor db"}}{"type":"assistant","message":{"content":[{"type":"text","text":"Decision: use WAL"}]}}"#).unwrap();
            let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "PreCompact"], &format!(r#"{{"session_id":"{sid}","cwd":"/","transcript_path":"{}","trigger":"manual","hook_event_name":"PreCompact"}}"#, p(&t)), &project).stdout));
            assert!(ctx.contains("[TSK SKELETON of"), "轮{i} 锚点保护");
        }
    }
    assert!(n_rewrite >= 7 && n_externalize >= 7 && n_sandbox >= 7 && n_retrieval >= 7,
        "30 轮里四种动作都要覆盖: rw={n_rewrite} ext={n_externalize} sx={n_sandbox} ret={n_retrieval}");

    // 量化：从 ledger 读真实 saved/orig → 绝对值 + 比例。
    let led = read_home(&home, sid, "ledger.jsonl");
    let mut saved_bytes = 0usize;
    let mut orig_bytes = 0usize;
    for line in led.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
        match v.get("kind").and_then(|k| k.as_str()) {
            Some("rewrite") | Some("sandbox") => {
                saved_bytes += v.get("saved").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
                orig_bytes += v.get("orig_bytes").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
            }
            Some("externalize") => {
                let o = v.get("orig_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                let n = v.get("new_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                orig_bytes += o as usize;
                saved_bytes += o.saturating_sub(n) as usize;
            }
            _ => {}
        }
    }
    let inject_rows = led.matches("\"kind\":\"inject\"").count();
    let inject_cost = est_tok(3853 + 30 * 200 + 2 * 250); // ruleset + 30×reinforce + 2×precompact
    let save_tokens = est_tok(saved_bytes);
    let orig_tokens = est_tok(orig_bytes);
    let ratio = if orig_bytes > 0 { saved_bytes as f64 / orig_bytes as f64 } else { 0.0 };
    eprintln!(
        "  [量化] 30轮: 原文 {orig_bytes}B(est {orig_tokens} tok) | 压缩 saved {saved_bytes}B(est {save_tokens} tok) | 注入成本 est {inject_cost} tok | 净省 est {} tok | 节省比例 {:.1}% | 注入记账 {inject_rows} 行",
        save_tokens.saturating_sub(inject_cost),
        ratio * 100.0
    );
    assert!(inject_rows >= 32, "注入记账（1 ruleset + 30 reinforce + 2 precompact）: {inject_rows}");
    assert!(save_tokens > inject_cost, "30 轮必须净省 token: {save_tokens} vs {inject_cost}");
    assert!(ratio > 0.5, "30 轮压缩比例应 >50%: {:.1}%", ratio * 100.0);
    for f in [sh, log] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 16. 逃逸口取回成本：压缩后模型需要全文的额外 token 开销
// =====================================================================
#[test]
fn escape_retrieval_cost() {
    eprintln!("== escape: 压缩后需全文/多行的额外 token 开销 ==");
    let (home, project) = setup();
    let content = file_lines(200);
    let (_stdout, sk, _file) = make_skeleton(&home, &project, "cost", &content);
    let sk_len = std::fs::read_to_string(&sk).unwrap().len();
    let orig_len = content.len();

    // 三场景：摘要 / 中间 2 行 / 全文。取回 = offset/limit 工具调用（内容 + 调用开销 ~90B）。
    let per_call = 90usize;
    let a_only_skel = est_tok(sk_len); // A：只要摘要 → 读骨架
    let b_two_rows = est_tok(sk_len + 2 * per_call); // B：骨架 + 2 次取回
    let c_direct = est_tok(orig_len); // C-直读：模型直接 Read 全文
    let c_via_skel = est_tok(sk_len + 5 * 450); // C-经骨架：骨架 + 5 次 limit=50 全量取回
    let overhead = c_via_skel.saturating_sub(c_direct);

    eprintln!("  [量化] A 只要摘要: 读骨架 est {a_only_skel} tok");
    eprintln!("  [量化] B 要中间 2 行: 骨架+2次取回 est {b_two_rows} tok (直读全文 est {c_direct})");
    eprintln!("  [量化] C 要全文: 直读 est {c_direct} tok vs 骨架+5次取回 est {c_via_skel} tok → 额外开销 {overhead} tok ({:.0}%)",
        if c_direct > 0 { overhead as f64 * 100.0 / c_direct as f64 } else { 0.0 });
    assert!(a_only_skel < c_direct, "只要摘要时压缩必须省");
    assert!(b_two_rows < c_direct, "取少量行时压缩仍省");
    assert!(c_via_skel > c_direct, "要全文时逃逸口取回有额外开销（如实量化，不隐藏）");
}

// =====================================================================
// 17. 真实工程 30 轮：复制 TSK 工程，定位功能/找 bug/修复，量化节省绝对值+比例
// =====================================================================
fn copy_dir_all(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let t = dst.join(e.file_name());
        if e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            copy_dir_all(&e.path(), &t);
        } else {
            let _ = std::fs::copy(e.path(), &t);
        }
    }
}

#[test]
fn real_project_30_turns() {
    eprintln!("== real_project: 复制 TSK 工程，30 轮读代码定位/找 bug/修复任务 ==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "rp";
    let start = |source: &str| format!(r#"{{"session_id":"{sid}","cwd":"/","source":"{source}","hook_event_name":"SessionStart"}}"#);
    let prompt = |text: &str| format!(r#"{{"session_id":"{sid}","prompt":"{text}","cwd":"/","hook_event_name":"UserPromptSubmit"}}"#);

    // 复制真实工程（当前包根 = 工程根），排除构建产物。
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let dst = project.join("repo");
    copy_dir_all(&src, &dst);
    let _ = std::fs::remove_dir_all(dst.join("target"));
    let files = [
        "core/src/inject.rs",
        "core/src/compress/skeleton.rs",
        "core/src/config.rs",
        "info.md",
        "core/src/assets/ruleset.txt",
        "cli/src/main.rs",
        "README.md",
        "core/src/ledger.rs",
        "core/src/sandbox.rs",
        "core/src/snapshot.rs",
    ];
    for f in files {
        assert!(dst.join(f).exists(), "工程文件存在: {f}");
    }

    let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "SessionStart"], &start("startup"), &project).stdout));
    assert!(ctx.contains("TSK OUTPUT MODE ACTIVE"));

    let mut n_rewrite = 0usize;
    let mut n_escape = 0usize;
    for i in 0..30 {
        let task = match i % 3 {
            0 => "定位这个工程的核心功能",
            1 => "找到代码里的边界条件 bug",
            _ => "评估 TSK 的 token 节省机制并指出改进点",
        };
        let ctx = parse_ctx(&String::from_utf8_lossy(&run(&home, &["hook", "UserPromptSubmit"], &prompt(&format!("第{i}轮：{task}")), &project).stdout));
        assert!(ctx.contains("Enforce this reply"), "轮{i} 强化");

        match i % 3 {
            0 => {
                let f = files[(i / 3) % files.len()];
                let fp = dst.join(f);
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&fp)), &project);
                let s = String::from_utf8_lossy(&out.stdout).into_owned();
                if s.contains("hookSpecificOutput") {
                    n_rewrite += 1;
                }
            }
            1 => {
                // 逃逸口：定位 inject.rs 第 30 行附近。
                let fp = dst.join("core/src/inject.rs");
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &format!(r#"{{"file_path":"{}","offset":30,"limit":10}}"#, p(&fp))), &project);
                assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "轮{i} 逃逸口");
                n_escape += 1;
            }
            _ => {
                // 读小文件（gate 回退或骨架）：断言输出合法（{} 或 updatedInput），不崩溃。
                let fp = dst.join("core/src/policy.rs");
                let out = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&fp)), &project);
                let s = String::from_utf8_lossy(&out.stdout).into_owned();
                assert!(s == "{}" || s.contains("hookSpecificOutput"), "轮{i} policy.rs 处理");
            }
        }
        if i == 14 || i == 29 {
            let t = project.join("transcript.jsonl");
            std::fs::write(&t, r#"{"type":"user","message":{"content":"refactor db"}}{"type":"assistant","message":{"content":[{"type":"text","text":"Decision: use WAL"}]}}"#).unwrap();
            let _ = run(&home, &["hook", "PreCompact"], &format!(r#"{{"session_id":"{sid}","cwd":"/","transcript_path":"{}","trigger":"manual","hook_event_name":"PreCompact"}}"#, p(&t)), &project);
        }
    }
    assert!(n_rewrite > 5, "真实工程多轮必须命中骨架/外置: {n_rewrite}");
    assert!(n_escape > 5, "逃逸口轮数: {n_escape}");

    // 绝对值 + 比例（不压缩基线 = 全部 Read 原文字节）。
    let led = read_home(&home, sid, "ledger.jsonl");
    let mut saved_bytes = 0usize;
    let mut orig_bytes = 0usize;
    for line in led.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
        match v.get("kind").and_then(|k| k.as_str()) {
            Some("rewrite") | Some("sandbox") => {
                saved_bytes += v.get("saved").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
                orig_bytes += v.get("orig_bytes").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
            }
            Some("externalize") => {
                let o = v.get("orig_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                let n = v.get("new_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                orig_bytes += o as usize;
                saved_bytes += o.saturating_sub(n) as usize;
            }
            _ => {}
        }
    }
    let inject_cost = est_tok(3853 + 30 * 200 + 2 * 250);
    let save_tokens = est_tok(saved_bytes);
    let orig_tokens = est_tok(orig_bytes);
    let ratio = if orig_bytes > 0 { saved_bytes as f64 / orig_bytes as f64 } else { 0.0 };
    eprintln!(
        "  [量化] 真实工程30轮: 原文 {orig_bytes}B(est {orig_tokens} tok) | 压缩 saved {saved_bytes}B(est {save_tokens} tok) | 注入成本 est {inject_cost} tok | 净省 est {} tok | 节省比例 {:.1}% | 绝对值 {saved_bytes}B",
        save_tokens.saturating_sub(inject_cost),
        ratio * 100.0
    );
    assert!(saved_bytes > 0, "真实工程必须有压缩收益");
    assert!(ratio > 0.3, "真实工程 30 轮压缩比例应 >30%: {:.1}%", ratio * 100.0);
}

// =====================================================================
// 18. 输出压缩整体收益：注入开销（ruleset+每轮 reinforce）vs 输出节省 → 净收益与 break-even
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
fn posttooluse_observation() {
    eprintln!("== posttooluse: 恒 passthrough + 观测记录 ==");
    let (home, project) = setup();
    let ev = r#"{"session_id":"po","cwd":"/","hook_event_name":"PostToolUse","tool_name":"Read","tool_use_id":"tu-1","tool_input":{"file_path":"/x"}}"#;
    let out = run(&home, &["hook", "PostToolUse"], ev, &project);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}", "PostToolUse 恒 passthrough");
    assert_eq!(out.status.code(), Some(0));
    let log = read_home(&home, "po", "events.log");
    assert!(log.contains("posttooluse: tool=Read tu_id=tu-1"), "观测行：\n{log}");
}

// =====================================================================
// 21. 沙箱示例脚本可直接使用（analyze_log.py）
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
fn sandbox_auto_routing() {
    eprintln!("== sandbox: P-1 自动路由（脚本存在才沙箱）==");
    let (home, project) = setup();
    let log = project.join("access.log");
    let body: String = (1..=2000usize).map(|i| format!("192.168.0.{i} GET /api/{i} status 200\n")).collect();
    std::fs::write(&log, &body).unwrap();
    assert!(body.len() > 50 * 1024, "fixture 须 >50KB: {}", body.len());
    let sxdir = project.join(".tsk/sandbox");
    std::fs::create_dir_all(&sxdir).unwrap();
    std::fs::write(sxdir.join("analyze_log.sh"), "#!/bin/sh\nwc -l < \"$1\"\n").unwrap();

    // Read .log（>50KB 聚合型）→ 沙箱结论进 context。
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("p1", &project, &read_input(&log)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let summary = Path::new(v.pointer("/hookSpecificOutput/updatedInput/file_path").unwrap().as_str().unwrap());
    assert!(summary.exists(), "summary 存在");
    let txt = std::fs::read_to_string(summary).unwrap();
    assert_eq!(txt.trim(), "2000", "沙箱结论 = 行数");
    let led = read_home(&home, "p1", "ledger.jsonl");
    assert!(led.contains("\"kind\":\"sandbox\""), "sandbox ledger:\n{led}");
    let _ = quant("sandbox自动路由", "2000行log -> 行数结论", body.len(), txt.len(), 0.9);

    // 缺 analyze_<ext> 脚本 → 回退骨架（不猜、不现造脚本）。
    let log2 = project.join("data.jsonl");
    let body2: String = (1..=2000usize).map(|i| format!("{{\"i\":{i}}}\n")).collect();
    std::fs::write(&log2, &body2).unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("p1", &project, &read_input(&log2)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let p = v.pointer("/hookSpecificOutput/updatedInput/file_path").unwrap().as_str().unwrap();
    let sk = std::fs::read_to_string(p).unwrap();
    assert!(sk.contains("[TSK SKELETON of"), "缺脚本回退骨架:\n{}", &sk[..80]);

    // Bug D 回归：P-1 沙箱 stdout >64KB 也必须截断外置（同 `tsk exec` 行为）。
    // 路由优先级：>100KB 恒外置；沙箱只作用于 50–100KB 的聚合型文件。故 fixture 取 ~70KB。
    let bigout_log = project.join("bigout.jsonl");
    std::fs::write(&bigout_log, (1..=2000usize).map(|i| format!("entry {i} payload payload payload\n")).collect::<String>()).unwrap();
    assert!(std::fs::metadata(&bigout_log).unwrap().len() > 50 * 1024, "须 >50KB 走沙箱候选");
    std::fs::write(sxdir.join("analyze_jsonl.sh"), "#!/bin/sh\ncat \"$1\"\n").unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("p1", &project, &read_input(&bigout_log)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let sum = Path::new(v.pointer("/hookSpecificOutput/updatedInput/file_path").unwrap().as_str().unwrap());
    let sum_txt = std::fs::read_to_string(sum).unwrap();
    assert!(sum_txt.contains("TSK: output truncated"), "P-1 沙箱大输出须截断提示:\n{}", &sum_txt[..160]);
    let orig_len = std::fs::metadata(&bigout_log).unwrap().len() as usize;
    assert!(sum_txt.len() < orig_len, "summary 须 < 原文");
    let _ = quant("sandbox自动路由(大输出)", "2000行/69KB jsonl 截断", orig_len, sum_txt.len(), 0.85);
}

// =====================================================================
// 23. 输出压缩 lite 档：更短 ruleset（省输入 token）
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
fn skeleton_edge_cases() {
    eprintln!("== skeleton: 边界（CRLF/空行/超长行）==");
    let (home, project) = setup();
    // CRLF 文件（Windows 换行，40 行）：行内容不得带 \r。
    let crlf = project.join("crlf.txt");
    std::fs::write(&crlf, (1..=40).map(|i| format!("line {i}\r\n")).collect::<String>()).unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("ed", &project, &read_input(&crlf)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let sk = Path::new(v.pointer("/hookSpecificOutput/updatedInput/file_path").unwrap().as_str().unwrap());
    let skt = std::fs::read_to_string(sk).unwrap();
    assert!(!skt.contains('\r'), "CRLF 行尾被剥除");
    assert!(skt.contains("line 40"), "尾部行正确");
    let _ = quant("skeleton", "40行 CRLF", std::fs::metadata(&crlf).unwrap().len() as usize, skt.len(), 0.1);

    // 空行混合（80 行，原文须 > 骨架 overhead 才不被 gate 回退）：空行原样保留。
    let blank = project.join("blank.txt");
    std::fs::write(&blank, (1..=80).map(|i| if i % 4 == 0 { "\n".to_string() } else { format!("line {i} with some content to beat the skeleton overhead\n") }).collect::<String>()).unwrap();
    let (_s, sk2, _f) = make_skeleton(&home, &project, "bl", &std::fs::read_to_string(&blank).unwrap());
    let skt = std::fs::read_to_string(&sk2).unwrap();
    assert!(skt.contains("\t\n") || skt.lines().any(|l| l.trim_end().is_empty() || l.ends_with('\t')), "空行保留:\n{skt}");
    eprintln!("  [量化] skeleton 空行: 80 行文件(含空行) -> 骨架保留空行结构");

    // 超长单行（200KB，1 行）：<8 行不骨架——但不拦外置。>100KB 恒外置（Bug B 回归点：
    // 单行 minified JSON / base64 / 单行日志也应走 ext 指针，不该整份进 context）。
    let huge = project.join("one.json");
    std::fs::write(&huge, "x".repeat(200 * 1024)).unwrap();
    let out = run(&home, &["hook", "PreToolUse"], &pretool_ev("ed", &project, &read_input(&huge)), &project);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let pointer = Path::new(v.pointer("/hookSpecificOutput/updatedInput/file_path").unwrap().as_str().unwrap());
    let ptr = std::fs::read_to_string(pointer).unwrap();
    assert!(ptr.starts_with("[TSK EXTERNALIZED output of Read | original 204800 bytes"), "单行 200KB 应外置而非 passthrough");
    let _ = quant("externalize", "200KB 单行 json", 200 * 1024, ptr.len(), 0.99);
}

// =====================================================================
// 30. report 收益分布 + events.log 类别完整性
// =====================================================================
#[test]
fn report_breakdown_and_events() {
    eprintln!("== report/events: 收益分布 + 类别完整性 ==");
    let (home, project) = setup();
    write_cfg(&home, "enabled: true\noutput_compression: full\n");
    let sid = "rb";

    // 一个会话里触发多种动作：骨架 + 逃逸口 + 外置 + 沙箱 + 注入。
    let f = project.join("mod.rs");
    std::fs::write(&f, file_lines(200)).unwrap();
    let _ = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&f)), &project);
    let _ = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &format!(r#"{{"file_path":"{}","offset":5,"limit":3}}"#, p(&f))), &project);
    let big = project.join("big.log");
    std::fs::write(&big, (1..=4000usize).map(|i| format!("log {i} padding padding padding\n")).collect::<String>()).unwrap();
    let _ = run(&home, &["hook", "PreToolUse"], &pretool_ev(sid, &project, &read_input(&big)), &project);
    let sx = std::env::temp_dir().join(format!("tsk-rb-{}", std::process::id()));
    std::fs::create_dir_all(&sx).unwrap();
    let log = sx.join("a.log");
    std::fs::write(&log, (1..=500).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
    let sh = sx.join("t.sh");
    std::fs::write(&sh, "#!/bin/sh\nwc -l < \"$1\"\n").unwrap();
    let _ = run(&home, &["exec", "--session", sid, p(&sh).as_str(), p(&log).as_str()], "", &sx);
    let _ = run(&home, &["hook", "SessionStart"], &format!(r#"{{"session_id":"{sid}","cwd":"/","source":"startup","hook_event_name":"SessionStart"}}"#), &project);

    // report 收益分布（按工具）。
    let out = run(&home, &["report", "--session", sid], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Read"), "Read 收益分布:\n{s}");
    assert!(s.contains("Bash"), "Bash(沙箱) 收益分布:\n{s}");
    assert!(s.contains("saved"), "总 saved");

    // events.log 各类别完整性（Grep 友好，一行一事）。
    let evlog = read_home(&home, sid, "events.log");
    for cat in ["retrieval", "externalize", "sandbox", "inject", "prompt_turn", "posttooluse", "compact_advice"] {
        eprintln!("  [量化] events.log 类别 {cat}: {}", evlog.matches(cat).count());
    }
    assert!(evlog.contains("retrieval"), "retrieval 类别");
    assert!(evlog.contains("externalize"), "externalize 类别");
    assert!(evlog.contains("sandbox"), "sandbox 类别");
    eprintln!("  [量化] report: {}（多动作会话收益分布）", s.trim().replace('\n', " | "));
    for f in [sh, log] {
        let _ = std::fs::remove_file(f);
    }
}

// =====================================================================
// 31. auto 档 × resume：compact 后 resume 注入行为（成本=0，取舍一致）
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
