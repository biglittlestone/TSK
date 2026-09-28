//! End-to-end tests: real stdin/stdout against the compiled binary.
//! We test behaviour, not line coverage — one test per capability.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A hermetic TSK home + a fake project. `run_tsk` passes `TSK_HOME` to the child
/// explicitly, so tests can run in parallel without stomping each other's env.
fn setup() -> (PathBuf, PathBuf) {
    let d = std::env::temp_dir().join(format!(
        "tsk-e2e-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let home = d.join("home");
    let project = d.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".tsk/skeletons")).unwrap();
    (home, project)
}

/// Paths must be forward-slash when they go into JSON; Rust on Windows is happy
/// with either but JSON is not (`:` + `\U` is an invalid escape).
fn p(s: &Path) -> String {
    s.to_string_lossy().replace('\\', "/")
}

fn run_tsk(home: &PathBuf, args: &[&str], stdin: &str, cwd: &Path) -> std::process::Output {
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
            // Inline YAML comments: take only the first token as the value.
            l.split_once(':')
                .filter(|(k, _)| k.trim() == key)
                .and_then(|(_, v)| v.split('#').next())
                .map(|v| v.trim().to_string())
        })
}




#[test]
fn tiny_and_missing_files_pass_through() {
    let (home, project) = setup();
    // 3-line file: below MIN_LINES=8, never compressed.
    let tiny = project.join("small.txt");
    std::fs::write(&tiny, "a\nb\nc\n").unwrap();
    for fp in ["small.txt", "does-not-exist.txt"] {
        let stdin = format!(
            r#"{{"session_id":"s1","cwd":"{}","tool_name":"Read","tool_input":{{"file_path":"{}"}}}}"#,
            p(&project),
            p(&project.join(fp))
        );
        let out = run_tsk(&home, &["hook", "PreToolUse"], &stdin, &project);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "{}",
            "expected passthrough for {fp}"
        );
    }
}

#[test]
fn session_start_injects_ruleset_only_when_enabled() {
    let (home, project) = setup();
    let stdin = r#"{"session_id":"s1","cwd":"/","source":"startup"}"#;
    // Default: output_compression off ⇒ passthrough.
    assert_eq!(run_tsk(&home, &["hook", "SessionStart"], stdin, &project).stdout, b"{}");
    // Turn output compression on ⇒ ruleset arrives.
    std::fs::write(home.join("config.yaml"), "enabled: true\noutput_compression: full\n").unwrap();
    let out = run_tsk(&home, &["hook", "SessionStart"], stdin, &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("hookSpecificOutput"));
    assert!(s.contains("TSK OUTPUT MODE ACTIVE"));
    assert!(s.contains("HARD RULES"));
}

#[test]
fn init_wires_hooks_and_off_strips_them_idempotently() {
    let (home, project) = setup();
    let _ = run_tsk(&home, &["init"], "", &project);
    let settings = project.join(".claude/settings.json");
    assert!(settings.exists());
    assert!(std::fs::read_to_string(&settings).unwrap().contains("tsk hook SessionStart"));
    assert_eq!(read_cfg(&home, "enabled").as_deref(), Some("true"));

    // `off` twice: the second run must be a no-op with no errors.
    for _ in 0..2 {
        let out = run_tsk(&home, &["off"], "", &project);
        assert!(String::from_utf8_lossy(&out.stdout).contains("config disabled"));
    }
    assert_eq!(read_cfg(&home, "enabled").as_deref(), Some("false"));
    assert!(
        !std::fs::read_to_string(&settings).unwrap().contains("tsk hook"),
        "TSK hooks should be stripped"
    );
}

