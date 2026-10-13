//! Drain 式日志模板挖掘（借鉴 headroom LogTemplate）【实验特性，暂不入 deveco PR】
//! 把「只有时间戳/ID/路径变化」的重复行折叠成 [模板: ...] (N 次) + 变体表（无损，可重建）。

// ---- Drain 式日志模板挖掘（借鉴 headroom LogTemplate）【实验特性，暂不入 deveco PR】----
// 把「只有时间戳/ID/路径变化」的重复行折叠成 [模板: <TS> worker-<*> job <*>] (N 次) + 变体表。
// 只折叠连续 run、保顺序；保守参数：min_run=3、相似度 0.4、至少 2 个常量 token。
const LT_MIN_RUN: usize = 3;
const LT_SIM: f32 = 0.4;
const LT_MIN_CONST: usize = 2;

struct LtRun {
    indices: Vec<usize>,
    /// Some(token)=该位置至今不变；None=已变化(通配 <*>)
    template: Vec<Option<String>>,
}

/// 新行是否延续当前 run：token 数相同 + 匹配位置占比 ≥ 相似度阈值（含已有通配位）。
fn lt_extends(run: &LtRun, tokens: &[&str]) -> bool {
    if tokens.len() != run.template.len() { return false }
    let len = tokens.len() as f32;
    let matches: usize = tokens.iter().enumerate()
        .map(|(i, t)| match &run.template[i] { Some(c) if c == t => 1, None => 1, _ => 0 })
        .sum();
    (matches as f32 / len) >= LT_SIM
}

/// 位置 token 与模板不同 → 变通配。
fn lt_merge(template: &mut [Option<String>], tokens: &[&str]) {
    for (i, t) in tokens.iter().enumerate() {
        if let Some(c) = &template[i] { if c != t { template[i] = None } }
    }
}

fn lt_flush(run: &LtRun, lines: &[&str], tokenized: &[Vec<&str>], tid: &mut usize, out: &mut String) {
    let consts = run.template.iter().filter(|t| t.is_some()).count();
    let varies = run.template.len() - consts;
    let collapse = run.indices.len() >= LT_MIN_RUN && consts >= LT_MIN_CONST && varies > 0;
    if !collapse {
        for &i in &run.indices { out.push_str(lines[i]); out.push('\n'); }
        return
    }
    // 模板头：[Template T<n>: TOKEN <*> TOKEN ...] (N occurrences)
    out.push_str(&format!("[Template T{tid}: "));
    *tid += 1;
    for (i, slot) in run.template.iter().enumerate() {
        if i > 0 { out.push(' ') }
        match slot { Some(c) => out.push_str(c), None => out.push_str("<*>") }
    }
    out.push_str(&format!("] ({} occurrences)\n", run.indices.len()));
    // 变体表：每行只输出变体位置的 token
    for &i in &run.indices {
        let toks = &tokenized[i];
        let mut first = true;
        for (pos, slot) in run.template.iter().enumerate() {
            if slot.is_none() {
                if !first { out.push(' ') }
                out.push_str(toks[pos]);
                first = false;
            }
        }
        out.push('\n');
    }
}

/// 日志模板挖掘（无损）：重复进度/日志行折叠为模板+变体表；空行/单行/不达标原样。
pub fn log_template(text: &str) -> String {
    if text.is_empty() { return String::new() }
    let lines: Vec<&str> = text.lines().collect();
    let tokenized: Vec<Vec<&str>> = lines.iter().map(|l| l.split_whitespace().collect()).collect();
    let mut out = String::with_capacity(text.len());
    let mut tid = 1usize;
    let mut run: Option<LtRun> = None;
    for (i, tokens) in tokenized.iter().enumerate() {
        if tokens.is_empty() {
            if let Some(r) = run.take() { lt_flush(&r, &lines, &tokenized, &mut tid, &mut out) }
            out.push_str(lines[i]); out.push('\n');
            continue
        }
        match run.as_mut() {
            Some(r) if lt_extends(r, tokens) => { r.indices.push(i); lt_merge(&mut r.template, tokens) }
            _ => {
                if let Some(r) = run.take() { lt_flush(&r, &lines, &tokenized, &mut tid, &mut out) }
                run = Some(LtRun { indices: vec![i], template: tokens.iter().map(|t| Some((*t).to_string())).collect() });
            }
        }
    }
    if let Some(r) = run.take() { lt_flush(&r, &lines, &tokenized, &mut tid, &mut out) }
    // 防御：绝不比原文长（折叠未生效时回退原文）
    if out.len() >= text.len() { return text.to_string() }
    out
}
