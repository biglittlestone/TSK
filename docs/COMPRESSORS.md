# TSK 指令压缩器全量技术文档（deveco 平台实测）

> 来源：`core/src/compact.rs`（Rust 收益引擎）+ `integration/deveco/tsk.plugin.ts`（薄桥）
> 实测：`rtk-benefit-test.mjs` 真实命令 AB + onloop 真实 deveco 会话
> 日期：2026-09-30

## 0. 架构一句话

**deveco 插件**在 `tool.execute.after` 收到 bash 大输出 → `Bun.spawn(["tsk","compress", cmd])` 喂 stdin → **Rust `compact_by_command` 按命令类型压成紧凑文本** → 改写 `output.output` 送模型；**原文同时落盘 `.tsk/kb/` + 倒排索引**，模型可 `tsk search` 无损取回。

```
bash 大输出 ──tool.execute.after──► tsk compress (Rust)
                                   │
                    ┌──────────────┴──────────────┐
                    ▼                             ▼
           压缩文本 → 模型上下文            原文 → .tsk/kb/k<hash>.txt
                                        + index.json5 (BM25 倒排)
                                        + tsk search 无损回取
```

- **有损但保格式**：命令结果语义保留（计数/文件/提交/错误行），只砍 bulk
- **无损保回取**：原文永远落 KB，`tsk search` 精确命中
- **>1600B 才触发**：小输出放行 0% 开销（避免改写小结果）
- 未命中命令回退通用 `distill`（错误感知保留关键行 + 结尾）

---

## 1. 收益总表（实测）

### 1.1 单命令 AB（`rtk-benefit-test.mjs`，真实命令真实输出）

| 命令 | 原始 | 压缩后 | **省%** | 输出形态 | 无损 KB |
|---|---|---|---|---|---|
| git diff HEAD~5 | 15,353B | 554B | **96%** | files+hunks | ✓ |
| node --test（失败） | 24,920B | 890B | **96%** | 测试报告+失败上限 | ✓ |
| ls -laR | 7,339B | 447B | **94%** | N dirs, N files + 前20 | ✓ |
| grep -rn | 13,291B | 1,370B | **90%** | 按文件分组+行号 | ✓ |
| find | 6,161B | 672B | **89%** | N files + 前20 | ✓ |
| git log --stat | 8,924B | 1,821B | **80%** | N commits + subject | ✓ |
| git status --short | 23B | 23B | 0%（小放行） | — | — |
| rg/wc/du/df/ps 小输出 | — | — | 0%（小放行） | — | — |

### 1.2 真实 deveco 会话（onloop，9-30，成功场净收益）

| 场景 | 成功场 | saved | retr | **净/场** | 命中压缩器 |
|---|---|---|---|---|---|
| S7 git考古 | 4 | 24,330 | 0 | **+6,083** | git log/show/diff |
| S2 git重构 | 5 | 13,767 | 0 | **+2,753** | git status/log/diff |
| S4 日志排错 | 5 | 1,932 | 266 | +487 | grep + distill |
| S5 研读 | 5 | 0 | 0 | 0 | （无大 bash 输出） |
| S8 性能 | 1 | 0 | 0 | 0 | — |

> 口径：net = saved_tok − tsk_search 回读 token（模型读 KB 原文与正常读原文对等，不计入）。
> **收益 = 模型是否产出大工具输出**：git/构建/日志类显著，研读类零收益但零副作用。

---

## 2. 每个压缩器：收益原理 + 压缩后形态 + 实现

### 2.1 `ls` / `tree` —— 目录清单 → 计数 + 前 20 项

**原理**：模型要的是"目录里有什么"，不是权限/大小/日期元数据。剥掉 `total` 行和权限串，只留名字，加 dir/file 计数。

**压缩后形态**：
```
0 dirs, 5 files:
a.ts
b.ts
c.ts
d.ts
e.ts
```

