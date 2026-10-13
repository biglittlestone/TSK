//! TSK report：ledger 收益统计（实时准确 saved/net/retrievals）+ deveco 输出压缩汇总 + CI cache-hit。
//! 从 main.rs 拆分：报告逻辑独立内聚，main.rs 专注命令分发与其他命令。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use tsk_core::ledger::Entry;
use tsk_core::storage;

/// Estimated tokens from bytes: ⌈bytes/4⌉ (TSK 不跑 tokenizer 的近似口径)。
fn est_tok(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// `tsk report [--session S | --all | --clean | --cache-hit P]`：默认当前会话。
pub fn report(session: Option<String>, all: bool, clean: bool, cache_hit_path: Option<PathBuf>) -> String {
    if let Some(p) = cache_hit_path {
        return cache_hit(&p);
    }
    if clean {
        return clear_ledgers();
    }
    if let Some(sid) = session {
        let entries = tsk_core::ledger::read_all(&storage::session_dir(&sid).join("ledger.jsonl"));
        return format!("session {sid}\n{}", summarize(&entries));
    }
    if all {
        return report_all();
    }
    // 默认：当前会话（最后一钩子记录的 session）。无记录则退回全量。
    match storage::read_current_session() {
        Some(sid) => {
            let entries = tsk_core::ledger::read_all(&storage::session_dir(&sid).join("ledger.jsonl"));
            format!("current session {sid}\n{}", summarize(&entries))
        }
        None => report_all(),
    }
}

/// 全部会话累计：saved / inject cost / net（= saved − 注入 − 实际回读）/ per-tool。
fn report_all() -> String {
    let mut total: usize = 0;
    let mut cost: usize = 0;
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    let mut sessions = 0usize;
    if let Ok(rd) = std::fs::read_dir(storage::tsk_home()) {
        for e in rd.flatten() {
            let p = e.path().join("ledger.jsonl");
            if !p.is_file() {
                continue;
            }
            sessions += 1;
            for x in tsk_core::ledger::read_all(&p) {
                total += saved_of(&x);
                cost += cost_of(&x);
                if let Entry::Retrieval(r) = &x {
                    cost += r.retrieved_bytes.unwrap_or(0); // 回读成本计入净省扣减
                }
                *tools.entry(tool_of(&x).unwrap_or("?").to_string()).or_insert(0) += saved_of(&x);
            }
        }
    }
    let net = total.saturating_sub(cost);
    let mut s = format!(
        "saved: {total} bytes (≈ {} est tokens) across {sessions} session(s)\n\
         inject cost: {cost} bytes (≈ {} est tokens)\n\
         net: {net} bytes\n",
        est_tok(total),
        est_tok(cost)
    );
    for (k, v) in &tools {
        s.push_str(&format!("  {k}: {v}\n"));
    }
    // deveco 输出压缩 saved：读 <cwd>/.tsk/injections.jsonl 的 sandbox-output 条目（rtk 式逐命令压缩）
    let (dc_tok, dc_n) = deveco_output_saved();
    if dc_n > 0 {
        s.push_str(&format!(
            "\ndeveco 输出压缩(deveco 插件 tool.execute.after): saved ≈ {dc_tok} tok / {dc_n} 次大输出\n"
        ));
    }
    s
}

/// 从 <cwd>/.tsk/injections.jsonl 汇总 deveco 输出压缩的 saved_tokens 与压缩次数。
fn deveco_output_saved() -> (usize, usize) {
    let path = std::env::current_dir().ok().map(|d| d.join(".tsk/injections.jsonl"));
    let Some(path) = path.filter(|p| p.is_file()) else { return (0, 0) };
    let Ok(content) = std::fs::read_to_string(&path) else { return (0, 0) };
    let mut tok = 0usize;
    let mut n = 0usize;
    for line in content.lines() {
        let Ok(v): Result<Value, _> = serde_json::from_str(line) else { continue };
        if v.get("kind").and_then(|k| k.as_str()) != Some("sandbox-output") {
            continue
        }
        if let Some(t) = v.get("saved_tokens").and_then(|t| t.as_u64()) {
            tok += t as usize;
            n += 1;
        }
    }
    (tok, n)
}

/// `report --clean`: remove every ledger + the current-session marker (reset stats).
fn clear_ledgers() -> String {
    let mut n = 0usize;
    if let Ok(rd) = std::fs::read_dir(storage::tsk_home()) {
        for e in rd.flatten() {
            let p = e.path().join("ledger.jsonl");
            if p.is_file() {
                if std::fs::remove_file(&p).is_ok() {
                    n += 1;
                }
            }
        }
    }
    let _ = std::fs::remove_file(storage::current_session_file());
    format!("tsk: cleared {n} ledger file(s) + current-session marker\n")
}

/// Estimated-token cost of one entry (injection bytes for `inject`, else 0).
fn cost_of(e: &Entry) -> usize {
    match e {
        Entry::Inject(x) => x.bytes,
        _ => 0,
    }
}

/// Saved bytes of one entry (compression delta).
fn saved_of(e: &Entry) -> usize {
    match e {
        Entry::Rewrite(x) => x.saved,
        Entry::Sandbox(x) => x.saved,
        Entry::Externalize(x) => x.orig_bytes.saturating_sub(x.new_bytes),
        _ => 0,
    }
}

fn tool_of(e: &Entry) -> Option<&str> {
    match e {
        Entry::Rewrite(x) => Some(&x.tool),
        Entry::Retrieval(x) => Some(&x.tool),
        Entry::Externalize(x) => Some(&x.tool),
        Entry::Sandbox(x) => Some(&x.tool),
        _ => None,
    }
}

/// 单会话汇总：saved / inject cost / net / retrievals（实时实测，无假设档位）+ per-tool。
fn summarize(entries: &[Entry]) -> String {
    let mut total = 0usize;
    let mut cost = 0usize;
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    let mut retrievals = 0usize;
    for e in entries {
        total += saved_of(e);
        cost += cost_of(e);
        if let Some(t) = tool_of(e) {
            *tools.entry(t.to_string()).or_insert(0) += saved_of(e);
        }
        if matches!(e, Entry::Retrieval(_)) {
            retrievals += 1;
        }
        // 回读全文的字节成本计入净省扣减（净省 = saved − 注入 − 实际回读）
        if let Entry::Retrieval(r) = e {
            cost += r.retrieved_bytes.unwrap_or(0);
        }
    }
    let net = total.saturating_sub(cost);
    let mut s = format!(
        "saved       : {} bytes ≈ {} est tokens\n\
         inject cost : {} bytes ≈ {} est tokens\n\
         net         : {} bytes\n\
         retrievals  : {}\n",
        fmt_int(total),
        est_tok(total),
        fmt_int(cost),
        est_tok(cost),
        fmt_int(net),
        retrievals
    );
    let maxk = tools.keys().map(|k| k.len()).max().unwrap_or(4);
    for (k, v) in &tools {
        let pct = if total > 0 { *v * 100 / total } else { 0 };
        s.push_str(&format!(
            "  {k:<pad$}: {val} bytes ({pct}%)\n",
            k = k,
            val = fmt_int(*v),
            pct = pct,
            pad = maxk
        ));
    }
    s
}

/// Thousands separator for readability: 199700 -> 199,700.
fn fmt_int(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// CI metric, iron law 6: cache reads / total input, from a transcript JSONL.
fn cache_hit(path: &PathBuf) -> String {
    let lines = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return format!("cache-hit: cannot read {}: {e}\n", path.display()),
    };
    let mut turns = 0u64;
    let mut reads = 0u64;
    let mut input = 0u64;
    for line in lines.lines() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(u) = v.pointer("/message/usage") else { continue };
        let i = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        let c = u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        turns += 1;
        reads += c;
        input += i;
    }
    if input == 0 {
        return format!("cache-hit: no usage data in {}\n", path.display());
    }
    format!("cache-hit: {}/{} turns, {:.1}% cache read\n", turns, turns, reads as f64 * 100.0 / input as f64)
}
