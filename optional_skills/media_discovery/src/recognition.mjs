import fs from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";

const PROMPT_PATH = path.join("prompts", "layers", "overlays", "image_text_revision_prompt.md");
const REVISION_CHUNK_CHARS = 6_000;

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

export function parseTesseractLanguages(output) {
  const languages = [];
  for (const line of String(output).split(/\r?\n/u)) {
    const candidate = line.trim();
    if (!/^[A-Za-z0-9_.\-/]+$/u.test(candidate)) continue;
    if (candidate === "osd" || candidate === "equ") continue;
    if (!languages.includes(candidate)) languages.push(candidate);
  }
  return languages.sort();
}

async function tesseractScreenshot(imagePath) {
  const common = [imagePath, "stdout", "--psm", "6"];
  const inventory = await runProcess("tesseract", ["--list-langs"], 15_000);
  const languages = inventory.ok ? parseTesseractLanguages(inventory.stdout) : [];
  if (languages.length === 0) {
    return { text: "", language_set: null, error_code: "ocr_language_data_unavailable" };
  }
  const languageSet = languages.join("+");
  const recognition = await runProcess("tesseract", [...common, "-l", languageSet], 120_000);
  if (recognition.ok) return { text: recognition.stdout.trim(), language_set: languageSet };
  return { text: "", language_set: null, error_code: "ocr_unavailable" };
}

async function reviewTemplate() {
  const workspace = process.env.WORKSPACE_ROOT;
  if (!workspace) return null;
  return fs.readFile(path.join(workspace, PROMPT_PATH), "utf8").catch(() => null);
}

export function splitRevisionChunks(text, maxChars = REVISION_CHUNK_CHARS) {
  const characters = Array.from(String(text).trim());
  const limit = Math.max(1, Number(maxChars) || REVISION_CHUNK_CHARS);
  const chunks = [];
  let start = 0;
  while (start < characters.length) {
    let end = Math.min(start + limit, characters.length);
    if (end < characters.length) {
      const floor = start + Math.floor(limit / 2);
      for (let index = end - 1; index >= floor; index -= 1) {
        if (/\s/u.test(characters[index])) {
          end = index + 1;
          break;
        }
      }
    }
    const chunk = characters.slice(start, end).join("");
    if (chunk.trim()) chunks.push(chunk);
    start = end;
  }
  return chunks;
}

function numericTokens(text) {
  return String(text).match(/\p{N}+/gu) || [];
}

export function revisionPreservesSource(rawText, reviewedText) {
  const raw = String(rawText);
  const reviewed = String(reviewedText);
  if (!reviewed.trim()) return false;
  if (JSON.stringify(numericTokens(raw)) !== JSON.stringify(numericTokens(reviewed))) return false;
  const rawCount = Array.from(raw).filter((character) => !/\s/u.test(character)).length;
  const reviewedCount = Array.from(reviewed).filter((character) => !/\s/u.test(character)).length;
  if (rawCount < 40) return reviewedCount <= rawCount * 2 + 16;
  return reviewedCount >= Math.floor(rawCount * 0.6)
    && reviewedCount <= Math.ceil(rawCount * 1.5);
}

function joinReviewedChunks(sourceChunks, reviewedChunks) {
  return reviewedChunks.map((chunk, index) => {
    if (index >= sourceChunks.length - 1) return chunk.trim();
    const boundary = sourceChunks[index].match(/\s+$/u)?.[0] || "";
    return `${chunk.trim()}${boundary}`;
  }).join("").trim();
}

export async function reviewRecognizedText(rawText) {
  const url = process.env.AGENT_INTERNAL_LLM_URL?.trim();
  const token = process.env.AGENT_INTERNAL_LLM_TOKEN?.trim();
  const template = await reviewTemplate();
  const chunks = splitRevisionChunks(rawText);
  if (!url || !token || !template || chunks.length === 0) {
    return {
      text: rawText,
      status: "fallback_raw",
      reviewed_by_model: false,
      error_code: "revision_gateway_or_text_unavailable",
      chunk_count: chunks.length,
    };
  }
  const reviewedChunks = [];
  let provider = null;
  let model = null;
  for (const [index, chunk] of chunks.entries()) {
    const prompt = template
      .replaceAll("__CHUNK_INDEX__", String(index + 1))
      .replaceAll("__CHUNK_COUNT__", String(chunks.length))
      .replace("__RAW_RECOGNIZED_TEXT__", chunk);
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
        return {
          text: rawText,
          status: "fallback_raw",
          reviewed_by_model: false,
          error_code: "model_review_empty",
          chunk_count: chunks.length,
        };
      }
      reviewedChunks.push(data.text.trim());
      provider ||= data.provider || null;
      model ||= data.model || null;
    } catch {
      return {
        text: rawText,
        status: "fallback_raw",
        reviewed_by_model: false,
        error_code: "model_review_failed",
        chunk_count: chunks.length,
      };
    } finally {
      clearTimeout(timer);
    }
  }
  const reviewedText = joinReviewedChunks(chunks, reviewedChunks);
  if (!revisionPreservesSource(rawText, reviewedText)) {
    return {
      text: rawText,
      status: "fallback_raw",
      reviewed_by_model: false,
      error_code: "revision_integrity_failed",
      chunk_count: chunks.length,
    };
  }
  return {
    text: reviewedText,
    status: "reviewed",
    reviewed_by_model: true,
    provider,
    model,
    chunk_count: chunks.length,
    raw_character_count: Array.from(rawText).length,
    reviewed_character_count: Array.from(reviewedText).length,
    source_language_policy: "preserve_source_language",
    layout_policy: "semantic_reflow",
  };
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
  const review = mode === "ocr_reviewed"
    ? await reviewRecognizedText(ocr.text)
    : { text: ocr.text, status: "skipped", reviewed_by_model: false };
  return {
    raw_text: ocr.text,
    text: review.text,
    source: "local_ocr",
    language_set: ocr.language_set,
    review: { ...review, text: undefined },
  };
}
