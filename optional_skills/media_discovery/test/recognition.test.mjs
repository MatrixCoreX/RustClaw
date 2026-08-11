import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  parseTesseractLanguages,
  reviewRecognizedText,
  revisionPreservesSource,
  splitRevisionChunks,
} from "../src/recognition.mjs";

const WORKSPACE_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

test("shared revision prompt requires semantic reflow instead of visual wrapping", async () => {
  const prompt = await fs.readFile(
    path.join(WORKSPACE_ROOT, "prompts/layers/overlays/image_text_revision_prompt.md"),
    "utf8",
  );

  assert.match(prompt, /Reflow text by semantic structure/u);
  assert.match(prompt, /Merge visual soft wraps/u);
});

test("Tesseract language inventory is parsed without a script preference", () => {
  assert.deepEqual(
    parseTesseractLanguages(
      "List of available languages in /tmp/tessdata (5):\neng\nchi_sim\nara\nosd\neng\n",
    ),
    ["ara", "chi_sim", "eng"],
  );
});

test("revision chunks preserve complete multilingual text", () => {
  const source = `${"مرحبا".repeat(1_300)}\n${"日本語".repeat(2_100)}`;
  const chunks = splitRevisionChunks(source, 6_000);

  assert.ok(chunks.length >= 2);
  assert.ok(chunks.every((chunk) => Array.from(chunk).length <= 6_000));
  assert.equal(chunks.join(""), source);
});

test("model review processes every long-text chunk without truncation", async (t) => {
  const source = `${"第一段内容 ".repeat(900)}\n${"second section ".repeat(700)}编号 20260811`;
  const originalFetch = globalThis.fetch;
  const originalEnvironment = {
    url: process.env.AGENT_INTERNAL_LLM_URL,
    token: process.env.AGENT_INTERNAL_LLM_TOKEN,
    workspace: process.env.WORKSPACE_ROOT,
  };
  let calls = 0;
  process.env.AGENT_INTERNAL_LLM_URL = "http://127.0.0.1/internal-llm";
  process.env.AGENT_INTERNAL_LLM_TOKEN = "test-token";
  process.env.WORKSPACE_ROOT = WORKSPACE_ROOT;
  globalThis.fetch = async (_url, options) => {
    calls += 1;
    const prompt = JSON.parse(options.body).prompt;
    const chunk = prompt.match(/BEGIN_UNTRUSTED_RECOGNIZED_TEXT\n([\s\S]*)\nEND_UNTRUSTED_RECOGNIZED_TEXT/u)?.[1];
    return {
      ok: true,
      async json() {
        return { ok: true, data: { text: chunk, provider: "test", model: "test-model" } };
      },
    };
  };
  t.after(() => {
    globalThis.fetch = originalFetch;
    for (const [key, value] of Object.entries({
      AGENT_INTERNAL_LLM_URL: originalEnvironment.url,
      AGENT_INTERNAL_LLM_TOKEN: originalEnvironment.token,
      WORKSPACE_ROOT: originalEnvironment.workspace,
    })) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  });

  const result = await reviewRecognizedText(source);

  assert.equal(result.status, "reviewed");
  assert.equal(result.text, source);
  assert.ok(calls >= 2);
  assert.equal(result.chunk_count, calls);
});

test("revision integrity rejects changed numbers and severe omissions", () => {
  assert.equal(revisionPreservesSource("订单 20260811 金额 128.50", "订单 20260812 金额 128.50"), false);
  assert.equal(revisionPreservesSource("A long source passage. ".repeat(40), "short"), false);
  assert.equal(revisionPreservesSource("今天天汽很好 20260811", "今天天气很好。20260811"), true);
});
