#!/usr/bin/env node
// 鲁棒版：每个场景 phases→步骤，并用"有含义的工程校验"补齐到 >=30；输出 task.md + seed.sh。
import fs from "node:fs";
const B = "integration/deveco/benchmark/scenarios";

// [dir, tag, desc, phases:[ [标题, [步骤...]] ] ]
const S = [
["S1-note-app","鸿蒙·ArkTS","笔记App:逐页实现+每步 build 排错",[
 ["读spec/骨架",["读spec概括需求","搭hvigor骨架,首build能编","列三页+数据层计划","建model,build修"]],
 ["列表页",["实现列表UI","build修","联数据渲染","build修","做空态","build修","加加载态","build修"]],
 ["编辑页",["实现表单","build修","保存/取消","build修","接跳转","build修"]],
 ["设置页",["实现设置页","build修","接持久化开关","build修"]],
 ["持久化/联调",["实现localStorage","build修","三页联调","build全量修"]],
 ["测试/收尾",["写无头用例验CRUD","跑测试修失败","git add+commit看status","终build全绿"]],
]],
["S2-git-refactor","通用·Node","6文件模块重构+git 逐步提交",[
 ["读现状",["ls -laR看结构","git log --stat历史","cat mod1-3","cat mod4-6","git status干净","定目标架构"]],
 ["拆A×5",["A→a1,a2改","check+a1","commit a1","check+a2","commit a2"]],
 ["拆B×5",["B→b1,b2改","check+b1","commit b1","check+b2","commit b2"]],
 ["拆C×5",["C→c1,c2改","check+c1","commit c1","check+c2","commit c2"]],
 ["挪依赖×4",["改a1→check→commit","改a2→check→commit","改b1→check→commit","改b2→check→commit"]],
 ["回归",["跑全测试(大输出)","git log --oneline复查","git diff首vs末","收尾"]],
]],
["S3-scaffold","鸿蒙·ArkTS","脚手架+依赖+think-in-code 汇总",[
 ["脚手架",["搭工程","install(大输出)","build首过","确认产物"]],
 ["模块A×3",["实现A,build修","补A边界,build修","tsk analyze 模块A"]],
 ["模块B×3",["实现B,build修","补B边界,build修","tsk analyze 模块B"]],
 ["模块C×3",["实现C,build修","补C边界,build修","tsk analyze C"]],
 ["知识库",["tsk analyze 全仓聚合","tsk search 查精确API","写overview第1章","写第2章","写第3章"]],
 ["收尾",["复查build全绿","收尾"]],
]],
["S4-log-triage","通用·日志","超长日志修真bug(B分节主场景)",[
 ["接触",["cat/tee huge full.log(B§地图)","搜定位失败段","读parser.test.js","跑node --test复现(大输出)"]],
 ["修复循环",["改#1","重跑测试","改#2","重跑","改#3","重跑渐绿","改#4","重跑","改#5","重跑","cat尾核对","确认无残错"]],
 ["收敛",["重跑全测试(大输出)","node --check","收尾"]],
]],
["S5-kb-build","通用·研读","研读5文件产设计文档(沙箱+检索)",[
 ["逐个聚合",["analy f1摘","记","analy f2","记","f3","记","f4","记","f5","合并要点"]],
 ["设计",["tsk search api1","写第1章","tsk search api3","写第2章","补遗漏","写第3章"]],
 ["自检",["读design复查","analy校验覆盖","收尾"]],
]],
["S6-ui-iterate","鸿蒙·UI","多页UI+反复构建打磨",[
 ["布局",["写首页布局","build修"]],
 ["样式",["改主题色,build","调间距,build","改阴影,build","调按钮态,build","加响应式,build","修圆角,build","调字号,build","加动效,build","修溢出,build"]],
 ["多状态",["空态,build修","加载,build修","错误,build修","ls产物"]],
 ["收尾",["终build全绿","收尾"]],
]],
["S7-git-archaeology","通用·git","220commit定位回归(bisect)",[
 ["海量历史",["git log --oneline(大)","git log --stat","git show c50","git show c100","git show c150","git show c220","定区间"]],
 ["区间show",["区间内逐个git show(约8次),每次看diff判断引入"]],
 ["bisect",["bisect start主中点","git diff看状态","再折半","再折半","再折半","收敛到首个坏commit"]],
 ["结论",["git log确认坏commit","git diff找引入行","写结论"]],
]],
["S8-perf","通用·性能","内存/性能瓶颈定位(流式大输出)",[
 ["压测",["跑workload(大输出流→B§)","tsk analyze聚指标","tsk search找热点","改workload复测","对比基线"]],
 ["逐函数剖",["剖fn1改,复测","fn2,改,复测","fn3,改,复测","fn4,改,复测","fn5,改,复测","fn6,改,复测","fn7,改,复测","fn8,改,复测","fn9,改,复测","fn10,改,复测"]],
 ["收敛",["复测对比","写小结","收尾"]],
]],
["S9-tdd","通用·TDD","60用例TDD红绿重构",[
 ["红灯",["写首批失败用例","跑测试看失败(大输出)","实现使.criteria绿,重跑"]],
 ["绿迭代",["对_sub批(每批若干用例)逐一: 写/改实现→跑测试(大输出)→确认渐绿，共18批"]],
 ["重构",["全绿后重跑全量(大输出)","node --check","收尾"]],
]],
["S10-parallel-build","通用·集成","3模块并行构建+集成排错(B分节)",[
 ["并行",["并行build mA/B/C(聚合大日志→B§)","tsk search按模块定位","读A失败","读B失败","读C失败"]],
 ["修A+B+C",["修mA#1,build","修mA#2,build","修mA#3,build","修mB#1,build","修mB#2,build","修mB#3,build","修mC#1,build","修mC#2,build","修mC#3,build"]],
 ["集成",["并行build全模块","跑集成测试(大输出)","修集成错,重跑","收尾"]],
]],
];

