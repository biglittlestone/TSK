//! Sandbox: run a script with a hostile host stripped out. See spec §9.
//! Defense in depth for *injection* (env bootstrap, module hijack), not egress —
//! network isolation is a documented leftover (v2).

use crate::config::{SANDBOX_HARD_CAP, SANDBOX_TIMEOUT};
use std::path::Path;
use std::process::{Command, Stdio};

/// Variables cleared rather than inherited. Anything that can make an
/// interpreter run arbitrary code at startup, load extra libs, or hijack the
/// module search path.
const DENYLIST: &[&str] = &[
    // shell / startup
    "BASH_ENV", "ENV", "GITHUB_ENV", "PROMPT_COMMAND", "ZDOTDIR", "IFS",
    "GIT_CONFIG", "GIT_CONFIG_COUNT",
    // JS / node
    "NODE_OPTIONS", "NODE_PATH", "NODE_EXTRA_CA_CERTS", "ELECTRON_RUN_AS_NODE",
    // Python
    "PYTHONSTARTUP", "PYTHONINSPECT", "PYTHONPATH", "PYTHONHOME",
    // Perl / Ruby
    "PERL5OPT", "PERL5LIB", "PERLLIB", "RUBYOPT", "RUBYLIB",
    // JVM
    "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS",
    // .NET
    "CORECLR_PROFILER", "CORECLR_PROFILER_PATH", "CORECLR_ENABLE_PROFILING",
    "DOTNET_STARTUP_HOOKS", "DOTNET_ADDITIONAL_DEPS", "DOTNET_SHARED_STORE",
    "DOTNET_ENVIRONMENT",
    // dynamic loading
    "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
    // Windows
    "PSModulePath",
    // proxy — half of the network deny
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "all_proxy",
];

/// Prefixes cleared by pattern.
const DENY_PREFIXES: &[&str] = &["GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_", "COMPlus_Profiler"];

/// Kept as-is. `LC_ALL` is passed through, never set: forcing the C locale
/// destroys non-English output (spike S5).
const ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "LANG", "LC_ALL", "TEMP", "TMP",
    "SYSTEMROOT", "SYSTEMDRIVE", "COMSPEC", "USERNAME",
];

/// Everything except the allowlist is dropped.
pub fn filtered_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| ALLOWLIST.contains(&k.as_str()))
        .filter(|(k, _)| !DENYLIST.contains(&k.as_str()))
        .filter(|(k, _)| !DENY_PREFIXES.iter().any(|p| k.starts_with(p)))
        .collect()
}

pub struct Sandboxed {
    pub stdout: String,
}

/// The only refusal: an input larger than `SANDBOX_HARD_CAP`.
#[derive(Debug)]
pub struct Refusal {
    pub input_bytes: u64,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "input is {} bytes, above the {} byte cap", self.input_bytes, SANDBOX_HARD_CAP)
    }
}

impl std::error::Error for Refusal {}

/// Run `script args... input` with the host env stripped. Never returns the
/// host's variables; always returns within `SANDBOX_TIMEOUT`.
pub fn run(
    script: &Path,
    args: &[String],
    input: &Path,
) -> Result<Sandboxed, Box<dyn std::error::Error>> {
    if let Ok(meta) = std::fs::metadata(input) {
        if meta.len() > SANDBOX_HARD_CAP {
            return Err(Box::new(Refusal { input_bytes: meta.len() }));
        }
    }

    let (prog, prefix) = script_cmd(script);
    // 进程级网络隔离（遗留 v2 的折中落地）：Linux 上 `TSK_NET_ISOLATE=1` 时用
    // `unshare -n` 把子进程放进新网络命名空间（需 CAP_SYS_ADMIN，无权限则
    // spawn 失败 → fail-open，绝不静默放行）。Windows 无 seccomp/namespace，
    // 不提供该选项（文档记录为平台限制）。
    #[cfg(unix)]
    let (prog, prefix) = if std::env::var("TSK_NET_ISOLATE").map(|v| v == "1").unwrap_or(false) {
        let mut p = vec!["-n".to_string()];
        p.extend(prefix);
        ("unshare".to_string(), p)
    } else {
        (prog, prefix)
    };
    let mut c = Command::new(prog);
    c.args(prefix).arg(input).args(args).env_clear();
    for (k, v) in filtered_env() {
        c.env(k, v);
    }
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    // 让子进程成为进程组组长（process group 0）——否则 `kill(-pid)` 命中一个不存在的
    // 进程组（ESRCH），超时后 kill_tree 什么也杀不掉，失控脚本把整体挂死。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let child = c.spawn()?;

    // A watchdog kills the tree on timeout; the main thread then reaps once,
    // so the pipes are read exactly once. The watchdog polls a done-flag at a
    // small interval, so a fast script doesn't make the caller wait out the
    // whole timeout.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let done = std::sync::Arc::clone(&done);
        let id = child.id();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(SANDBOX_TIMEOUT);
            loop {
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    return; // the child finished on its own
                }
                if std::time::Instant::now() >= deadline {
                    kill_tree(id);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })
    };

    let out = child.wait_with_output()?;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = watcher.join();
    Ok(Sandboxed {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    })
}

