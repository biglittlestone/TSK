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
fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|[\x1b][()][0-9;]*[A-Za-z]|\x1b\]0;[^\x07]*\x07").unwrap();
    re.replace_all(s, "").to_string()
}

mod log_template;
use log_template::log_template;

/// 按命令类型压缩。返回 `None` = 放行原文，不做丢信息压缩（通用蒸馏已移除）。
///
/// 唯一**按输出内容**判定的例外（不依赖命令名）：测试/构建日志形态
/// （输出含 `BUILD SUCCESSFUL/FAILED`、Tests/failed/✗ 汇总行）——这类输出被截断
/// 的部分天然是进度噪音，结论行+失败行（上限 CAP_ERR）+ KB 无损回取足以端到端作答。
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
                // 长命中行截断：行号保留用于定位，完整内容模型可 read/章节地图按需取回（降低回读）
                let content = c[3].to_string();
                let content = if content.chars().count() > 180 { format!("{}…", content.chars().take(180).collect::<String>()) } else { content };
                groups.entry(c[1].to_string()).or_default().push(format!("{}: {}", &c[2], content));
                nmatches += 1;
            } else {
                let content = if l.chars().count() > 180 { format!("{}…", l.chars().take(180).collect::<String>()) } else { l.to_string() };
                groups.entry("(match)".to_string()).or_default().push(content);
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
            let mut out = format!("wc ({} 项，公共前缀\"{}\"已剥)：\n{}", files_r.len(), prefix, head(&shown, CAP_LIST));
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
    // ---- devecocli docs：ArkUI API 文档 → 结构目录（标题/接口签名/参数名:类型） ----
    // 文档正文/示例代码全丢，只留组件结构骨架——模型看目录即知有哪些接口/参数，细节按节 search 取回（0 回读常规问题）。
    if is(cmd, "devecocli") && cmd.contains("docs") {
        let re_title = Regex::new(r"^\s*(#{1,6})\s+\S").unwrap();
        let re_sig = Regex::new(r"^[\w.]+\s*\(.*\)\s*$").unwrap();
        let re_key = Regex::new(r"^\*\*[^*]+\*\*：").unwrap();
        let mut keep: Vec<String> = Vec::new();
        let mut in_code = false;
        let mut n_titles = 0;
        let mut n_sigs = 0;
        let mut n_params = 0usize;
        for l in &lines {
            let t = l.trim();
            if t.starts_with("```") { in_code = !in_code; continue }
            if in_code { continue }
            if t.starts_with("...output truncated") || t.starts_with("Full output saved") { continue }
            if t.starts_with('|') {
                // markdown 表格内 `\|` 是转义竖线（类型 union），先还原再按 `|` 分列
                let cols: Vec<String> = t.replace("\\|", "|").split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                if cols.len() < 2 { continue }
                let a = cols[0].clone();
                // 表头行（第一列是列标签）与 `| --- |` 分隔行跳过
                if a == "参数名" || a == "名称" || a.starts_with("---") { continue }
                // 去掉最后一列（说明/正文），其余列拼成「参数名: 类型[| 更多类型][| 约束]」
                let joined = cols[1..cols.len().saturating_sub(1)].join(" | ");
                let btrunc = if joined.chars().count() > 80 { format!("{}…", joined.chars().take(80).collect::<String>()) } else { joined };
                n_params += 1;
                if n_params > CAP_LIST { continue } // 只显示前 N 参数，计数保留
                keep.push(format!("{a}: {btrunc}"));
                continue
            }
            if re_title.is_match(t) { n_titles += 1; keep.push(t.to_string()); continue }
            if re_sig.is_match(t) { n_sigs += 1; keep.push(t.to_string()); continue }
            if re_key.is_match(t) {
                let trunc = t.chars().take(60).collect::<String>();
                keep.push(if t.chars().count() > 60 { format!("{trunc}…") } else { t.to_string() });
            }
            // 其余 = 正文段落/说明 → 丢弃
        }
        if !keep.is_empty() {
            let shown = keep.iter().take(60).cloned().collect::<Vec<_>>().join("\n");
            let more = if keep.len() > 60 { format!("\n… +{} more", keep.len() - 60) } else { String::new() };
            return Some(format!("devecocli docs ({n_titles} 标题, {n_sigs} 接口, {n_params} 参数)：\n{shown}{more}"));
        }
    }
    // ---- PowerShell（Windows）：gci / gci -r / gps / sls / measure ----
    // 输出为「表格 + ---- 分隔线 + 数据行」，命令名大小写不敏感。未列出的 PS 命令放行。
    {
        let l = cmd.to_lowercase();
        let first = l.split_whitespace().next().unwrap_or("").to_string();
        let is_ps = ["gci", "get-childitem", "gps", "get-process", "sls", "select-string", "measure-object", "measure", "dir"]
            .iter().any(|c| c == &first)
            || l.starts_with("get-") || l.starts_with("powershell") || l.starts_with("pwsh");
        if is_ps {
            // 1) Measure-Object：保留有值的统计行
            if l.contains("measure-object") || first == "measure" {
                let keep: Vec<String> = lines.iter()
                    .filter(|x| Regex::new(r"^\s*(Count|Sum|Average|Maximum|Minimum)\s*:\s*\S").unwrap().is_match(x))
                    .map(|x| x.trim().to_string()).collect();
                if !keep.is_empty() { return Some(format!("Measure-Object:\n{}", head(&keep, CAP_LIST))) }
            }
            // 2) Get-Process / gps / ps：表头 + 前 N 数据行（跳分隔线）
            if l.contains("get-process") || first == "gps" || first == "ps" {
                let hi = lines.iter().position(|x| x.trim().starts_with("Id") || x.contains("ProcessName"));
                let header = hi.map(|i| lines[i].clone()).unwrap_or_else(|| "PID ProcessName".to_string());
                let rows: Vec<String> = (hi.map(|i| &lines[i + 1..]).unwrap_or(&lines[..]))
                    .iter().filter(|x| !x.trim().starts_with('-') && !x.trim().is_empty())
                    .map(|x| x.to_string()).collect();
                let more = if rows.len() > CAP_LIST { format!("\n… +{} more", rows.len() - CAP_LIST) } else { String::new() };
                return Some(format!("ps:\n{header}\n{}{}", head(&rows, CAP_LIST), more));
            }
            // 3) Get-ChildItem -Recurse：全路径列表（find 语义）
            if l.contains("recurse") {
                let paths: Vec<String> = lines.iter()
                    .filter(|x| {
                        let t = x.trim();
                        !t.is_empty() && !t.starts_with('-') && !t.starts_with("FullName")
                            && t.contains('\\') && t.starts_with('C')
                    })
                    .map(|x| x.trim().to_string())
                    .collect();
                if paths.len() > 1 {
                    let h = head(&paths, CAP_LIST);
                    let more = if paths.len() > CAP_LIST { format!("\n… +{} more paths", paths.len() - CAP_LIST) } else { String::new() };
                    return Some(format!("{} files:\n{h}{more}", paths.len()));
                }
            }
            // 4) Get-ChildItem / gci / dir：ls 语义（只保留 Mode 数据行，跳表头/分隔线/目录标题）
            if l.contains("get-childitem") || first == "gci" || first == "dir" {
                let names: Vec<String> = lines.iter()
                    .filter_map(|x| {
                        let t = x.trim();
                        if Regex::new(r"^[d-][-arhsx]{5}").unwrap().is_match(t) {
                            let last = t.split_whitespace().last().unwrap_or(t).to_string();
                            Some(if t.starts_with('d') { format!("{last}/") } else { last })
                        } else { None }
                    })
                    .collect();
                let dirs = names.iter().filter(|n| n.ends_with('/')).count();
                let files = names.len() - dirs;
                if names.len() > 1 || dirs > 0 {
                    let shown = names.iter().take(CAP_LIST).cloned().collect::<Vec<_>>().join("\n");
                    let more = if names.len() > CAP_LIST { format!("\n… +{} more", names.len() - CAP_LIST) } else { String::new() };
                    let dd = if dirs == 1 { "" } else { "s" };
                    let ff = if files == 1 { "" } else { "s" };
                    return Some(format!("{dirs} dir{dd}, {files} file{ff}:\n{shown}{more}"));
                }
            }
            // 5) Select-String / sls：grep 语义（行号 + 内容）
            if l.contains("select-string") || first == "sls" {
                let rows: Vec<String> = lines.iter()
                    .filter(|x| !x.trim().is_empty() && !x.trim().starts_with('-') && !x.contains("IgnoreCase"))
                    .map(|x| x.trim().to_string()).collect();
                if !rows.is_empty() {
                    let h = head(&rows, CAP_LIST);
                    let more = if rows.len() > CAP_LIST { format!("\n… +{} more matches", rows.len() - CAP_LIST) } else { String::new() };
                    return Some(format!("grep ({} matches):\n{h}{more}", rows.len()));
                }
            }
            // Get-Content / gc：cat 语义 → 走下方 cat 大输出分支
        }
    }
    // ---- cat / gc（读文件大输出）：日志先 Drain 模板折叠（无损，实验特性）；JSON 提取键；否则保留头尾 ----
    // 尽量降低回读：日志折叠保留全部语义（模板+变体表可重建），JSON 键/头尾给结构；深挖走章节地图。
    if (is(cmd, "cat") || is(cmd, "gc")) && raw.chars().count() > 2000 {
        // 日志类（非 JSON：`{` 对象或 `[{` 数组才算 JSON；`[时间戳]` 是日志）：重复行模板化（Drain，无损）
        let t0 = raw.trim_start();
        if !t0.starts_with('{') && !t0.starts_with("[{") && !t0.starts_with("[ {") {
            let folded = log_template(raw);
            if folded.len() < raw.len() && folded.contains("[Template T") {
                return Some(format!("cat ({} B, 日志模板折叠):\n{}", raw.len(), folded));
            }
        }
        let h = raw.chars().take(400).collect::<String>();
        let t = raw.chars().rev().take(400).collect::<String>().chars().rev().collect::<String>();
        let keyline = if raw.trim_start().starts_with('{') {
            let mut keys: Vec<String> = Regex::new(r#""([A-Za-z0-9_]+)"\s*:"#).unwrap()
                .captures_iter(raw).filter_map(|c| c.get(1).map(|m| m.as_str().to_string())).collect();
            keys.sort(); keys.dedup();
            if keys.is_empty() { String::new() } else { format!("\nJSON 键 ({}): {}", keys.len().min(40), keys.iter().take(40).cloned().collect::<Vec<_>>().join(" ")) }
        } else { String::new() };
        return Some(format!("cat ({} B)：\n头部:\n{h}\n…\n尾部:\n{t}{keyline}", raw.len()));
    }
    // ---- hvigor 构建日志（export … && node … 输出）：保留 BUILD 结论 + 错误行 + 步数 ----
    // 构建结论在尾部（BUILD SUCCESSFUL/FAILED）+ 错误行 → 模型完成构建确认/排查 0 回读；深挖堆栈走章节地图。
    if raw.contains("hvigor") && (raw.contains("BUILD SUCCESSFUL") || raw.contains("BUILD FAILED") || raw.contains("Finished :entry")) {
        let status: Vec<String> = lines.iter()
            .filter(|l| l.contains("hvigor") || l.contains("BUILD ") || l.contains("Finished :entry") || l.contains("UP-TO-DATE"))
            .map(|l| strip_ansi(l)).collect();
        let fails: Vec<String> = lines.iter()
            .filter(|l| {
                let s = strip_ansi(l);
                s.contains("ERROR") || s.contains("FAILED") || s.contains("Exception") || s.contains("error:") || s.contains("WARN:")
            })
            .map(|l| strip_ansi(l)).collect();
        if !status.is_empty() {
            // 构建结论在尾部（BUILD SUCCESSFUL/FAILED）+ 错误行 → 模型完成构建确认/排查 0 回读
            let tail_status: Vec<String> = status.iter().rev().take(6).cloned().collect::<Vec<_>>().iter().rev().cloned().collect();
            let fails_shown = if fails.is_empty() { String::new() } else { format!("\nfailures:\n{}", fails.iter().take(CAP_ERR).cloned().collect::<Vec<_>>().join("\n")) };
            return Some(format!("hvigor build ({} 步, {} 错误):\n{}{}", status.len(), fails.len(), tail_status.join("\n"), fails_shown));
        }
    }
    // ---- 测试/构建 ----
    if ["vitest", "jest", "mocha", "phpunit", "pytest", "ctest", "golangci"].iter().any(|s| cmd.contains(s))
        || (cmd.contains("test") && Regex::new(r"(?i)(pass|fail|✓|✗|✖|Tests:|Suites:)|failed").unwrap().is_match(raw))
        || Regex::new(r"BUILD (SUCCESSFUL|FAILED)").unwrap().is_match(raw) {
        let summary: Vec<String> = lines.iter().filter(|l| Regex::new(r"(?i)(Tests:|test suites|passed|failed|✓ \d|✗ \d|BUILD (SUCCESSFUL|FAILED))").unwrap().is_match(l)).take(4).map(|l| l.to_string()).collect();
        let fails: Vec<String> = lines.iter().filter(|l| Regex::new(r"(?i)✖|✗|×|FAIL|failed|Error:|\berror\b").unwrap().is_match(l)).take(CAP_ERR).map(|l| l.to_string()).collect();
        let mut parts = Vec::new();
        if !summary.is_empty() { parts.push(summary.join("\n")) }
        if !fails.is_empty() { parts.push(format!("failures ({} shown):\n{}", fails.len(), fails.join("\n"))) }
        if !parts.is_empty() { return Some(parts.join("\n\n")) }
    }
    None
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
    fn pwsh_gci_ls_shaped() {
        let raw = "    目录: C:\\Windows\\System32\\drivers\n\nMode  LastWriteTime  Length Name\n----  -------------  ------ ----\nd-----  2026/6/25  9:41         DriverData\n-a----  2026/6/25  9:33  294912 1394ohci.sys\n-a----  2026/6/25  9:30  108464 3ware.sys";
        let out = compact_by_command("Get-ChildItem C:\\Windows", &raw).unwrap();
        assert!(out.contains("1 dir, 2 files:"), "got: {out}");
        assert!(out.contains("DriverData/") && out.contains("1394ohci.sys"), "got: {out}");
        assert!(!out.contains("294912"), "丢时间戳/大小: {out}");
    }
    #[test]
    fn pwsh_measure_shaped() {
        let raw = "Count    : 475\nAverage  :\nSum      : 12345\nMaximum  : 999\nMinimum  :";
        let out = compact_by_command("Measure-Object", &raw).unwrap();
        assert!(out.contains("Count") && out.contains("Sum") && out.contains("Maximum"), "got: {out}");
        assert!(!out.contains("Average"), "无值统计行丢弃: {out}");
    }
    #[test]
    fn pwsh_gps_shaped() {
        let raw = "   Id ProcessName      CPU         WorkingSet64\n   -- -----------      ---         ------------\n 8004 AggregatorHost  1038.953125  15106048\n16364 aha_doctor        47.03125    40550400";
        let out = compact_by_command("Get-Process | Select-Object -First 10", &raw).unwrap();
        assert!(out.contains("ProcessName") && out.contains("AggregatorHost"), "got: {out}");
        assert!(out.contains("aha_doctor"), "got: {out}");
    }
    #[test]
    fn cat_json_keeps_keys() {
        let raw = format!("{{\"apiVersion\":\"24\",\"displayName\":\"Ets\",\"meta\":{{\"metaVersion\":\"3.0.0\"}},\"pad\":\"{}\"}}", "p".repeat(3000));
        let out = compact_by_command("cat oh-units.json", &raw).unwrap();
        assert!(out.contains("JSON 键") && out.contains("apiVersion") && out.contains("displayName"), "got: {out}");
        assert!(out.starts_with("cat ("), "got: {out}");
    }
    #[test]
    fn hvigor_build_keeps_conclusion() {
        let raw = "> hvigor Finished :entry:clean... after 1 ms\n> hvigor Finished :entry:assembleHap... after 1 ms\n> hvigor BUILD FAILED in 2 s\n* Try: Run with --stacktrace\nERROR: Cannot resolve module";
        let out = compact_by_command("export DEVECO_SDK_HOME=x && node build.js", &raw).unwrap();
        assert!(out.contains("BUILD FAILED"), "结论保留: {out}");
        assert!(out.contains("ERROR"), "错误行保留: {out}");
        assert!(out.contains("hvigor build ("), "got: {out}");
    }
    #[test]
    fn npm_build_success_summary() {
        let raw = "step-0000 init\nstep-0001 compile\nBUILD SUCCESSFUL in 3 s 911 ms";
        let out = compact_by_command("npm run build", &raw).unwrap();
        assert!(out.contains("BUILD SUCCESSFUL"), "结论保留: {out}");
        assert!(!out.contains("step-0000"), "进度噪音丢弃: {out}");
    }
    #[test]
    fn npm_build_failed_errors() {
        let raw = "step-0000 compile\nERROR: Cannot resolve module x\nBUILD FAILED in 2 s";
        let out = compact_by_command("npm run build", &raw).unwrap();
        assert!(out.contains("BUILD FAILED") && out.contains("ERROR"), "got: {out}");
    }
    #[test]
    fn log_template_collapses_repeats_lossless() {
        // 常量占比高 + N 大 → 模板头摊销后折叠才省（与 headroom 一致；变体多的短样例会回退）
        let raw = mk(10, &|i| format!("2025-01-15T12:34:{:02} INFO worker-{} processing job {}", 56 + i, i + 1, 42 + i));
        let raw = format!("{raw}\nBUILD DONE");
        let out = log_template(&raw);
        assert!(out.contains("[Template T1:"), "模板头: {out}");
        assert!(out.contains("<*>"), "通配: {out}");
        assert!(out.contains("(10 occurrences)"), "计数: {out}");
        assert!(out.contains("BUILD DONE"), "结论原样: {out}");
        assert!(out.contains("worker-1") && out.contains("worker-10"), "变体表含全部行: {out}");
    }
    #[test]
    fn log_template_never_folds_across_levels() {
        // 级别位不允通配：ERROR 行不得并入 INFO run（否则丢失「几条 ERROR」的分组语义）
        let mut raw = mk(4, &|i| format!("INFO worker-{} processing job {}", i + 1, 42 + i));
        raw.push_str("\nERROR worker-5 processing job 47");
        raw.push_str("\nDONE");
        let out = log_template(&raw);
        assert!(out.contains("[Template T1: INFO <*> processing job <*>]"), "INFO run 折叠: {out}");
        assert!(out.contains("ERROR worker-5 processing job 47"), "ERROR 行原样保留: {out}");
        assert!(out.contains("(4 occurrences)"), "只含 4 条 INFO: {out}");
        assert!(out.contains("DONE"), "结论原样: {out}");
    }
    #[test]
    fn log_template_conservative_when_not_enough_repeats() {
        let raw = "line one\nline two\nline three\nBUILD DONE";
        let out = log_template(raw);
        assert!(!out.contains("[Template"), "少重复不折叠: {out}");
        assert!(out == raw, "原样返回: {out}");
    }
    #[test]
    fn test_report_shaped() {
        let raw = format!("RUN\nt... Tests 0 passed | 40 failed (40)\n{}", mk(40, &|i| format!("✗ t{i}.test.ts > case\n  AssertionError")));
        let out = compact_by_command("npx vitest run", &raw).unwrap();
        assert!(out.contains("Tests") && out.contains("failures (20 shown):"), "got: {out}");
    }
    #[test]
    fn devecocli_docs_keeps_structure() {
        let raw = "#### 接口\nTextPicker(options?: TextPickerOptions)\n根据指定的数据列表创建文本选择器。\n**元服务API：** 从API version 11开始，该接口支持在元服务中使用。\n| 参数名 | 类型 | 必填 | 说明 |\n| --- | --- | --- | --- |\n| options | TextPickerOptions | 否 | 配置文本选择器的参数。 |\n| range | string[] | 否 | 选择器的数据选择列表。 |\n```\nconst a = 1;\n```\n正文段落应被丢弃";
        let out = compact_by_command("devecocli docs read API/TextPicker", &raw).unwrap();
        assert!(out.contains("#### 接口"), "标题保留: {out}");
        assert!(out.contains("TextPicker(options?: TextPickerOptions)"), "接口签名保留: {out}");
        assert!(out.contains("options: TextPickerOptions"), "参数名:类型保留: {out}");
        assert!(out.contains("range: string[]"), "参数名:类型保留: {out}");
        assert!(!out.contains("配置文本选择器"), "说明列丢弃: {out}");
        assert!(!out.contains("const a"), "代码块丢弃: {out}");
        assert!(!out.contains("正文段落"), "正文丢弃: {out}");
        assert!(out.contains("1 标题, 1 接口, 2 参数"), "计数: {out}");
    }
    #[test]
    fn devecocli_long_type_truncated() {
        let raw = "#### 属性\n| 名称 | 类型 | 只读 | 可选 | 说明 |\n| --- | --- | --- | --- | --- |\n| range | string[] | string[][]10+ | Resource | TextPickerRangeContent[] | 选择列表。 |";
        let out = compact_by_command("devecocli docs read X", &raw).unwrap();
        assert!(out.contains("range: string[] | string[][]10+ | Resource"), "类型 union 保留: {out}");
        assert!(!out.contains("选择列表"), "说明丢弃: {out}");
    }
}