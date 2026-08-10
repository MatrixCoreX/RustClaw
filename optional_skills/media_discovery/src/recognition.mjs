import fs from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";

const PROMPT_PATH = path.join("prompts", "layers", "overlays", "image_text_revision_prompt.md");
const MAX_REVIEW_CHARS = 12_000;

function runProcess(program, args, timeoutMs) {
  return new Promise((resolve) => {
    const child = spawn(program, args, { stdio: ["ignore", "pipe", "pipe"] });
    const stdout = [];
    const stderr = [];
    const timer = setTimeout(() => child.kill("SIGTERM"), timeoutMs);
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.on("error", (error) => {
      clearTimeout(timer);
      resolve({ ok: false, code: null, stdout: "", stderr: String(error) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({
        ok: code === 0,
        code,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    });
  });
}

async function tesseractScreenshot(imagePath) {
  const common = [imagePath, "stdout", "--psm", "6"];
  const multilingual = await runProcess("tesseract", [...common, "-l", "eng+chi_sim"], 120_000);
  if (multilingual.ok) return { text: multilingual.stdout.trim(), language_set: "eng+chi_sim" };
  const fallback = await runProcess("tesseract", [...common, "-l", "eng"], 120_000);
  if (fallback.ok) return { text: fallback.stdout.trim(), language_set: "eng" };
  return { text: "", language_set: null, error_code: "ocr_unavailable" };
}

async function reviewPrompt(rawText) {
  const workspace = process.env.WORKSPACE_ROOT;
  if (!workspace) return null;
  const template = await fs.readFile(path.join(workspace, PROMPT_PATH), "utf8").catch(() => null);
  if (!template) return null;
  return template
    .replaceAll("__CHUNK_INDEX__", "1")
    .replaceAll("__CHUNK_COUNT__", "1")
    .replace("__RAW_RECOGNIZED_TEXT__", rawText.slice(0, MAX_REVIEW_CHARS));
}

async function modelReview(rawText) {
  const url = process.env.AGENT_INTERNAL_LLM_URL?.trim();
  const token = process.env.AGENT_INTERNAL_LLM_TOKEN?.trim();
  const prompt = await reviewPrompt(rawText);
  if (!url || !token || !prompt) return { text: rawText, status: "fallback_raw" };
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 120_000);
  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-agent-internal-llm-token": token,
      },
      body: JSON.stringify({
        skill_name: "media_discovery",
        prompt_source: "skills/media_discovery/image_text_revision",
        prompt,
        temperature: 0,
        max_tokens: 8192,
      }),
      signal: controller.signal,
    });
    const payload = await response.json();
    const data = response.ok && payload?.ok === true ? payload.data : null;
    if (typeof data?.text !== "string" || !data.text.trim()) {
      return { text: rawText, status: "fallback_raw", error_code: "model_review_empty" };
    }
    return {
      text: data.text.trim(),
      status: "reviewed",
      provider: data.provider || null,
      model: data.model || null,
    };
  } catch {
    return { text: rawText, status: "fallback_raw", error_code: "model_review_failed" };
  } finally {
    clearTimeout(timer);
  }
}

export async function recognizeScreenshot(imagePath, mode) {
  if (mode === "metadata_only") {
    return { raw_text: "", text: "", source: "metadata_only", review: { status: "skipped" } };
  }
  const ocr = await tesseractScreenshot(imagePath);
  if (!ocr.text) {
    return {
      raw_text: "",
      text: "",
      source: "local_ocr",
      error_code: ocr.error_code || "recognition_unavailable",
      review: { status: "skipped" },
    };
  }
  const review = mode === "ocr_reviewed" ? await modelReview(ocr.text) : { text: ocr.text, status: "skipped" };
  return {
    raw_text: ocr.text,
    text: review.text,
    source: "local_ocr",
    language_set: ocr.language_set,
    review: { ...review, text: undefined },
  };
}
