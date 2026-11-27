"use strict";
const labels = {
  zh: {
    title: "音视频转写", intro: "把录音或视频里的语音转成文字。", uploadLabel: "选择音频或视频",
    start: "开始转写", resultTitle: "转写结果", copy: "复制文字", save: "下载 TXT",
    historyTitle: "我的转写记录", refresh: "刷新", more: "查看更多",
    limit: "单个文件最大 {size} MB。", idle: "上传文件，或从历史记录打开转写结果。",
    opening: "正在连接…", listing: "正在加载记录…", empty: "还没有转写记录。",
    uploading: "正在上传…", queued: "等待转写", running: "正在转写", retry_wait: "等待重试",
    succeeded: "已完成", failed: "转写失败", canceled: "已取消", unknown: "正在处理",
    copied: "已复制。", copyManual: "已选中文字，请使用系统的复制功能。",
    invalid: "请选择音频或视频文件。", tooLarge: "文件超过大小限制，请压缩或截取后重试。",
    busy: "另一个文件正在上传，请稍候。", unreadable: "暂时无法读取结果，请刷新后重试。",
    uploadFailed: "上传或提交失败，请重试。", openFailed: "暂时无法连接，请刷新页面。",
    fileExpired: "上传的文件已过期，请重新上传。", decodeFailed: "无法读取音轨，请换用可正常播放的文件。",
    providerFailed: "转写服务暂时无法处理此文件，请确认录音有清晰语音后重试。",
    unavailable: "应用或转写服务当前不可用，请联系管理员。", noName: "未命名文件",
  },
  en: {
    title: "Audio and video transcription", intro: "Turn speech from recordings or videos into text.",
    uploadLabel: "Choose an audio or video file", start: "Transcribe", resultTitle: "Transcript",
    copy: "Copy text", save: "Download TXT", historyTitle: "My transcripts", refresh: "Refresh", more: "Load more",
    limit: "Maximum file size is {size} MB.", idle: "Upload a file or open a transcript from your history.",
    opening: "Connecting…", listing: "Loading history…", empty: "No transcripts yet.", uploading: "Uploading…",
    queued: "Waiting to transcribe", running: "Transcribing", retry_wait: "Waiting to retry", succeeded: "Complete",
    failed: "Transcription failed", canceled: "Canceled", unknown: "Processing", copied: "Copied.",
    copyManual: "The text is selected. Use your device’s copy command.",
    invalid: "Choose an audio or video file.", tooLarge: "This file is too large. Compress or trim it and try again.",
    busy: "Another upload is in progress.", unreadable: "Couldn’t open the result. Refresh and try again.",
    uploadFailed: "The upload or submission failed. Try again.", openFailed: "Couldn’t connect. Refresh the page.",
    fileExpired: "The uploaded file has expired. Upload it again.",
    decodeFailed: "Couldn’t read the audio track. Try a file that plays correctly.",
    providerFailed: "The transcription service couldn’t process this file. Check that the recording contains clear speech and retry.",
    unavailable: "The app or transcription service is unavailable. Contact your administrator.", noName: "Untitled file",
  },
};

const get = (id) => document.getElementById(id);
const pending = new Map();
const records = new Map();
const settled = new Set(["succeeded", "failed", "canceled", "cancelled"]);
const file = get("file");
let words = labels.zh;
let language = "zh";
let maxBytes = 0;
let uploading = false;
let loadingHistory = false;
let selected = null;
let active = null;
let next = null;
let pollTimer;
let openingTimer;

function request(type, details = {}, timeout = 15000) {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  const id = hex.replace(/^(........)(....)(....)(....)/, "$1-$2-$3-$4-");
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error("aipp_request_timeout"));
    }, timeout);
    pending.set(id, { resolve, reject, timer });
    parent.postMessage({ schema_version: 1, type, id, ...details }, "*");
  });
}

function status(id, text, error = false) {
  const element = get(id);
  element.textContent = text;
  element.classList.toggle("error", error);
}

function statusLabel(value) {
  const key = value === "cancelled" ? "canceled" : value;
  return Object.hasOwn(words, key) ? words[key] : words.unknown;
}

function submitError(code) {
  if (code === "ui_attachment_too_large") return words.tooLarge;
  if (code === "ui_attachment_upload_busy") return words.busy;
  if (code === "aipp_transcript_attachment_invalid") return words.invalid;
  if (code === "aipp_not_available" || code === "aipp_package_changed") return words.unavailable;
  return words.uploadFailed;
}

function jobError(code) {
  if (code === "aipp_not_available" || code === "workspace_scope_unavailable") return words.unavailable;
  if (code === "aipp_transcript_attachment_invalid" || code?.startsWith("ui_attachment_")) return words.fileExpired;
  if (code?.startsWith("video_")) return words.decodeFailed;
  return words.providerFailed;
}

function redrawHistory() {
  const list = get("history");
  list.replaceChildren();
  get("more").hidden = next == null;
  for (const job of records.values()) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    const title = document.createElement("span");
    const detail = document.createElement("small");
    button.className = "job";
    button.type = "button";
    if (job.task_id === selected) button.setAttribute("aria-current", "true");
    title.textContent = job.name || words.noName;
    const created = new Date(job.created_at ? job.created_at * 1000 : NaN);
    const date = Number.isNaN(created.valueOf()) ? "" : new Intl.DateTimeFormat(language, {
      dateStyle: "short", timeStyle: "short",
    }).format(created);
    detail.textContent = [statusLabel(job.status), date].filter(Boolean).join(" · ");
    button.append(title, detail);
    button.addEventListener("click", () => { void openJob(job.task_id); });
    item.append(button);
    list.append(item);
  }
}