**实现**（`compact.rs:34-49`）：
```rust
let names: Vec<String> = lines.iter().filter(|l| !l.starts_with("total ")).map(|l| {
    if Regex::new(r"^[bcdlps-][rwx-]{9}").unwrap().is_match(l) {
        l.split_whitespace().last().unwrap_or(&l)
    } else { &l }
}).collect();
// 计数 + 前 CAP_LIST(20) 项 + "+N more"
```

### 2.2 `find` —— 路径清单 → 计数 + 前 20

**原理**：过滤 `Permission denied`/`find:` 错误行，剥尾斜杠，dir/file 计数。

**形态**：`2 dirs, 30 files:\n<前20路径>\n… +N more paths`

**实现**：`compact.rs:51-57`。

### 2.3 `git status` —— 状态行 → 按状态分组计数

**原理**：`git status --short` 每行是 `XY 路径`，模型关心"哪些文件改了/新增/删除"，按状态分组 + 计数最省。

**形态**：
```
git status (5 行)：
?? (2):
  new1.js
  new2.js
M (3):
  mod1.js
  mod2.js
  mod3.js
```

**实现**：`compact.rs:59-74`，正则 `^([MADRCU?! ]{2}|[ MADRCU?]) ?(.*)$` 分组，空组回退 `raw` 前 800 字符。

### 2.4 `git log` / `git show` —— 提交历史 → 提交号 + subject 行

**原理**：每条 commit 只留 `短hash + subject`，丢 Author/Date/body/stat（模型要"有哪些提交、主题是什么"）。

**形态**：
```
2 commits:
abc1234567de fix: 解析空行
def4567890ab feat: 加缓存
```

**实现**：`compact.rs:76-95`，匹配 `^commit <hash>`，向后扫前 13 行找 subject（跳过 Author/Date/Merge/Signed-off）。

### 2.5 `git diff` / `git show` —— diff → 文件清单 + hunk 坐标

**原理**：模型要先知道"改了哪些文件、哪些 hunk"，再决定要不要看具体行。留 `diff --git` 文件名 + `@@ -a +b @@` 坐标。

**形态**：
```
2 files:
a.js
b.js

2 hunks:
@@ -1,5 +1,6 @@
@@ -10,3 +10,4 @@
```

**实现**：`compact.rs:97-117`，双正则 `^diff --git a/(\S+)(?: b/(\S+))?$` 和 `^@@ -(\d+...) +(\d+...) @@(.*)$`。文件改名显示 `a → b`。

### 2.6 `grep` / `rg` —— 匹配行 → 按文件分组 + 前 5 行/文件

**原理**：`grep -rn` 输出 `file:line:text`，按文件分组，每文件前 5 行，保留行号（模型可定位）。

**形态**：
```
grep (2 files, 13 matches):
a.js (6):
  3: const x = 1
  5: const y = 2
  … +4 rows
b.js (7):
  1: import z
  … +6 rows
```

**实现**：`compact.rs:119-144`，正则 `^([^:]+):(\d+):(.*)$` 分组。

### 2.7 `wc` —— 多文件计数 → 剥公共前缀

**原理**：`wc -l common/dir/*.ts` 每行都带 `common/dir/` 前缀，剥掉只留文件名差异。

**形态**：
```
wc (2 项，公共前缀"common/dir/"已剥)：
12 a.ts
34 b.ts
46 total
```

**实现**：`compact.rs:146-158`，`common_prefix()` 算所有路径共同前缀，`[prefix.len()..]` 切掉。

### 2.8 `du` / `df` / `ps` —— 表格式 → 表头 + 前 N 行

**原理**：模型要表头和前几行趋势，不关心数百行。

**形态**：
```
du (5 项):
1  a.ts
2  b.ts
...
total: ...
```
```
ps:
PID USER ...
123 u ...
… +N more
```

**实现**：`compact.rs:160-179`。

### 2.9 `curl` / `wget` —— 传输输出 → 结尾状态/错误/速率