#[test]
fn report_aggregates_the_ledger() {
    let (home, project) = setup();
    let sdir = home.join("sess-report");
    std::fs::create_dir_all(&sdir).unwrap();
    std::fs::write(
        sdir.join("ledger.jsonl"),
        r#"{"kind":"rewrite","ts":1,"session":"sess-report","tool":"Read","tu_id":"t1","strategy":"skeleton","orig_bytes":1000,"new_bytes":100,"saved":900}
{"kind":"retrieval","ts":2,"session":"sess-report","tool":"Read","reason":"escape_hatch_offset_limit"}
"#,
    )
    .unwrap();
    let out = run_tsk(&home, &["report", "--session", "sess-report"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!((s.contains("saved") && s.contains("900")));
    assert!((s.contains("retrievals") && s.contains("1")));
    assert!((s.contains("Read") && s.contains("900")));
}

#[test]
fn doctor_runs_and_reports_wiring() {
    let (home, project) = setup();
    let _ = run_tsk(&home, &["init"], "", &project);
    std::fs::write(home.join("config.yaml"), "enabled: true\n").unwrap();
    let out = run_tsk(&home, &["doctor"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("settings.json wired"));
    assert!(s.contains("[ok]"));
    // Bug E1 回归：doctor 不得以主入口 fail-open 兜底的 `{}` 尾巴收尾。
    assert!(!s.trim().ends_with("{}"), "doctor 输出不得含空对象尾缀:\n{s}");
    // Bug E2 回归："tsk in PATH" 检查基于 PATH 里是否真的存在可执行文件（split_paths 正确处理 :/;）。
    assert!(s.contains("tsk in PATH"), "doctor 须报告 PATH 检查:\n{s}");
}

#[test]
fn cache_hit_metric_from_a_transcript() {
    let (home, project) = setup();
    let t = project.join("transcript.jsonl");
    std::fs::write(
        &t,
        r#"{"type":"assistant","message":{"usage":{"input_tokens":1000,"cache_read_input_tokens":500}}}
"#,
    )
    .unwrap();
    let t_str = p(&t);
    let out = run_tsk(&home, &["report", "--cache-hit", &t_str], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("cache-hit"));
    assert!(s.contains("1 turns"));
    assert!(s.contains("50.0%"));
}

#[test]
fn report_defaults_to_current_session_and_clean() {
    let (home, project) = setup();
    // 模拟 hook 已写过的"当前会话"标记 + 该会话 ledger（含注入成本行）。
    std::fs::create_dir_all(home.join("cur")).unwrap();
    std::fs::write(
        home.join("cur/ledger.jsonl"),
        r#"{"kind":"rewrite","ts":1,"session":"cur","tool":"Read","tu_id":"t","strategy":"skeleton","orig_bytes":1000,"new_bytes":100,"saved":900}
{"kind":"inject","ts":2,"session":"cur","source":"session_start_ruleset","bytes":300}
"#,
    )
    .unwrap();
    std::fs::write(home.join("current-session"), "cur").unwrap();

    // 无参数 → 默认当前会话：saved + 注入成本 + 净值 + est tokens 预览。
    let out = run_tsk(&home, &["report"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("current session cur"), "默认统计当前会话:\n{s}");
    assert!((s.contains("saved") && s.contains("900")), "saved:\n{s}");
    assert!((s.contains("inject cost") && s.contains("300")), "注入成本:\n{s}");
    assert!((s.contains("net") && s.contains("600")), "净值 900-300:\n{s}");
    assert!(s.contains("est tokens"), "token 预览:\n{s}");

    // --all：跨会话全量。
    let out = run_tsk(&home, &["report", "--all"], "", &project);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!((s.contains("saved") && s.contains("900")) && (s.contains("inject cost") && s.contains("300")), "--all:\n{s}");

    // --clean：清空 ledger + 当前标记。
    let out = run_tsk(&home, &["report", "--clean"], "", &project);
    assert!(String::from_utf8_lossy(&out.stdout).contains("cleared"), "clean 输出");
    assert!(!home.join("cur/ledger.jsonl").exists(), "ledger 已清");
    assert!(!home.join("current-session").exists(), "当前标记已清");
}