async function refreshHistory(before) {
  if (loadingHistory) return;
  loadingHistory = true;
  get("refresh").disabled = true;
  get("more").disabled = true;
  status("historyStatus", words.listing);
  try {
    const page = await request("transcript.list", { before });
    if (!before) records.clear();
    for (const job of page.items) records.set(job.task_id, job);
    next = page.next;
    redrawHistory();
    status("historyStatus", records.size ? "" : words.empty);
  } catch {
    status("historyStatus", words.unreadable, true);
  } finally {
    loadingHistory = false;
    get("refresh").disabled = false;
    get("more").disabled = false;
  }
}

function showText(text) {
  get("transcript").value = text;
  get("transcript").hidden = !text;
  get("copy").disabled = !text;
  get("save").disabled = !text;
}

async function openJob(taskId, fromPoll = false, failures = 0) {
  clearTimeout(pollTimer);
  if (!fromPoll) {
    selected = taskId;
    active = null;
    showText("");
    status("resultStatus", words.unknown);
    redrawHistory();
  }
  try {
    const job = await request("transcript.read", { taskId });
    if (selected !== taskId) return;
    active = job;
    const previous = records.get(job.task_id);
    if (previous && previous.status !== job.status) {
      records.set(job.task_id, { ...previous, status: job.status });
      redrawHistory();
    }
    if (job.status === "succeeded" && job.text) showText(job.text);
    if (job.status === "failed") status("resultStatus", jobError(job.error_code), true);
    else status("resultStatus", [job.name, statusLabel(job.status)].filter(Boolean).join(" · "));
    if (!settled.has(job.status)) pollTimer = setTimeout(() => { void openJob(taskId, true); }, 2000);
  } catch {
    if (selected !== taskId) return;
    status("resultStatus", words.unreadable, true);
    if (fromPoll && failures < 2) {
      pollTimer = setTimeout(() => { void openJob(taskId, true, failures + 1); }, 3500);
    }
  }
}

function updateSubmit() {
  get("start").disabled = uploading || !file.files.length || !maxBytes;
  file.disabled = uploading || !maxBytes;
}

file.addEventListener("change", () => {
  status("uploadStatus", "");
  updateSubmit();
});
get("start").addEventListener("click", async () => {
  const chosen = file.files[0];
  if (!chosen || uploading) return;
  if (!/^(audio|video)\//.test(chosen.type) || !chosen.size) {
    status("uploadStatus", words.invalid, true);
    return;
  }
  if (chosen.size > maxBytes) {
    status("uploadStatus", words.tooLarge, true);
    return;
  }
  uploading = true;
  updateSubmit();
  status("uploadStatus", words.uploading);
  try {
    const job = await request("transcript.submit", { file: chosen }, 180000);
    status("uploadStatus", "");
    file.value = "";
    void refreshHistory();
    void openJob(job.task_id);
  } catch (error) {
    status("uploadStatus", submitError(error.message), true);
  } finally {
    uploading = false;
    updateSubmit();
  }
});

get("refresh").addEventListener("click", () => {
  void refreshHistory();
  if (selected) void openJob(selected);
});
get("more").addEventListener("click", () => { if (next) void refreshHistory(next); });
get("copy").addEventListener("click", async () => {
  const output = get("transcript");
  if (!output.value) return;
  try {
    await navigator.clipboard.writeText(output.value);
    status("resultStatus", words.copied);
  } catch {
    output.focus();
    output.select();
    const copied = document.execCommand("copy");
    status("resultStatus", copied ? words.copied : words.copyManual);
  }
});
get("save").addEventListener("click", () => {
  if (!active?.text) return;
  const url = URL.createObjectURL(new Blob([active.text], { type: "text/plain;charset=utf-8" }));
  const a = document.createElement("a");
  const stem = (active.name || "transcript").replace(/\.[^.]+$/, "").replace(/[^\p{L}\p{N}._ -]/gu, "_");
  a.href = url;
  a.download = (stem || "transcript") + ".txt";
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
});

window.addEventListener("message", (event) => {
  const message = event.data;
  if (event.source !== parent || message?.schema_version !== 1) return;
  if (message.type === "transcript.context" && !maxBytes && Number.isSafeInteger(message.maxBytes) && message.maxBytes > 0) {
    clearTimeout(openingTimer);
    language = message.lang === "en" ? "en" : "zh";
    words = labels[language];
    document.documentElement.lang = language === "en" ? "en" : "zh-CN";
    if (message.theme === "light" || message.theme === "dark") document.documentElement.dataset.theme = message.theme;
    for (const id of ["title", "intro", "uploadLabel", "start", "resultTitle", "copy", "save", "historyTitle", "refresh", "more"]) {
      get(id).textContent = words[id];
    }
    document.title = words.title;
    maxBytes = message.maxBytes;
    get("limit").textContent = words.limit.replace("{size}", String(maxBytes / 1048576));
    status("uploadStatus", "");
    status("resultStatus", words.idle);
    updateSubmit();
    void refreshHistory();
  }
  if (message.type === "transcript.result") {
    const handler = pending.get(message.id);
    if (!handler) return;
    pending.delete(message.id);
    clearTimeout(handler.timer);
    if (message.ok) handler.resolve(message.data);
    else handler.reject(new Error(message.error || "aipp_request_failed"));
  }
});
status("uploadStatus", words.opening);
openingTimer = setTimeout(() => status("uploadStatus", words.openFailed, true), 10000);
parent.postMessage({ schema_version: 1, type: "transcript.ready" }, "*");