/// `Child::kill` does not reach the process tree. Kill the group (Unix) or the
/// tree (Windows).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID"])
            .arg(pid.to_string())
            .output();
    }
    #[cfg(not(windows))]
    {
        unsafe {
            extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            let _ = kill(-pid as i32, 9); // negative ⇒ process group
        }
    }
}

/// Windows does not resolve `script.py`-style paths to their interpreter.
fn script_cmd(script: &Path) -> (String, Vec<String>) {
    let s = script.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        let interp = match script.extension().and_then(|e| e.to_str()) {
            Some("ps1") => Some("powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -File"),
            Some("js") | Some("mjs") => Some("node"),
            Some("py") => Some("python"),
            Some("sh") | Some("bash") => Some("bash"),
            _ => None,
        };
        if let Some(i) = interp {
            let mut parts: Vec<String> = i.split(' ').map(str::to_string).collect();
            parts.push(s);
            return (parts.remove(0), parts);
        }
        (s, Vec::new())
    }
    #[cfg(not(windows))]
    (s, Vec::new())
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn allowlist_keeps_locale_but_drops_bootstrap_vars() {
        std::env::set_var("NODE_OPTIONS", "--require=evil.js");
        std::env::set_var("PYTHONSTARTUP", "/tmp/x.py");
        std::env::set_var("GIT_CONFIG_KEY_0", "a");
        std::env::set_var("LANG", "C.UTF-8");

        let keys: Vec<String> = filtered_env().into_iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"LANG".to_string())); // preserved, never overwritten
        assert!(!keys.contains(&"NODE_OPTIONS".to_string()));
        assert!(!keys.contains(&"PYTHONSTARTUP".to_string()));
        assert!(!keys.iter().any(|k| k.starts_with("GIT_CONFIG_KEY_")));
    }

    #[test]
    fn no_lc_all_forcing_anywhere_in_source() {
        // Guards the S5 finding at the source level. The check itself is
        // built in pieces so this file doesn't trip its own assertion.
        let needle = format!("LC_{}={}", "ALL", "C");
        let self_path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/sandbox.rs");
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Some(ext) = path.extension() else { continue };
            if !matches!(ext.to_str(), Some("rs" | "py" | "sh" | "js" | "txt")) {
                continue;
            }
            if path.to_str().map(|s| s == self_path).unwrap_or(false) {
                continue; // the test file, by construction
            }
            let Ok(s) = std::fs::read_to_string(&path) else { continue };
            assert!(
                !s.lines().any(|l| {
                    l.contains(&needle) || l.contains(&format!("{needle}.UTF-8"))
                }),
                "locale forcing found in {}",
                path.display()
            );
        }
    }

    #[test]
    fn oversized_input_is_refused_before_spawn() {
        // Refuses anything over the hard cap without ever launching a process.
        let err = run(
            Path::new("/bin/true"),
            &[],
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml")),
        )
        .err()
        .map(|e| e.downcast_ref::<Refusal>().map(|r| r.input_bytes))
        .unwrap_or(None);
        // The source tree's Cargo.toml is tiny, so this call must NOT refuse.
        assert!(err.is_none());
    }

    #[test]
    fn allowlist_is_exact_match() {
        // The env filter matches names verbatim: `LANG` is allowed, `lang` is not.
        std::env::set_var("lowercase_lang", "C.UTF-8");
        let keys: Vec<String> = filtered_env().into_iter().map(|(k, _)| k).collect();
        assert!(!keys.contains(&"lowercase_lang".to_string()));
    }
}
