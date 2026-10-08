//! Output-compaction engine (rtk 式逐命令语义压缩)。
//! 收益轴核心：把工具大输出按「命令类型」压成符合工具短结果格式的紧凑文本。
//! 由 `tsk compress` 暴露；deveco 插件经 spawn 调用（薄桥），也是共享核心的一部分。
//! 有损但保格式；无损靠调用方把 raw 落盘 KB 保回取。

use regex::Regex;

pub const CAP_LIST: usize = 20;
pub const CAP_ERR: usize = 20;

/// 命令名是否出现（cx=命令首词边界；`(^|[\\s/:])cmd( ?|$)`）。
fn is(cmd: &str, c: &str) -> bool {
    let pat = format!(r"(^|[\s/:]){}( ?|$)", regex::escape(c));
    Regex::new(&pat).map(|re| re.is_match(cmd)).unwrap_or(false)
}

fn lines_of(raw: &str) -> Vec<String> {
    raw.split('\n').map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect()
}
fn head(a: &[String], n: usize) -> String {
    let shown = a.iter().take(n).cloned().collect::<Vec<_>>().join("\n");
    if a.len() > n { format!("{shown}\n… +{} more", a.len() - n) } else { shown }
}
fn common_prefix(a: &[String]) -> String {
    if a.is_empty() { return String::new() }
    let mut p: String = a[0].clone();
    for s in a { while !s.starts_with(&p) { p.pop(); } }
    p
}

