//! Files larger than `EXTERNALIZE_THRESHOLD` are replaced by a pointer file plus a
//! short excerpt; the full bytes go to `~/.tsk/<session>/ext/<sha1>.txt`. See spec §6.2.

use crate::storage;
use std::path::PathBuf;

/// `pointer` is what the `Read` is redirected to; `full` holds the whole original.
pub struct Externalized {
    pub pointer: PathBuf,
    /// The full original text, externalized to `ext/<sha1>.txt`. The pointer
    /// names this exact file so a later `Read` can retrieve the whole content.
    pub full: PathBuf,
    /// Bytes of the original — what was kept out of context.
    pub bytes: usize,
    /// Size of the pointer file: what actually entered the context.
    pub pointer_bytes: usize,
}

pub fn externalize(session_id: &str, content: &str) -> Option<Externalized> {
    let bytes = content.len();
    let h = storage::sha1_hex(content.as_bytes());

    let dir = storage::ext_dir(session_id);
    storage::ensure_dir(&dir);
    let full = dir.join(format!("{h}.txt"));
    if std::fs::write(&full, content).is_err() {
        return None;
    }

    let excerpt = head(content, 1024);
    let pointer = dir.join(format!("{h}.pointer.txt"));
    let ptr_text = format!(
        "[TSK EXTERNALIZED output of Read | original {bytes} bytes | full: {}]\n\
         [TSK excerpt: head 1KB follows; retrieve full content via Read on the path above]\n{excerpt}",
        full.display()
    );
    if std::fs::write(&pointer, &ptr_text).is_err() {
        return None;
    }
    Some(Externalized {
        pointer,
        full,
        bytes,
        pointer_bytes: ptr_text.len(),
    })
}

/// First `n` bytes, cut on a char boundary, at the last newline before it.
fn head(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut cut = n;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let line_end = s[..cut].rfind('\n').map(|i| i + 1).unwrap_or(cut);
    &s[..line_end]
}

#[cfg(test)]
mod t {
    use super::*;
    use std::path::PathBuf;

    /// Point storage at a throwaway dir so these tests never touch a real `~/.tsk`.
    /// Returns a guard that deletes the dir on drop.
    struct Hermetic(PathBuf);

    impl Hermetic {
        fn new() -> Self {
            let d = PathBuf::from(format!("/tmp/tsk-t-{}-{}", std::process::id(), uuid()));
            let _ = std::fs::create_dir_all(&d);
            std::env::set_var("TSK_HOME", &d);
            Self(d)
        }
    }

    impl Drop for Hermetic {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Test-case index — one per test that has run so far in this process.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn uuid() -> usize {
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn head_is_utf8_safe_and_line_bounded() {
        assert_eq!(head("a".repeat(2000).as_str(), 1024).len(), 1024);

        let lines: String = (0..200).map(|i| format!("{i}\n")).collect();
        assert!(head(&lines, 50).ends_with('\n'));
    }

    #[test]
    fn excerpt_never_drops_the_rest() {
        // Cutting must not silently discard content: the pointer always names
        // the exact full file, which exists and is readable.
        let _d = Hermetic::new();
        let s = "x".repeat(3000);
        let got = externalize("sess-t", &s).unwrap();
        assert_eq!(got.bytes, 3000);
        assert!(got.full.exists(), "full 文件必须落盘");
        let ptr = std::fs::read_to_string(&got.pointer).unwrap();
        assert!(ptr.starts_with("[TSK EXTERNALIZED output of Read | original 3000 bytes"));
        assert!(ptr.contains(&format!("full: {}", got.full.display())), "指针须指向确切路径");
        // pointer 以 1KB 为界，行尾截断。
        assert!(ptr.contains("[TSK excerpt: head 1KB follows"));
    }
}
