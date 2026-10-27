#!/usr/bin/env bun
// L4 跨实现一致性：TS 移植（deveco-code 的 bash-compact.ts） vs Rust 核心（tsk compress）。
//
// 为什么放在 TSK 仓：deveco 的 PR 只有 TS，不应知道 Rust 版本；
// 这份「逐字节一致」校验属于 TSK 侧的移植守护，不进入 PR。
//
// 用法：
//   bun tools/parity-bash-compact.mjs                     # 自动找 ../deveco-code 与 PATH 里的 tsk
//   bun tools/parity-bash-compact.mjs --ts <path> --tsk <bin>
//   bun tools/parity-bash-compact.mjs --verbose
//
// 退出码：0=全部一致；1=存在不一致；2=参数/文件缺失；3=tsk 不可用。
import { spawnSync } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import { pathToFileURL } from "node:url"

const HERE = import.meta.dirname
const argv = process.argv.slice(2)
const arg = (name, def) => {
  const i = argv.indexOf(name)
  return i >= 0 ? argv[i + 1] : def
}
const verbose = argv.includes("--verbose")

const tsPath = arg(
  "--ts",
  process.env.DEVECO_CODE_BASH_COMPACT ||
    path.resolve(process.env.DEVECO_CODE_HOME || path.join(HERE, "..", "..", "deveco-code"), "packages/core/src/tool/bash-compact.ts"),
)
const tskBin = arg("--tsk", process.env.TSK_BIN || "tsk")

if (!fs.existsSync(tsPath)) {
  console.error(`[fatal] 找不到 TS 实现：${tsPath}\n用 --ts 或 DEVECO_CODE_BASH_COMPACT 指定。`)
  process.exit(2)
}
const probe = spawnSync(tskBin, ["--version"], { encoding: "utf8" })
if (probe.error) {
  console.error(`[fatal] 找不到 tsk 可执行文件：${tskBin}\n用 --tsk 或 TSK_BIN 指定（需已构建）。`)
  process.exit(3)
}

const { compactByCommand } = await import(pathToFileURL(tsPath).href)

// ---------------- 夹具（与 Rust core/src/compact/mod.rs 的单测 + TS 测试同源） ----------------
const cases = []
const add = (name, cmd, raw) => cases.push({ name, cmd, raw })

// PowerShell
add("ps: generic table", "Get-Service", "Status   Name               DisplayName\n------   ----               -----------\n" + Array.from({ length: 30 }, (_, i) => `Running  svc${i}  Service ${i}`).join("\n"))
add("ps: plain text (no match)", "Get-ItemProperty X", "line one\nline two\n- single dash\nline four")
add("ps: measure-object", "Get-ChildItem | Measure-Object", "Count : 42\nAverage : 3.5\nSum : 147\nMaximum : 9\nMinimum : 1")
add("ps: get-process", "Get-Process", " NPM(K)    PM(M)   Id  ProcessName\n ------    -----   --  -----------\n" + Array.from({ length: 40 }, (_, i) => `   100     50.0  ${1000 + i}  proc${i}`).join("\n"))
add("ps: select-string", "Select-String -Pattern export", Array.from({ length: 30 }, (_, i) => `C:\\x\\f${i}.ts:${i + 1}:export const v${i} = ${i}`).join("\n"))
add("ps: get-childitem ls", "Get-ChildItem C:\\x", "Mode   LastWriteTime   Length Name\n----   -------------   ------ ----\n" + Array.from({ length: 30 }, (_, i) => `${i % 2 ? "-a----" : "d-----"}  2026/01/01  1024 item${i}`).join("\n"))
add("ps: recurse any drive", "Get-ChildItem -Recurse", "C:\\a\\b.txt\nD:\\proj\\src\\c.txt\n\\\\server\\share\\d.txt\nE:\\e.txt")
add("ps: -name", "Get-ChildItem -Name", "file1.txt\nfile2.txt\nsubdir")
add("ps: json", "Get-Content package.json | ConvertFrom-Json", `{"name":"pkg","version":"1.0.0","scripts":{"build":"tsc"},"dependencies":{"x":"^1"}}`)
add("ps: kv", "Get-Item C:\\x\\Foo.txt | Format-List", "Name        : Foo\nLength      : 12345\nMode        : -a---\nLastWrite   : 2026/01/01\nFullName    : C:\\x\\Foo.txt")
add("ps: winevent", "Get-WinEvent -LogName System", `ProviderName: TestProv\nTimeCreated  Id  LevelDisplayName  Message\n--------\n2026-01-01  1  Error  ${"x".repeat(300)}\n2026-01-02  2  Error  short`)