/// 按命令类型压缩（未识别返回 None → 调用方回退通用蒸馏）。输出格式与插件 v 版本一致。
pub fn compact_by_command(cmd: &str, raw: &str) -> Option<String> {
    let lines = lines_of(raw);
    // ---- ls / tree ----
    if is(cmd, "ls") || is(cmd, "tree") {
        // 过滤: total 行 / `.` `..` 噪音 / `./xxx:` 递归目录标题行(目录名已在项列表里)
        let names: Vec<String> = lines.iter()
            .filter(|l| !l.starts_with("total "))
            .filter(|l| !l.trim().starts_with("./") || !l.trim().ends_with(':'))
            .map(|l| {
                if Regex::new(r"^[bcdlps-][rwx-]{9}").unwrap().is_match(l) {
                    // 目录项(drwx…)带斜杠后缀便于计数；普通文件取最后字段
                    let last = l.split_whitespace().last().unwrap_or_else(|| l.as_str());
                    if l.starts_with('d') { format!("{last}/") } else { last.to_string() }
                } else { l.to_string() }
            })
            // `.` `..` 在 ls -laR 里是权限行(drwx… .)，取名字后变成 `.`/`../` → 再滤一次
            .filter(|n| n.trim().trim_end_matches('/') != "." && n.trim().trim_end_matches('/') != "..")
            .collect();
        let dirs = names.iter().filter(|n| n.ends_with('/')).count();
        let files = names.len() - dirs;
        let shown = names.iter().take(CAP_LIST).cloned().collect::<Vec<_>>().join("\n");
        let more = if names.len() > CAP_LIST { format!("\n… +{} more", names.len() - CAP_LIST) } else { String::new() };
        let dd = if dirs == 1 { "" } else { "s" };
        let ff = if files == 1 { "" } else { "s" };
        let text = format!("{dirs} dir{dd}, {files} file{ff}:\n{shown}{more}");
        return Some(text);
    }
    // ---- find ----
    if is(cmd, "find") {
        let paths: Vec<&str> = lines.iter().map(|l| l.as_str()).filter(|l| !l.contains("Permission denied") && !l.starts_with("find:")).collect();
        let files2: Vec<String> = paths.iter().map(|l| l.trim().trim_end_matches('/').to_string()).collect();
        let dirs = paths.iter().filter(|l| l.ends_with('/')).count();
        let h = head(&files2, CAP_LIST);
        return Some(format!("{dirs} dirs, {} files:\n{h}{}", files2.len() - dirs, if files2.len() > CAP_LIST { format!("\n… +{} more paths", files2.len() - CAP_LIST) } else { String::new() }));
    }
    // ---- git status ----
    if is(cmd, "git") && cmd.contains("status") {
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        let re = Regex::new(r"^([MADRCU?! ]{2}|[ MADRCU?]) ?(.*)$").unwrap();
        for l in &lines {
            if let Some(c) = re.captures(l) {
                let st = c[1].replace(' ', "");
                let st = if st.is_empty() { "?".to_string() } else { st };
                groups.entry(st).or_default().push(c[2].trim().to_string());
            }
        }
        if groups.is_empty() {
            return Some(raw.trim().chars().take(800).collect());
        }
        let rows: Vec<String> = groups.iter().map(|(st, fs_)| format!("{st} ({}):\n{}", fs_.len(), head(fs_, CAP_LIST))).collect();
        return Some(format!("git status ({} 行)：\n{}", lines.len().saturating_sub(1).max(1), rows.join("\n")));
    }
    // ---- git log / show ----
    if is(cmd, "git") && (cmd.contains("log") || cmd.contains("show")) {
        let re = Regex::new(r"(?i)^commit ([0-9a-f]{7,40})").unwrap();
        let mut commits = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if let Some(c) = re.captures(l) {
                let mut subject = String::new();
                for j in (i + 1)..(i + 13).min(lines.len()) {
                    let t = lines[j].trim();
                    if t.is_empty() { continue }
                    if t.starts_with("Author") || t.starts_with("Date") || t.starts_with("Merge") || t.starts_with("Signed-off") || t.starts_with("commit ") { continue }
                    subject = if lines[j].starts_with("    ") { t.to_string() } else { t.to_string() };
                    break;
                }
                commits.push(format!("{} {}", &c[1][..c[1].len().min(12)], subject));
            }
        }
        if !commits.is_empty() {
            return Some(format!("{} commits:\n{}", commits.len(), head(&commits, CAP_LIST)));
        }
    }
    // ---- git diff / show ----
    if is(cmd, "git") && (cmd.contains("diff") || cmd.contains("show")) {
        let freq = Regex::new(r"^diff --git a/(\S+)(?: b/(\S+))?$").unwrap();
        let hunk = Regex::new(r"^@@ -(\d+(?:,\d+)?) \+(\d+(?:,\d+)?) @@(.*)$").unwrap();
        let mut files = Vec::new();
        let mut hunks = Vec::new();
        for l in &lines {
            if let Some(c) = freq.captures(l) {
                let a = c[1].to_string();
                let b = c.get(2).map(|m| m.as_str().to_string());
                if let Some(bv) = b { if bv != a { files.push(format!("{a} → {bv}")); continue } }
                files.push(a);
            } else if let Some(c) = hunk.captures(l) {
                let tail = c.get(3).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                hunks.push(format!("@@ -{} +{} @@{}", &c[1], &c[2], if tail.is_empty() { String::new() } else { format!(" {tail}") }));
            }
        }
        let mut parts = Vec::new();
        if !files.is_empty() { parts.push(format!("{} files:\n{}", files.len(), head(&files, CAP_LIST))) }
        if !hunks.is_empty() { parts.push(format!("{} hunks:\n{}", hunks.len(), hunks.iter().take(CAP_ERR).cloned().collect::<Vec<_>>().join("\n"))) }
        if !parts.is_empty() { return Some(parts.join("\n\n")) }
    }
    // ---- grep / rg：按文件分组 ----
    if is(cmd, "grep") || is(cmd, "rg") {
        let re = Regex::new(r"^([^:]+):(\d+):(.*)$").unwrap();
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        let mut nmatches = 0usize;
        for l in &lines {
            if let Some(c) = re.captures(l) {
                groups.entry(c[1].to_string()).or_default().push(format!("{}: {}", &c[2], &c[3]));
                nmatches += 1;
            } else {
                groups.entry("(match)".to_string()).or_default().push(l.to_string());
                nmatches += 1;
            }
        }
        if !groups.is_empty() {
            let mut parts = Vec::new();
            let mut shown_files = 0usize;
            for (f, rows) in groups.iter().take(CAP_LIST) {
                let shown_rows: Vec<String> = rows.iter().take(5).map(|r| format!("  {r}")).collect();
                let extra = if rows.len() > 5 { format!("\n  … +{} rows", rows.len() - 5) } else { String::new() };
                parts.push(format!("{f} ({}):\n{}{}", rows.len(), shown_rows.join("\n"), extra));
                shown_files += 1;
            }
            if groups.len() > shown_files { parts.push(format!("… +{} files", groups.len() - shown_files)) }
            return Some(format!("grep ({} files, {nmatches} matches):\n{}", groups.len(), parts.join("\n")));
        }
    }
    // ---- wc：剥公共前缀 ----
    if is(cmd, "wc") {
        let re = Regex::new(r"\s+").unwrap();
        let rows: Vec<Vec<String>> = lines.iter().map(|l| re.split(l.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect::<Vec<String>>()).filter(|r| r.len() >= 2).collect();
        let files_r: Vec<&Vec<String>> = rows.iter().filter(|r| r.last().map(|s| s.as_str()) != Some("total")).collect();
        if !rows.is_empty() && !files_r.is_empty() {
            let paths: Vec<String> = files_r.iter().map(|r| r.last().unwrap().clone()).collect();
            let prefix = if paths.len() > 1 { common_prefix(&paths) } else { String::new() };
            let shown: Vec<String> = files_r.iter().map(|r| format!("{} {}", r[..r.len() - 1].join(" "), r.last().unwrap()[prefix.len()..].to_string())).collect();
            let mut out = format!("wc ({} 项，公共前缀\"{}\"已剥)：\n{}", files_r.len(), prefix, show(&shown, CAP_LIST));
            if let Some(t) = rows.iter().find(|r| r.last().map(|s| s.as_str()) == Some("total")) { out = format!("{out}\n{}", t.join(" ")) }
            return Some(out);
        }
    }
    // ---- du ----
    if is(cmd, "du") {
        let total = lines.iter().find(|l| l.ends_with("total")).map(|l| l.trim().to_string());
        let rows: Vec<String> = lines.iter().filter(|l| !l.ends_with("total")).map(|l| l.to_string()).collect();
        if !rows.is_empty() { return Some(format!("du ({} 项):\n{}{}", rows.len(), head(&rows, CAP_LIST), total.map(|t| format!("\ntotal: {t}")).unwrap_or_default())) }
    }
    // ---- df ----
    if is(cmd, "df") {
        if !lines.is_empty() {
            let rows: Vec<String> = lines[1..].iter().map(|l| l.to_string()).collect();
            let extra = if lines.len() - 1 > CAP_LIST { format!("\n… +{} more", lines.len() - 1 - CAP_LIST) } else { String::new() };
            return Some(format!("df:\n{}\n{}{}", lines[0], head(&rows, CAP_LIST), extra));
        }
    }
    // ---- ps ----
    if is(cmd, "ps") {
        let hi = lines.iter().position(|l| l.starts_with("PID") || l.starts_with("UID") || l.starts_with("USER"));
        let header = hi.map(|i| lines[i].clone()).unwrap_or_else(|| lines.first().map(|s| s.to_string()).unwrap_or_default());
        let rows: Vec<String> = (hi.map(|i| &lines[i + 1..]).unwrap_or_else(|| &lines[1..])).iter().map(|l| l.to_string()).collect();
        return Some(format!("ps:\n{header}\n{}{}", head(&rows, CAP_LIST), if rows.len() > CAP_LIST { format!("\n… +{} more", rows.len() - CAP_LIST) } else { String::new() }));
    }
    // ---- curl / wget：结尾状态/速率/错误 ----
    if is(cmd, "curl") || is(cmd, "wget") {
        let keep: Vec<String> = lines.iter().filter(|l| Regex::new(r"(?i)HTTP/|error|failed|cannot|resolve|100%|Saved to").unwrap().is_match(l)).map(|l| l.to_string()).collect();
        if !keep.is_empty() { return Some(keep.iter().rev().take(8).cloned().collect::<Vec<_>>().iter().rev().cloned().collect::<Vec<_>>().join("\n")) }
    }
    // ---- 测试/构建 ----
    if ["vitest", "jest", "mocha", "phpunit", "pytest", "ctest", "golangci"].iter().any(|s| cmd.contains(s))
        || (cmd.contains("test") && Regex::new(r"(?i)(pass|fail|✓|✗|✖|Tests:|Suites:)|failed").unwrap().is_match(raw)) {
        let summary: Vec<String> = lines.iter().filter(|l| Regex::new(r"(?i)(Tests:|test suites|passed|failed|✓ \d|✗ \d)").unwrap().is_match(l)).take(4).map(|l| l.to_string()).collect();
        let fails: Vec<String> = lines.iter().filter(|l| Regex::new(r"(?i)✖|✗|×|FAIL|failed|Error:|\berror\b").unwrap().is_match(l)).take(CAP_ERR).map(|l| l.to_string()).collect();
        let mut parts = Vec::new();
        if !summary.is_empty() { parts.push(summary.join("\n")) }
        if !fails.is_empty() { parts.push(format!("failures ({} shown):\n{}", fails.len(), fails.join("\n"))) }
        if !parts.is_empty() { return Some(parts.join("\n\n")) }
    }
    None
}
fn show(a: &[String], n: usize) -> String {
    let shown = a.iter().take(n).cloned().collect::<Vec<_>>().join("\n");
    if a.len() > n { format!("{shown}\n… +{} more", a.len() - n) } else { shown }
}

