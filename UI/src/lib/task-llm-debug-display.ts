import type { TaskLlmDebugCall, TaskLlmDebugEntry } from "../types/api";

function compactMetaValue(value: string | number | null | undefined): string | null {
  if (value == null) return null;
  const text = String(value).trim();
  return text ? text : null;
}

export function taskLlmDebugCallEntry(call: TaskLlmDebugCall): TaskLlmDebugEntry {
  return call.entry ?? call;
}

export function taskLlmDebugUsageTokens(call: TaskLlmDebugCall): string[] {
  const usage = taskLlmDebugCallEntry(call).usage;
  if (!usage) return [];
  const promptTokens = usage.prompt_tokens ?? usage.input_tokens;
  const completionTokens = usage.completion_tokens ?? usage.output_tokens;
  return [
    promptTokens != null ? `prompt=${promptTokens}` : null,
    completionTokens != null ? `completion=${completionTokens}` : null,
    usage.reasoning_tokens != null ? `reasoning=${usage.reasoning_tokens}` : null,
    usage.cached_tokens != null ? `cached=${usage.cached_tokens}` : null,
    usage.total_tokens != null ? `total=${usage.total_tokens}` : null,
  ].filter((item): item is string => Boolean(item));
}

export function taskLlmDebugCallMetaTokens(call: TaskLlmDebugCall): string[] {
  const entry = taskLlmDebugCallEntry(call);
  return [
    compactMetaValue(entry.status) ? `status=${entry.status}` : null,
    compactMetaValue(entry.model) ? `model=${entry.model}` : null,
    compactMetaValue(entry.provider) ? `provider=${entry.provider}` : null,
    compactMetaValue(entry.vendor) ? `vendor=${entry.vendor}` : null,
    compactMetaValue(entry.prompt_source ?? entry.prompt_file)
      ? `stage=${entry.prompt_source ?? entry.prompt_file}`
      : null,
    compactMetaValue(entry.call_id) ? `call_id=${entry.call_id}` : null,
    compactMetaValue(entry.ts) ? `ts=${entry.ts}` : null,
    ...taskLlmDebugUsageTokens(call),
  ].filter((item): item is string => Boolean(item));
}

export function taskLlmDebugRawFields(call: TaskLlmDebugCall): string {
  const entry = taskLlmDebugCallEntry(call);
  const entryKeys = Object.keys(entry)
    .filter((key) => entry[key as keyof TaskLlmDebugEntry] != null)
    .sort()
    .map((key) => (call.entry ? `entry.${key}` : key));
  const callKeys = Object.keys(call)
    .filter((key) => key !== "entry" && call[key as keyof TaskLlmDebugCall] != null)
    .sort();
  return [...new Set([...callKeys, ...entryKeys])].join(", ");
}

export function taskLlmDebugRequestData(call: TaskLlmDebugCall): unknown {
  const entry = taskLlmDebugCallEntry(call);
  return entry.request_payload ?? entry.prompt;
}

export function taskLlmDebugResponseData(call: TaskLlmDebugCall): unknown {
  const entry = taskLlmDebugCallEntry(call);
  return entry.raw_response ?? entry.clean_response ?? entry.response ?? entry.error;
}
