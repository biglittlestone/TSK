//! Searchable index over TSK's externalized/stored content (Think-in-Code 辅助)。
//!
//! context-mode 的 `ctx_search` 收益：索引全文、按需返回精确 chunk（代码块/关键行原样），
//! 而不是把 bulk 读进上下文。这里把 TSK 已外置/落盘的全文（`<TSK_HOME>/<session>/ext/*.txt`
//! 及项目 `.tsk/kb/*.txt`）建成轻量倒排索引，`tsk search <query>` 按词得分返回
//! { 路径, 分数, 精确匹配行窗 snippet, 长度 }。纯文件+内存，不用 SQLite。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::storage;

const SNIPPET_LINES: usize = 6; // snippet 上下各取几行
const MAX_INDEX_BYTES: usize = 1024 * 1024; // 单文件索引上限（防超大文件读满）

/// 一个可检索的落盘片段。
pub struct IndexedFile {
    pub path: std::path::PathBuf,
    pub session: String,
    pub title: String,
    pub bytes: usize,
    pub text: String,
}

/// 一条搜索结果。
#[derive(Debug)]
pub struct SearchHit {
    pub path: std::path::PathBuf,
    pub score: u32,
    pub bytes: usize,
    pub snippet: String, // 精确匹配行窗（原文保留）
}

fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect()
}

/// 收集 TSK 已落盘的可检索全文。
///   - `<TSK_HOME>/<session>/ext/*.txt`：externalize 的原文；
///   - 项目 `.tsk/kb/*.txt`：知识库原文（若存在）。
pub fn collect_indexed() -> Vec<IndexedFile> {
    let mut out = Vec::new();
    let home = storage::tsk_home();
    let mut scan = |root: &Path, prefix: &str| {
        let Ok(read) = std::fs::read_dir(root) else { return };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if !p.is_file() || p.extension().and_then(|x| x.to_str()) != Some("txt") {
                continue;
            }
            let bytes = std::fs::metadata(&p).map(|m| m.len() as usize).unwrap_or(0);
            if bytes == 0 || bytes > MAX_INDEX_BYTES {
                continue;
            }
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let session = prefix.to_string();
            out.push(IndexedFile { path: p.clone(), session, title: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), bytes, text });
        }
    };
    // sessions = <TSK_HOME>/<sid>/ext/
    if let Ok(rd) = std::fs::read_dir(&home) {
        for e in rd.flatten() {
            let sid = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() {
                scan(&e.path().join("ext"), &sid);
            }
        }
    }
    out
}

/// 由 query 的所有词对每个文件打分（简单 BM25 式：词频累加），返回按分降序。
/// 仅保留出现任意查询词的文件；查词越少命中越精确。
pub fn search(query: &str, top: usize, session_filter: Option<&str>) -> Vec<SearchHit> {
    let qterms = tokenize(query);
    if qterms.is_empty() {
        return Vec::new();
    }
    let qset: HashSet<String> = qterms.iter().cloned().collect();
    let mut hits: Vec<SearchHit> = Vec::new();

    // 统计文档频率（df），用于 IDF 权重
    let mut df: HashMap<String, u32> = HashMap::new();
    let mut doc_scores: Vec<(IndexedFile, HashMap<String, i32>)> = Vec::new();
    for f in collect_indexed() {
        if let Some(sess) = session_filter {
            if f.session != *sess {
                continue;
            }
        }
        let terms = tokenize(&f.text);
        let mut tf: HashMap<String, i32> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::new();
        for t in &terms {
            *tf.entry(t.clone()).or_default() += 1;
            if seen.insert(t.clone()) {
                *df.entry(t.clone()).or_default() += 1;
            }
        }
        doc_scores.push((f, tf));
    }
    let n = doc_scores.len().max(1) as f64;

    for (f, tf) in &doc_scores {
        let mut score = 0f64;
        for t in &qset {
            let Some(&c) = tf.get(t) else { continue };
            // tf * idf；idf = ln(1 + (N - df + 0.5)/(df + 0.5))
            let dfn = *df.get(t).unwrap_or(&1) as f64;
            let idf = (1.0 + (n - dfn + 0.5) / (dfn + 0.5)).ln();
            score += (c as f64) * idf;
        }
        if score > 0.0 {
            // 找首个命中行的 snippet
            let snippet = snippet_around_hit(&f.text, &qset, SNIPPET_LINES);
            hits.push(SearchHit { path: f.path.clone(), score: (score * 100.0) as u32, bytes: f.bytes, snippet });
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits.truncate(top.max(1));
    hits
}

/// 返回首个包含查询词的行的「±SNIPPET_LINES 行」原文窗口，保留原样（精确 chunk）。
fn snippet_around_hit(text: &str, qset: &HashSet<String>, pad: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    // 找首个命中行
    let mut hit = None;
    for (i, l) in lines.iter().enumerate() {
        let lt = l.to_lowercase();
        if qset.iter().any(|q| lt.contains(q)) {
            hit = Some(i);
            break;
        }
    }
    match hit {
        Some(c) => {
            let s = c.saturating_sub(pad);
            let e = (c + pad + 1).min(lines.len());
            let mut out = String::new();
            if s > 0 {
                out.push_str(&format!("…[{} lines above]…\n", c - s));
            }
            out.push_str(&lines[s..e].join("\n"));
            if e < lines.len() {
                out.push_str(&format!("\n…[{} lines below]…", lines.len() - e));
            }
            out
        }
        None => text.chars().take(600).collect(),
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn tokenize_splits_and_lowercases() {
        assert_eq!(tokenize("Error: Cannot find foo_bar"), vec!["error", "cannot", "find", "foo_bar"]);
    }

    #[test]
    fn search_finds_exact_line_snippet() {
        let text = "line a\nline b\nERROR 404 at foo\nline d\nline e\nline f\nline g\n";
        let q: HashSet<String> = ["error"].iter().map(|s| s.to_string()).collect();
        let s = snippet_around_hit(text, &q, 1);
        assert!(s.contains("ERROR 404 at foo"), "snippet must keep exact hit, got: {s}");
        assert!(s.contains("line b") && s.contains("line d"), "snippet should include neighbors");
    }
}