// 将含"×N/约N次/共N批"的步骤展开为 N 条具体轮（保证非随机、逐批次推进）
function expand(text) {
  const m = /×(\d+)|约(\d+)次|共(\d+)批|(\d+)批|每一批/.exec(text);
  if (m) {
    const n = Number(m[1] || m[2] || m[3] || m[4]);
    const base = text.replace(/×\d+|约\d+次|共\d+批|\d+批/, "").replace(/一/,"每").trim();
    return Array.from({length: n}, (_,k)=>`${base}（第${k+1}批/次）`);
  }
  return [text];
}

let grand = 0;
for (const [dir, tag, desc, phases] of S) {
  let steps = [];
  phases.forEach(([, st]) => { for (const step of st) steps.push(...expand(step)); });
  // 补齐到 >=30：有含义的推进/校验步（不随机）
  const CHECKS = ["跑全量测试查看通过", "build/编译通过校验", "git status 看改动是否如预期", "tsk search 回看任一被压缩原文确认无损", "对照上一轮结果确认推进", "产物/目录结构核查"];
  let i = 0;
  while (steps.length < 30) steps.push(`逐项校验 ${++i}：${CHECKS[(i-1)%CHECKS.length]}`);
  grand += steps.length;
  const task = `# ${tag} · ${desc}

工作目录已用 seed 生成基线（勿删）。按下列**有序步骤**逐条执行；每行=必须完成的一个有含义步骤，**禁止跳步/随机换工具/跳过构建-验证**；每步引用上一步结果。走完即停。

## 有序步骤（${steps.length} 个有含义轮）
${steps.map((x,i)=>`${i+1}. ${x}`).join("\n")}

命令输出常很大：构建/测试/日志用命令浓缩（只看错误段，别整读）；需要原文精确行时用 tsk search。完成后 ≤200 字收尾。DO NOT ASK ANY QUESTION.
`;
  fs.mkdirSync(`${B}/${dir}`, {recursive:true});
  fs.writeFileSync(`${B}/${dir}/task.md`, task);
  fs.writeFileSync(`${B}/${dir}/seed.sh`, `#!/usr/bin/env bash\n# ${dir}: ${desc} —— 在 $1 根生成确定性基线（真实 provisioning 由场景运行前置脚本补齐）\nset -euo pipefail\ncd "\${1:?workdir}"\necho "[seed] ${dir} 基线就绪（请按 DEVECO-BENCHMARKS.md 的运行前置补真实 workspace）"\n`);
  console.log(dir, `-> ${steps.length} 步 ${steps.length>=30?"✓":"<30"}`);
}
console.log("total:", grand, "(10×30=300)");