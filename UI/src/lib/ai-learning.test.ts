import assert from "node:assert/strict";
import test from "node:test";

import { classifyLearningLink, parseReadmeLearningPages } from "./ai-learning";

test("groups level-three sections under chapters and omits repository preamble", () => {
  const pages = parseReadmeLearningPages(`# Product

Intro.

## Overview

Text.

### Details

\`\`\`mermaid
flowchart LR
  A --> B
\`\`\`

## Setup

Steps.
`);

  assert.equal(pages.length, 3);
  assert.equal(pages[0].title, "Overview");
  assert.equal(pages[0].kind, "chapter");
  assert.match(pages[0].markdown, /^## Overview/);
  assert.doesNotMatch(pages[0].markdown, /# Product/);
  assert.equal(pages[1].title, "Details");
  assert.equal(pages[1].chapterTitle, "Overview");
  assert.equal(pages[1].kind, "section");
  assert.equal(pages[1].diagramCount, 1);
  assert.match(pages[1].markdown, /^## Overview\n\n### Details/);
  assert.equal(pages[2].id, "setup");
});

test("does not split headings inside fenced code", () => {
  const pages = parseReadmeLearningPages(`## One

\`\`\`text
## Not a page
\`\`\`

## Two
`);

  assert.deepEqual(pages.map((page) => page.title), ["One", "Two"]);
});

test("does not create an empty chapter overview before its first section", () => {
  const pages = parseReadmeLearningPages(`## Runtime

### Execute

Run it.
`);

  assert.equal(pages.length, 1);
  assert.equal(pages[0].title, "Execute");
  assert.equal(pages[0].chapterTitle, "Runtime");
});

test("classifies only web URLs and page anchors as interactive links", () => {
  assert.equal(classifyLearningLink("https://example.com/docs"), "external");
  assert.equal(classifyLearningLink("http://example.com"), "external");
  assert.equal(classifyLearningLink("#runtime"), "internal");
  assert.equal(classifyLearningLink("docs/runtime.md"), "reference");
  assert.equal(classifyLearningLink("../README.md"), "reference");
  assert.equal(classifyLearningLink("javascript:alert(1)"), "reference");
});
