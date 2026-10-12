# TSK × DevEco Code 一键安装

让 DevEco Code 的大命令输出自动压缩（git / ls / grep / find / devecocli docs / 测试…），
压缩后原文落盘知识库，可无损取回。**全程 fail-open，任何异常不影响原功能。**

## 安装（1 分钟）

1. 双击 `install.bat`
2. 看到 `DONE. Restart DevEco Code` 即完成
3. 重启 DevEco Code —— 插件自动加载，**无需改任何配置**

## 验证（可选）

新开会话，跑一个大命令（如 `git log --stat`、`ls -laR src`）：
输出 >1600 字符时会看到 `[TSK {bash}: 压 …B→…B]` 包裹头 + 全文指针 + 章节地图。

## 卸载（双击）

1. 双击 `uninstall.bat`
2. 重启 DevEco Code

脚本自动：删除 `%USERPROFILE%\.tsk\bin\tsk.exe` + 移除用户 PATH 条目 + 删除插件 `tsk.ts` + 删除 `/tsk` 命令文件（`tsk.js`/`tsk.md`）。

## 常见问题

**重启 DevEco 后终端里 `tsk` 命令找不到？**
- 新加的 `%USERPROFILE%\.tsk\bin` 需要**重启电脑**才能让所有进程（含 DevEco 及其内嵌终端）生效。
- **插件本身不依赖 PATH**：会依次尝试 `tsk` → `%USERPROFILE%\.tsk\bin\tsk.exe` 兜底，压缩照常工作。

**如何查看 tsk 的累计收益？**
在 DevEco 会话里输入（`/tsk` 已注册，直接输出，不依赖模型）：

| 输入 | 作用 |
|---|---|
| `/tsk` | 本会话（最近会话）收益 |
| `/tsk all` | 全部会话累计收益 |
| `/tsk clean` | **重置记账**（清空统计，之后重新累计） |

等价终端命令：`tsk report`（本会话）/ `tsk report --all`（全部）/ `tsk report --clean`（重置）。
前提：install.bat 已装 `tsk`；插件记账在项目 `.tsk/injections.jsonl`。

## 说明

- 安装内容：`tsk.exe`（压缩引擎 → `%USERPROFILE%\.tsk\bin`）+ `tsk.plugin.ts`（插件 → `%USERPROFILE%\.deveco\plugin\tsk.ts`）+ `tsk.js`/`tsk.md`（`/tsk` 命令 → `.deveco\command\` 与 `.config\opencode\command\`）
- 要求：DevEco Code 自带 Bun 运行时；无需安装 Node/Python
- 支持的命令级压缩：ls/tree · find · git status/log/diff · grep/rg · wc · du · df · ps · curl/wget · devecocli docs · 测试/构建
- 不认识的命令原样放行，小输出（≤1600 字符）不压
