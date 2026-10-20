#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { readRecords } from "../../optional_skills/media_discovery/src/storage.mjs";
import { platformItemId } from "../../optional_skills/media_discovery/src/platforms.mjs";
import { renderCsv, VIDEO_COLUMNS, IMAGE_COLUMNS } from "../../optional_skills/media_discovery/src/csv.mjs";

const [traceFile, summaryFile, workspace, mode = "partial", oracleFile] = process.argv.slice(2);
if (!traceFile || !summaryFile || !workspace || !["partial", "complete"].includes(mode)) {
  throw new Error("Usage: node verify_media_discovery_metrics_regression.mjs <runtime.log> <summary.jsonl> <isolated-workspace> [partial|complete]");
}
const oracleUrl = oracleFile ? new URL(`file://${path.resolve(oracleFile)}`) : new URL("./cases/media_discovery_metrics_live_regression_20261020.json", import.meta.url);
let oracle = JSON.parse(await fs.readFile(oracleUrl, "utf8"));
if (oracle.extends) {
  const base = JSON.parse(await fs.readFile(new URL(oracle.extends, oracleUrl), "utf8"));
  oracle = { ...base, ...oracle, cases: Object.fromEntries([...new Set([...Object.keys(base.cases), ...Object.keys(oracle.cases)])]
    .map(name => [name, { ...base.cases[name], ...oracle.cases[name] }])) };
}
if (oracle.selected_cases) {
  assert.ok(Array.isArray(oracle.selected_cases) && oracle.selected_cases.length, "selected cases required");
  oracle.cases = Object.fromEntries(oracle.selected_cases.map(name => {
    assert.ok(oracle.cases[name], `unknown selected case: ${name}`);
    return [name, oracle.cases[name]];
  }));
}
const trace = (await fs.readFile(traceFile, "utf8")).split("\n");
const summaries = (await fs.readFile(summaryFile, "utf8")).trim().split("\n").filter(Boolean).map(JSON.parse);
const root = path.join(workspace, "data/skills/media_discovery");
let state = { runs: [] }, records = [];
try {
  state = JSON.parse(await fs.readFile(path.join(root, "state.json"), "utf8"));
  records = await readRecords(root);
} catch (error) { if (error.code !== "ENOENT") throw error; }
const normalized = values => [...new Set((values || []).map(c => JSON.stringify([c.metric, c.operator, c.value])))].sort();
const equivalent = (filters = {}, platform, rules) => {
  const groups = [filters, filters.platforms?.[platform]].filter(Boolean);
  if (JSON.stringify(normalized(groups.flatMap(g => g.all || []))) !== JSON.stringify(normalized(rules.all))) return false;
  const expectedAny = normalized(rules.any);
  const actualAny = groups.map(g => normalized(g.any)).filter(g => g.length);
  return expectedAny.length ? actualAny.length > 0 && actualAny.every(g => JSON.stringify(g) === JSON.stringify(expectedAny)) : actualAny.length === 0;
};
const replyFindings = [];
const taskFindings = [];
const checkedRuns = new Map();
const compactNumber = value => String(value).replace(/[,，\s]/gu, "");
// This numeric-presence check is necessary, not a replacement for reviewing label/value semantics.
const replyContainsValue = (reply, metric) => [metric.display, metric.value].filter(value => value != null && String(value).trim()).some(value => {
  const escaped = compactNumber(value).replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  return new RegExp(`(?<![0-9])${escaped}(?![0-9])`, "u").test(compactNumber(reply));
});
let checked = 0;
for (const [name, spec] of Object.entries(oracle.cases)) {
  const summary = summaries.find(row => row.case_name === name);
  if (!summary) { assert.notEqual(mode, "complete", `case missing: ${name}`); continue; }
  const taskPassed = summary.status === "succeeded" && summary.assertion === "pass";
  if (!taskPassed) taskFindings.push({ case: name, status: summary.status, assertion: summary.assertion });
  if (oracle.forbid_filesystem_steps) assert.ok(!trace.some(line => line.includes(`executor_step_start task_id=${summary.task_id} `)
    && /action=capability:filesystem\./u.test(line)), `${name}: unexpected cross-skill filesystem read`);
  const calls = trace.flatMap(line => {
    const match = /executor_step_execute task_id=(\S+).* skill=media_discovery args=(\{.*\})(?: call_id=|$)/u.exec(line);
    return match && match[1] === summary.task_id ? [JSON.parse(match[2])] : [];
  });
  const actions = calls.filter(args => args.action === spec.action);
  assert.ok(actions.length, `${name}: no actual requested dispatch`);
  assert.ok(calls.every(args => [spec.action, "status", "list_runs", ...(spec.allowed_actions || [])].includes(args.action)), `${name}: unexpected skill action`);
  if (spec.action === "run_once") assert.equal(actions.length, 1, `${name}: repeated collection`);
  for (const [platform, rules] of Object.entries(spec.platforms)) {
    assert.ok(actions.some(args => [args.platform, ...(args.platforms || [])].includes(platform)
      && equivalent(args.engagement_filters, platform, rules)
      && (spec.max_items_per_run === undefined || args.max_items_per_run === spec.max_items_per_run)), `${name}: incorrect ${platform} thresholds/quota`);
  }
  const report = { case: name, task_id: summary.task_id, task_passed: taskPassed, thresholds: "passed", actual_dispatches: actions.length };
  if (spec.no_collection_state) {
    assert.equal(state.runs.length, 0, `${name}: unexpected collection run`);
    assert.equal(records.length, 0, `${name}: unexpected saved records`);
    assert.equal(Object.keys(state.active_runs || {}).length, 0, `${name}: unexpected active run`);
    assert.ok(!state.background_worker, `${name}: unexpected background worker`);
    report.no_collection_state = true;
  }
  if (spec.action === "run_once") {
    const platform = Object.keys(spec.platforms)[0];
    assert.deepEqual(actions[0].seed_urls.map(url => platformItemId(platform, url)), oracle.seed_urls[platform].map(url => platformItemId(platform, url)), `${name}: source identities/order`);
    const runs = state.runs.filter(run => run.platforms.length === 1 && run.platforms[0] === platform
      && equivalent(run.platform_configs[platform].engagement_filters, platform, spec.platforms[platform]));
    assert.equal(runs.length, 1, `${name}: unexpected batch count`);
    const run = runs[0];
    checkedRuns.set(name, run);
    assert.equal(run.status, "completed_batch", `${name}: ${run.error_code || run.status}`);
    for (const [key, value] of Object.entries(spec.run_counts)) assert.equal(run.counts[key] || 0, value, `${name}: count ${key}`);
    if (spec.expected_filter_status) assert.ok(run.filter_report?.some(row => row.metric === spec.expected_filter_status.metric
      && row.status === spec.expected_filter_status.status), `${name}: missing availability diagnosis`);
    const saved = records.filter(record => record.platform === platform && record.discovered_at >= run.started_at && record.discovered_at <= run.finished_at);
    if (spec.new_records !== undefined) assert.equal(saved.length, spec.new_records, `${name}: unexpected saved records`);
    if (spec.minimum_new_records) assert.ok(saved.length >= spec.minimum_new_records, `${name}: no saved records`);
    for (const record of saved) {
      assert.ok(typeof record.platform_text === "string" && record.platform_text.trim(), `${name}: missing caption`);
      assert.equal(record.engagement.platform, platform);
      for (const metric of spec.required_metrics || []) assert.ok(record.engagement.metrics[metric], `${name}: missing ${metric}`);
      for (const metric of spec.absent_metrics || []) assert.equal(record.engagement.metrics[metric], undefined, `${name}: invented ${metric}`);
      for (const metric of Object.values(record.engagement.metrics)) if (metric.value !== undefined) assert.ok(Number.isSafeInteger(metric.value) && metric.value >= 0);
    }
    Object.assign(report, { run_id: run.run_id, counts: run.counts, saved_records: saved.length, metrics: [...new Set(saved.flatMap(r => Object.keys(r.engagement.metrics)))].sort() });
    if (spec.verify_reply_values) {
      const missing = saved.flatMap(record => Object.entries(record.engagement.metrics)
        .filter(([, metric]) => !replyContainsValue(summary.text || "", metric))
        .map(([metric]) => ({ global_sequence: record.global_sequence, metric })));
      report.reply_metric_values_present = missing.length === 0;
      report.reply_missing_values = missing;
      if (missing.length) replyFindings.push({ case: name, missing });
    }
  }
  if (spec.action === "get_run_results") {
    const run = checkedRuns.get(spec.source_case);
    assert.ok(run, `${name}: missing source run`);
    assert.ok(actions.every(args => args.run_id === run.run_id), `${name}: wrong saved run queried`);
    const saved = records.filter(record => record.run_id === run.run_id);
    assert.ok(saved.length);
    const missing = saved.flatMap(record => Object.entries(record.engagement.metrics)
      .filter(([, metric]) => !replyContainsValue(summary.text || "", metric))
      .map(([metric]) => ({ global_sequence: record.global_sequence, metric })));
    Object.assign(report, { source_run_id: run.run_id, reply_metric_values_present: missing.length === 0, reply_missing_values: missing });
    if (missing.length) replyFindings.push({ case: name, missing });
  }
  console.log(JSON.stringify(report));
  checked += 1;
}
if (mode === "complete") {
  assert.equal(checked, Object.keys(oracle.cases).length);
  const noCollection = Object.values(oracle.cases).every(spec => spec.no_collection_state);
  if (!noCollection) {
    for (const [kind, columns, file] of [["video", VIDEO_COLUMNS, "videos.csv"], ["image", IMAGE_COLUMNS, "images.csv"]]) {
      assert.equal(await fs.readFile(path.join(root, "exports", file), "utf8"), renderCsv(columns, records.filter(record => record.kind === kind)), `${file}: ledger/CSV mismatch`);
    }
  }
  assert.equal(Object.keys(state.active_runs || {}).length, 0);
  assert.ok(!state.background_worker, "unexpected continuous collection");
  const passed = replyFindings.length === 0 && taskFindings.length === 0;
  console.log(JSON.stringify({ complete: true, passed, checked, total_records: records.length, csv_consistent: noCollection ? null : true,
    csv_verification: noCollection ? "not_applicable_no_collection" : "passed", no_active_run: true, task_findings: taskFindings, reply_findings: replyFindings }));
  if (!passed) process.exitCode = 1;
}