// 编译器
add("compile: clang", "clang -c src/a.c", "src/a.c:3:5: error: use of undeclared identifier 'x'\nsrc/a.c:7:1: warning: unused variable 'y'\nsrc/b.c:2:9: error: expected ';'")
add("compile: rustc", "rustc src/main.rs", "error[E0425]: cannot find value `x` in this scope\nerror: aborting due to previous error")

// 既有命令
add("ls", "ls -la", Array.from({ length: 60 }, (_, i) => `${i < 20 ? "drwxr-xr-x" : "-rw-r--r--"} user grp ${1000 + i} Sep 27 10:00 mod${String(i).padStart(2, "0")}`).join("\n"))
add("git status", "git status --short", Array.from({ length: 140 }, (_, i) => (i % 3 === 0 ? `M  mod${i}.ts` : i % 3 === 1 ? `?? untracked${i}.js` : `A  added${i}.ts`)).join("\n"))
add("git log", "git log --stat", Array.from({ length: 40 }, (_, i) => `commit ${"deadbeef" + String(i).padStart(8, "0")}\nAuthor: u\nDate:  2026-09-2${i % 9}\n\n    fix: patch ${i} thing`).join("\n\n"))
add("git diff", "git diff HEAD", `diff --git a/src/a.ts b/src/a.ts\nnew file mode 100644\n@@ -0,0 +1,200 @@\n${Array.from({ length: 300 }, () => ` ${"a".repeat(40)}`).join("\n")}\ndiff --git a/src/b.ts b/src/b.ts\n@@ -10,5 +10,4 @@\n 1 file changed, 205 insertions(+), 5 deletions(-)`)
add("grep", "rg -n 'export const' src", Array.from({ length: 70 }, (_, i) => `src/f${i}.ts:${12 + i}:export const v${i} = ${i};`).join("\n"))
add("wc", "wc -l a/b/c/*.ts", Array.from({ length: 90 }, (_, i) => `123 456 7890 common/dir/${String.fromCharCode(97 + (i % 10))}.ts`).join("\n"))
add("find", "find . -name '*.ts'", Array.from({ length: 140 }, (_, i) => `/repo/src/d${i % 4}/file${String(i).padStart(3, "0")}.ts`).join("\n"))
add("du", "du -sh *", `12K\t./node_modules\n${Array.from({ length: 80 }, (_, i) => `${i + 1}M\t./src/file${i}.ts`).join("\n")}\n500M\ttotal`)
add("df", "df -h", `Filesystem     1K-blocks    Used Available Use% Mounted on\n${Array.from({ length: 45 }, (_, i) => `/dev/sda${i}  1048576 ${i * 10} ${999999 - i * 10} ${i}% /mnt/d${i}`).join("\n")}`)
add("ps", "ps aux", `PID USER COMMAND\n${Array.from({ length: 120 }, (_, i) => `${1000 + i} acetone cmd${i} --flag=${"z".repeat(40)} ${i}`).join("\n")}`)
add("curl", "curl -v https://x", `* Trying 1.2.3.4...\n* Connected\n> GET / HTTP/1.1\n< HTTP/1.1 200 OK\n< content-type: text/html\n${Array.from({ length: 60 }, () => "< header line").join("\n")}\n* Connection #0 left intact`)
add("cat", "cat someBig.log", "y".repeat(3200))
add("hvigor", "hvigorw assembleHap", "> hvigor Starting\nCompiling entry\nERROR: ArkTS: 10605001 at entry/src/main.ets:12\nBUILD FAILED in 3 s")
add("tests", "npx vitest run", ` RUN  v2\n Tests  0 passed | 40 failed (40)\n${Array.from({ length: 40 }, (_, i) => `✗ case ${i}\n  AssertionError`).join("\n")}`)

// ---------------- 对比 ----------------
const norm = (s) => {
  const t = (s ?? "").trim()
  return t === "" ? "{}" : t
}

let bad = 0
for (const c of cases) {
  const ts = norm(compactByCommand(c.cmd, c.raw))
  const p = spawnSync(tskBin, ["compress", c.cmd], { input: c.raw, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
  if (p.error) {
    console.error(`[fatal] 调用 tsk 失败：${p.error.message}`)
    process.exit(3)
  }
  const rust = norm(p.stdout)
  if (ts === rust) {
    if (verbose) console.log(`✓ ${c.name}`)
  } else {
    bad++
    console.log(`✗ ${c.name}`)
    console.log(`  TS  : ${JSON.stringify(ts.slice(0, 600))}`)
    console.log(`  Rust: ${JSON.stringify(rust.slice(0, 600))}`)
  }
}

console.log(`\n${cases.length - bad}/${cases.length} 与 Rust 一致（TS=${tsPath}）`)
process.exit(bad ? 1 : 0)
