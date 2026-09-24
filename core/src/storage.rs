//! Layout of the two storage trees. See spec §4.

// ~/.tsk/            config.yaml + per-session machine-private data
// <project>/.tsk/    skeletons/ (project artifacts, gitignorable)

use sha1::Digest as _;
use std::path::{Path, PathBuf};

/// `~/.tsk` — overridable via `TSK_HOME` so tests run hermetic.
pub fn tsk_home() -> PathBuf {
    std::env::var("TSK_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        home().join(".tsk")
    })
}

/// `HOME`, then `USERPROFILE` on Windows.
fn home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// `<tsk_home>/<session_id>` — ledger, snapshot, events.log, ext/.
pub fn session_dir(session_id: &str) -> PathBuf {
    tsk_home().join(session_id)
}

/// File recording the most recent session — so `tsk report` (no args) can default
/// to the *current* session rather than scanning everything.
pub fn current_session_file() -> PathBuf {
    tsk_home().join("current-session")
}

pub fn write_current_session(session: &str) {
    ensure_dir(&tsk_home());
    if !session.is_empty() {
        let _ = std::fs::write(current_session_file(), session);
    }
}

pub fn read_current_session() -> Option<String> {
    std::fs::read_to_string(current_session_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Externalized large outputs land here.
pub fn ext_dir(session_id: &str) -> PathBuf {
    session_dir(session_id).join("ext")
}

/// `<project>/.tsk/skeletons`.
pub fn skeleton_dir(project: &Path) -> PathBuf {
    project.join(".tsk").join("skeletons")
}

/// Canonical path form: forward slashes, no trailing slash, no `.` segments.
/// Feeds the sha1 that names a skeleton — must be stable across platforms.
pub fn canonical_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.trim_end_matches('/').to_string()
}

/// Deterministic skeleton path: `<project>/.tsk/skeletons/<sha1>-<basename>.skeleton.txt`.
/// The same original path always yields the same skeleton (recall anchor).
pub fn derive_skeleton_path(project: &Path, orig: &Path) -> PathBuf {
    let h = sha1_hex(canonical_path(orig).as_bytes());
    let base = orig
        .file_name()
        .map(|b| b.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Owned("unnamed".to_string()));
    skeleton_dir(project).join(format!("{h}-{base}.skeleton.txt"))
}

/// `sha1(bytes)` as lowercase hex.
pub fn sha1_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(40);
    for b in sha1::Sha1::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Create `dir` (and parents) if absent. Never fails the caller's path.
pub fn ensure_dir(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
}

/// Create the parent of `path`, if any. Used when writing a config file.
pub fn ensure_parent_dir(path: &Path) -> bool {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => {
            ensure_dir(p);
            true
        }
        _ => false,
    }
}
