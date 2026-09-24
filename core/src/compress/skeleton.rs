//! Skeleton compression: keep the head and tail of a file, replace the middle with
//! a marker that tells the model how to retrieve the omitted lines. See spec §6.1.
//! Pure function — the same input always produces the same bytes (iron law 2).

use crate::config::{MIN_LINES, SKELETON_HEAD, SKELETON_TAIL};

/// `lines` is the original text split into lines.
pub fn skeleton(text: &str, orig_rel: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').map(strip_nl).collect();
    let n = lines.len();
    if n < MIN_LINES || SKELETON_HEAD + SKELETON_TAIL >= n {
        return None;
    }

    let mut out = String::new();
    write_head(&mut out, orig_rel, n);
    for i in 0..SKELETON_HEAD {
        emit_line(&mut out, i + 1, lines[i]);
    }

    // `omitted` spans lines HEAD+1 ..= N-TAIL (1-based, inclusive).
    let omit_start = SKELETON_HEAD + 1;
    let omit_end = n - SKELETON_TAIL;
    out.push_str(&format!(
        "[TSK omitted lines {omit_start}..{omit_end}. Full file: Read {orig_rel} with offset/limit, \
         or Grep <pattern> {orig_rel}]\n"
    ));

    // Tail: last SKELETON_TAIL lines, in original order.
    for i in (n - SKELETON_TAIL)..n {
        emit_line(&mut out, i + 1, lines[i]);
    }
    Some(out)
}

fn write_head(out: &mut String, orig_rel: &str, n: usize) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "[TSK SKELETON of {orig_rel} | {n} lines total | head {SKELETON_HEAD} + tail {SKELETON_TAIL} \
         | TSK v1]\n"
    );
}

/// 6-column right-aligned number, TAB, original text.
fn emit_line(out: &mut String, num: usize, line: &str) {
    use std::fmt::Write;
    let _ = write!(out, "{num:>6}\t{line}");
    if !line.ends_with('\n') {
        out.push('\n');
    }
}

/// Strip one line ending. Order matters: `\r\n` must drop `\n` first, then `\r`
/// (a `\r`-only suffix check on `"…\r\n"` would leave the `\r` behind).
fn strip_nl(s: &str) -> &str {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.strip_suffix('\r').unwrap_or(s)
}

#[cfg(test)]
mod t {
    use super::*;

    fn text(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn small_files_pass_through() {
        assert!(skeleton(&text(7), "a.txt").is_none());
        assert!(skeleton(&text(8), "a.txt").is_none()); // head 5 + tail 3 would touch
    }

    #[test]
    fn exact_shape() {
        let got = skeleton(&text(10), "a.txt").unwrap();
        // Numbers right-aligned to 6 columns, then TAB, then the original line.
        let expected = [
            "[TSK SKELETON of a.txt | 10 lines total | head 5 + tail 3 | TSK v1]",
            "     1\tline 1",
            "     2\tline 2",
            "     3\tline 3",
            "     4\tline 4",
            "     5\tline 5",
            "[TSK omitted lines 6..7. Full file: Read a.txt with offset/limit, or Grep <pattern> a.txt]",
            "     8\tline 8",
            "     9\tline 9",
            "    10\tline 10",
        ]
        .join("\n")
        + "\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn pure() {
        let a = skeleton(&text(50), "a.txt");
        let b = skeleton(&text(50), "a.txt");
        assert_eq!(a, b); // iron law 2
    }

    #[test]
    fn marker_points_at_offset_limit_escape() {
        let s = skeleton(&text(100), "data/big.log").unwrap();
        assert!(s.contains("Read data/big.log with offset/limit"));
        assert!(s.contains("[TSK omitted lines 6..97."));
    }

    #[test]
    fn preserves_non_ascii_verbatim() {
        let t = "第一行\n第二行\n第三行\n第四行\n第五行\n第六行\n第七行\n第八行\n第九行\n第十行\n";
        let s = skeleton(t, "中.txt").unwrap();
        assert!(s.contains("第一行"));
        assert!(s.contains("第十行"));
    }
}