**原理**：模型要"下载成功没/报错没"，留 HTTP 状态、error、`100%`、Saved 行（倒序取最后 8 条）。

**实现**：`compact.rs:181-184`。

### 2.10 测试/构建（vitest/jest/mocha/pytest/node --test…）—— 报告 → 摘要 + 失败上限

**原理**：模型最关心"过了几个、挂了几个、哪些失败"。留 `Tests:|Suites:|passed|failed` 摘要 + 失败行（CAP_ERR=20）。

**形态**（node --test 40 失败）：
```
Tests 0 passed | 40 failed (40)
failures (20 shown):
✗ t0.test.ts > case
  AssertionError ...
✗ t1.test.ts > case
  ...
```

**实现**：`compact.rs:186-194`，命令含测试框架名 **或** 输出含 pass/fail 标记才命中。

### 2.11 通用兜底 `distill`（插件侧，未命中命令）—— 错误感知蒸馏

**原理**：任何大输出若没被上述命中，保留 `关键错误/状态行 + 结尾`，丢中间 bulk——模型仍能看错修错。

**形态**：
```
── 关键错误/状态行 ──
ERROR build-3 cannot load mod1
✖ error count (1.24ms)
── 结尾 ──
Tests 7, suites 0
── end ──
```

**实现**：`tsk.plugin.ts distill()`（`FINAL_RESULT_KEEP` 结尾字节 + KEY 正则过滤前 60 关键行）。

### 2.12 通用前置清洗（所有输出先过）—— 收益放大器

`compressPipeline`（插件侧）：ANSI 剥除 → 命令专用过滤（npm warn/git warning/hint）→ JSON minify → 进度行剔除 → 重复行折叠（≤3 次）→ `smartTruncate`（超 4×KEEP 时 head60%+tail40%+超大行采样）。

---

## 3. 无损回读（KB + tsk search）

- 每个压缩输出同时 `kbsave`：原文写 `.tsk/kb/k<hash>.txt` + `index.json5` 倒排索引（tokenize + inv 词表）
- 大输出按空行切 **B 分节**（≤64 节，每节独立落盘+索引）→ 模型只看"章节地图"即可 `tsk search` 定向取回单节
- `tsk search <q>`：query tokenize → 倒排打分 → 返回 top3 节 + 精确 snippet → 模型 read KB 文件拿全文
- 实测 `git diff` 原文可精确找回（KB 命中），回读开销可忽略（净收益中 retr ≤ 0.7%）

---

## 4. 阈值与放行策略（为什么小输出 0%）

```
>1600B 才触发压缩；小输出原样放行（0% 开销、不改写）
```
实测：git status(23B)/rg(18B)/wc(1.5KB)/du/df/ps 全 0%——避免对"本来就是短结果"的命令做无谓改写。

---

## 5. 关键代码位置速查

| 文件 | 位置 | 内容 |
|---|---|---|
| `core/src/compact.rs` | 全文 | 13 个指令压缩器 + 单测 |
| `cli/src/main.rs:129-131` | `Cmd::Compress` | stdin 读 raw → `compact_by_command` |
| `integration/deveco/tsk.plugin.ts` | `tskCompress()` L120 | `Bun.spawn(["tsk","compress",cmd])` 薄桥 |
| `tsk.plugin.ts` | `kbsave/kbsearch/indexSections` | KB 落盘 + 倒排 + B 分节 |
| `tsk.plugin.ts` | `tool.execute.after` L262 | 大输出 → 压缩改写 output + 落 KB |

## 6. 结论

1. **收益集中在"大工具输出"**：git diff/log/show 96%/80%、测试 96%、grep 90%、ls 94%、find 89%
2. **每成功场净省 0~6,083 tok**（git 考古类最高），研读类零收益但零副作用、零 hook 错误
3. **净收益 = saved − tsk_search 回读**；实测回读占比 ≤0.7%，可忽略
4. **免费额度是唯一硬约束**：GLM-5.1 免费 50M tokens/日，够 ~15-20 成功场
