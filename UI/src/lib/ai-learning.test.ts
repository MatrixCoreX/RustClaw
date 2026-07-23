import assert from "node:assert/strict";
import test from "node:test";

import { parseReadmeLearningPages } from "./ai-learning";

test("splits README by level-two headings and preserves the preamble", () => {
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

  assert.equal(pages.length, 2);
  assert.equal(pages[0].title, "Overview");
  assert.match(pages[0].markdown, /^# Product/);
  assert.equal(pages[0].diagramCount, 1);
  assert.equal(pages[0].subsectionCount, 1);
  assert.equal(pages[1].id, "setup");
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
