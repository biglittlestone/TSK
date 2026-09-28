#!/usr/bin/env node
// Generate a LARGE structured reference document for the sandbox-exec benchmark
// scenario 04. It's a realistic multi-section technical/business spec the model
// must turn into a small structured overview. ~200KB+ so wholesale reading is
// genuinely expensive and /tsk-analyze (sandbox summary) is the efficient path.
import fs from 'node:fs';

const SECTIONS = [
  '总体架构', '数据模型', '认证与权限', '订单子系统', '库存子系统',
  '支付与结算', '消息与通知', '报表与统计', '安全与合规', '运维与监控',
];
const SUBS = ['需求', '接口', '表结构', '状态机', '约束', '测试要点', '边界与异常', '依赖'];
const VERBS = ['须', '不得', '应', '可', '必须', '禁止'];

let out = ['发布项目：星核平台（Starbuild）—— 技术规格与需求说明书 v3.2', ''];
out.push(`序言：本文档为星核平台全量需求与设计规格，共 ${SECTIONS.length} 个大域。用于评审、排期与验收。面内容采用统一结构：每节含「需求条目 + 接口契约 + 数据表 + 边界用例」。`);
out.push('');

SECTIONS.forEach((sec, si) => {
  out.push(`# ${si + 1}. ${sec}`);
  out.push(`本域覆盖 ${sub2k(sec)} 个能力点，接口 ${60 + si * 3} 个，状态机 ${4 + (si % 3)} 个。`);
  out.push('');
  SUBS.forEach((sub, k) => {
    out.push(`## ${si + 1}.${k + 1} ${sec}·${sub}`);
    out.push(`本小节编号 REQ-${String(si + 1).padStart(2, '0')}-${String(k + 1).padStart(2, '0')}，共 ${3 + ((si * 7 + k) % 6)} 条需求。`);
    const n = 3 + ((si * 7 + k) % 6);
    for (let r = 0; r < n; r++) {
      const verb = VERBS[(si * 3 + k + r) % VERBS.length];
      out.push(`- REQ-${String(si + 1).padStart(2, '0')}-${String(k + 1).padStart(2, '0')}-${r + 1}：系统${verb}在 ${sub} 场景下，当 ${ev(si, k, r)} 时，${action(si, k, r)}，预期耗时 ≤ ${20 + ((si * 11 + k * 3 + r) % 480)} ms，P${1 + ((si + k + r) % 3)}。`);
    }
    out.push('接口契约：`POST /api/' + slug(sec) + '/' + (k + 1) + '` 入参 `{reqId, seq, flag}`，出参 `{code, data, ts}`；错误码见下表。');
    out.push('');
    out.push('| 错误码 | 含义 | 处理 |');
    for (let e = 0; e < 4; e++) {
      out.push(`| E-${si}${k}${e}0${e} | ${sub}第${e + 1}类异常 | ${e % 2 ? '重试' : '返回提示并记日志'} |`);
    }
    out.push('');
    // some prose for real length/summarizability
    for (let p = 0; p < 2; p++) {
      out.push(`${sec}域${sub}设计的核心矛盾在于一致性成本与吞吐的平衡：在默认强一致的前提下，允许对 ${k % 2 ? '读多写少' : '写多读少'} 的路径放宽为最终一致。实现上采用 ${k % 3 === 0 ? '两阶段提交' : k % 3 === 1 ? '事件溯源' : '本地消息表'}，配合 ${(si % 2) ? '分布式锁' : '乐观版本号'} 兜底。对于超时与重放，通过幂等键 ${'KEY-' + slug(sec).toUpperCase() + '-' + (k + 1)} 去重；失败补偿由补偿队列异步执行并回写状态。需要重点回归的链路见 §${si + 1}.${(k + 1) % 8 + 1} 的边界用例。`);
    }
    out.push('');
  });
  // a canonical fact line the model must surface (so the summary has specific content)
  out.push(`> 本域权威基线（必需在交付报告中出现）：${sec} 的线上容量目标为 ${8000 + si * 733} QPS，SLO 为 ${(99 + (si % 2) * 0.5).toFixed(1)}9%；主责任人 ${'李' + String.fromCharCode(65 + si)}。`);
  out.push('');
});

fs.writeFileSync('E:/00work/TSK/scenarios/04-sandbox-doc/reference/spec.md', out.join('\n'));
console.log(`wrote ${(out.join('\n').length / 1024).toFixed(0)} KB → 04-sandbox-doc/reference/spec.md (${out.length} lines)`);

function slug(s) { return s.replace(/[^\u4e00-\u9fa5A-Za-z]/g, '').toLowerCase() || 'sec'; }
function sub2k(s) { return (s.length % 6) + 3; }
function ev(si, k, r) { return `网关在 ${si * 5 + k + 1} 号实例上收到 seq=${si}${k}${r} 的请求`; }
function action(si, k, r) { return `${k % 2 ? '降级到备用通道并记录 traceId' : '按主链路完成并返回摘要'}，成功后更新 ${si + 1}.${k + 1} 域的缓存`; }