#[cfg(test)]
mod t {
    use super::*;
    fn mk(n: usize, f: &dyn Fn(usize) -> String) -> String { (0..n).map(|i| f(i)).collect::<Vec<_>>().join("\n") }
    #[test]
    fn ls_counts_and_caps() {
        let raw = mk(60, &|i| format!("-rw-r--r-- u g {} Jan 1 00:0{} file{}.ts", 100 + i, i % 10, i));
        let out = compact_by_command("ls -la", &raw).unwrap();
        assert!(out.contains("0 dirs, 60 files:"), "got: {out}");
        assert!(out.contains("file0.ts") && out.contains("+40 more"), "got: {out}");
    }
    #[test]
    fn git_status_groups() {
        let raw = mk(30, &|i| if i % 2 == 0 { format!("M  mod{i}.ts") } else { format!("?? new{i}.js") });
        let out = compact_by_command("git status --short", &raw).unwrap();
        assert!(out.contains("?? (15):") && out.contains("M (15):"), "got: {out}");
    }
    #[test]
    fn git_log_oneline() {
        let raw = mk(5, &|i| format!("commit deadbeef000{i}\nAuthor: u\n\n    fix patch {i}\n\n body"), );
        let out = compact_by_command("git log", &raw).unwrap();
        assert!(out.contains("5 commits:") && out.contains("fix patch 0"), "got: {out}");
        assert!(!out.contains("body line"), "got: {out}");
    }
    #[test]
    fn wc_strips_common_prefix() {
        let raw = mk(5, &|i| format!("1 2 3 common/dir/{}a.ts", "abcde".chars().nth(i).unwrap()));
        let out = compact_by_command("wc -l *.ts", &raw).unwrap();
        assert!(out.contains("公共前缀\"common/dir/\"") && !out.contains("common/dir/a.ts") && out.contains("a.ts"), "got: {out}");
    }
    #[test]
    fn test_report_shaped() {
        let raw = format!("RUN\nt... Tests 0 passed | 40 failed (40)\n{}", mk(40, &|i| format!("✗ t{i}.test.ts > case\n  AssertionError")));
        let out = compact_by_command("npx vitest run", &raw).unwrap();
        assert!(out.contains("Tests") && out.contains("failures (20 shown):"), "got: {out}");
    }
}