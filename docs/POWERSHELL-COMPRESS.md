# PowerShell 命令输出压缩调研

> **状态（2026-10-19）：已实现并扩展。** 本文档的「现状」描述已过时——PowerShell 压缩已落地于
> `core/src/compact/mod.rs`（PS 块），覆盖 measure/get-process/get-childitem(-recurse)/get-item/
> select-string **+ 通用表格兜底 + Get-WinEvent/Get-EventLog + `-Name` 变体**。
> 完整现状见 `docs/COMPRESSORS.md` §7.1；本文保留为设计留档。

目标：评估 TSK 对 **PowerShell 命令**（Windows 环境下 deveco/开发常用）的输出压缩。
现状：当前 `compact_by_command` 只识别 bash 系命令（ls/git/grep/find/wc/du/df/ps/curl/devecocli docs/测试），
PowerShell 命令（`Get-ChildItem`/`gci`/`Get-Process`/`Select-String`/`Measure-Object`…）**全部不识别 → 放行**。

## 1. PowerShell 命令与 bash 映射（实测输出格式）

| PowerShell 命令（别名） | bash 对应 | 实测输出格式 | 压缩可行性 |
|---|---|---|---|
| `Get-ChildItem`（gci / ls / dir） | `ls` | 表格：`目录:` 行 + 表头 `Mode LastWriteTime Length Name` + `----` 分隔线 + 数据行（`d-----`=目录） | ✅ 强（ls 语义） |
| `Get-ChildItem -Recurse` | `find` | 表格：单列 `FullName` + 分隔线 + 全路径行 | ✅ 强（find 语义） |
| `Get-Process`（gps / ps） | `ps` | 表格：表头 `Id ProcessName CPU WorkingSet64` + 分隔线 + 数据行 | ✅ 中（ps 语义） |
| `Select-String`（sls） | `grep` | 表格：表头 `IgnoreCase LineNumber Line` + 分隔线 + 命中行（Line 列含匹配内容） | ✅ 中（grep 语义） |
| `Measure-Object`（measure） | `wc` | 属性列表：`Count : 475` / `Sum :` / `Maximum :` | ✅ 强（统计语义） |
| `Get-Content`（gc / cat） | `cat` | 纯文本行 | 放行（模型用 -Tail/-First 或 read 工具） |
| `Get-Help` | （文档） | markdown/文本 | 参照 devecocli docs 思路（低优先） |

### 实测样例（本机）

```
Get-ChildItem:
    目录: C:\Windows\System32\drivers
Mode          LastWriteTime  Length Name
----          -------------  ------ ----
d-----  2026/6/25      9:41         DriverData
-a----  2026/6/25      9:33  294912 1394ohci.sys

Get-Process:
   Id ProcessName      CPU         WorkingSet64
   -- -----------      ---         ------------
 8004 AggregatorHost                   15106048

Measure-Object:
Count    : 475
Average  :
Sum      :
Maximum  :
```

## 2. 压缩设计（每类解析规则）

统一难点：PowerShell 输出是「表格 + `----` 分隔线 + 数据行」，且 **命令名大小写不敏感**
（`Get-ChildItem` vs `get-childitem`），当前 `is()` 正则大小写敏感需处理。

### 2.1 gci / Get-ChildItem → ls 式
- 跳过：空行、`目录:` 行、表头行（首列 Mode）、`----` 分隔线
- 数据行：`^[d-]-a?---- ...` 首字符 `d`=目录（Name 加 `/`）；取**最后字段**为 Name
- 输出：`N dir, M file:` + 前 20 Name（+ more），复用 CAP_LIST

### 2.2 gci -Recurse / Get-ChildItem -Recurse → find 式
- 跳过表头/分隔线，取全路径行
- 输出：`N files:` + 前 20 路径（+ more）

### 2.3 gps / Get-Process → ps 式
- 保留表头 + 前 20 数据行（丢 CPU/内存列精度？保留原行）
- 输出：`ps:` + 表头 + 前 N 行

### 2.4 measure / Measure-Object → 统计式
- 提取 `Count`/`Sum`/`Maximum`/`Minimum` 有值的行
- 输出：`Measure-Object: Count N | Sum S | …`

### 2.5 sls / Select-String → grep 式
- 命中行按文件分组（若含 Path 列）或保留 `行号: 内容`
- 输出：`grep (N files, M matches):` + 分组前 20

## 3. 落地建议（compact.rs 扩展）

1. **PowerShell 命令识别**：`is()` 匹配命令首词（别名 + 全名 + `Get-` 前缀），处理大小写不敏感：
   ```rust
   // cmd 小写化后匹配：gci|get-childitem|gps|get-process|sls|select-string|measure-object|measure|gc|get-content|ls|dir
   ```
   （`ls`/`dir` 在 PowerShell 里是 Get-ChildItem 别名——注意与 bash ls 区分：命令含 `Get-ChildItem`/`gci`/`powershell`/`pwsh` 前缀才走 PowerShell 分支，否则走 bash ls。）
2. **新增 `pwsh` 分支**：按子命令分派到 2.1-2.5。
3. **单测**：构造上述实测格式断言（计数/表头/分隔线跳过）。
4. **收益预期**：Windows 用户用 PowerShell 跑 `gci`/`gps`/`sls` 大输出时触发；复用现有 ls/ps/grep 的 80-98% 压缩比。

## 4. 结论

PowerShell 命令压缩**可行且收益明确**：5 个常用命令（gci/gci -r/gps/sls/measure）输出格式稳定，
可复用 ls/find/ps/grep/wc 语义压缩（跳过表头+分隔线，保留核心列）。落地成本低（compact.rs 一个分支 + 单测），
收益场景 = Windows 开发者用 PowerShell 检索/列目录/查进程（大输出触发）。
