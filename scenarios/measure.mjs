#!/usr/bin/env node
// Summarize a captured `claude -p` output stream (aggregated `--output-format json`
// OR per-step `--output-format stream-json`). Sums real input tokens across steps:
// input_tokens + cache_creation + cache_read. With `--tools`, also lists tool names.
//
// Usage: node measure.mjs <capture> [--tools|--steps]
import fs from 'node:fs';

const file = process.argv[2];
const wantTools = process.argv.includes('--tools');
const wantSteps = process.argv.includes('--steps');
const lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean);

let input = 0, cacheCreate = 0, cacheRead = 0, output = 0;
let sessionId = null, cost = 0, numTurns = null, terminalReason = null, isError = null;
const toolUse = {};       // tool name -> count
const steps = [];
let aggregatedSeen = false;

for (const ln of lines) {
  let obj;
  try { obj = JSON.parse(ln); } catch { continue; }
  // aggregated `--output-format json`: single object with top-level usage + num_turns
  if (obj.usage && obj.usage.input_tokens !== undefined && obj.num_turns !== undefined) {
    aggregatedSeen = true;
    sessionId = obj.session_id ?? sessionId;
    numTurns = obj.num_turns; terminalReason = obj.terminal_reason; isError = obj.is_error;
    cost += obj.total_cost_usd ?? 0;
    const u = obj.usage;
    input += u.input_tokens ?? 0; cacheCreate += u.cache_creation_input_tokens ?? 0;
    cacheRead += u.cache_read_input_tokens ?? 0; output += u.output_tokens ?? 0;
    continue;
  }
  // stream-json: assistant events carry message.usage + message.content (tool_use)
  const mu = obj?.message?.usage;
  if (mu && mu.input_tokens !== undefined) {
    if (!sessionId && obj.session_id) sessionId = obj.session_id;
    numTurns = obj.num_turns ?? numTurns;
    input += mu.input_tokens ?? 0; cacheCreate += mu.cache_creation_input_tokens ?? 0;
    cacheRead += mu.cache_read_input_tokens ?? 0; output += mu.output_tokens ?? 0;
    steps.push({ input: mu.input_tokens, output: mu.output_tokens });
  }
  // collect tool_use
  const content = obj?.message?.content ?? obj?.content ?? [];
  for (const c of (Array.isArray(content) ? content : [])) {
    if (c?.type === 'tool_use' && c.name) toolUse[c.name] = (toolUse[c.name] || 0) + 1;
  }
}

const result = {
  parsed_as: aggregatedSeen ? 'aggregated' : 'stream',
  session_id: sessionId,
  num_turns: numTurns,
  terminal_reason: terminalReason, is_error: isError,
  cost_usd: Number(cost.toFixed(4)),
  total_input_tokens: input + cacheCreate + cacheRead,
  input_tokens: input, cache_creation_tokens: cacheCreate, cache_read_tokens: cacheRead,
  output_tokens: output,
};
if (wantTools) result.tools = toolUse;
if (wantSteps) result.step_count = steps.length;
console.log(JSON.stringify(result, null, 2));