import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";

import {
  classifyLearningLink,
  LEARNING_STAGE_ORDER,
  learningHeadingId,
  orderLearningPagesByStage,
  parseReadmeLearningPages,
  parseStandaloneLearningDocument,
  parseStandaloneLearningPages,
  searchLearningPages,
} from "./ai-learning";

function repositoryRoot(): string {
  const candidates = [process.cwd(), resolve(process.cwd(), "..")];
  const root = candidates.find((candidate) =>
    existsSync(resolve(candidate, "README.md"))
      && existsSync(resolve(candidate, "docs/architecture"))
  );
  assert.ok(root, "repository root with bundled learning sources must exist");
  return root;
}

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

test("omits repository-only navigation blocks from the learning sequence", () => {
  const pages = parseReadmeLearningPages(`## Runtime

Details.

<!-- ai-learning-exclude:start -->
## Architecture Index

Repository links.
<!-- ai-learning-exclude:end -->

## Setup

Steps.
`);

  assert.deepEqual(pages.map((page) => page.title), ["Runtime", "Setup"]);
});

test("classifies only web URLs and page anchors as interactive links", () => {
  assert.equal(classifyLearningLink("https://example.com/docs"), "external");
  assert.equal(classifyLearningLink("http://example.com"), "external");
  assert.equal(classifyLearningLink("#runtime"), "internal");
  assert.equal(classifyLearningLink("docs/runtime.md"), "reference");
  assert.equal(classifyLearningLink("../README.md"), "reference");
  assert.equal(classifyLearningLink("javascript:alert(1)"), "reference");
});

test("keeps one architecture document as one learning page", () => {
  const page = parseStandaloneLearningDocument({
    id: "architecture-agent-loop",
    chapterId: "architecture-guide",
    chapterTitle: "Architecture Guide",
    markdown: `# Agent Loop

<!-- ai-learning-navigation:start -->
Navigation.
<!-- ai-learning-navigation:end -->

Introduction.

## Runtime

\`\`\`mermaid
flowchart LR
  A --> B
\`\`\`

## Planning

Details.
`,
  });

  assert.equal(page.id, "architecture-agent-loop");
  assert.equal(page.title, "Agent Loop");
  assert.equal(page.chapterTitle, "Architecture Guide");
  assert.equal(page.kind, "section");
  assert.equal(page.diagramCount, 1);
  assert.doesNotMatch(page.markdown, /Navigation/);
  assert.match(page.markdown, /Introduction/);
  assert.match(page.markdown, /## Planning/);
});

test("splits architecture guides at functional level-two boundaries", () => {
  const pages = parseStandaloneLearningPages({
    id: "architecture-agent-loop",
    chapterId: "architecture-guide",
    chapterTitle: "Architecture Guide",
    markdown: `# Agent Loop

Introduction.

## Runtime

Runtime details.

## Planning

Planning details.
`,
  });

  assert.deepEqual(pages.map((page) => page.title), ["Runtime", "Planning"]);
  assert.match(pages[0].markdown, /Introduction/);
  assert.doesNotMatch(pages[1].markdown, /Runtime details/);
  assert.equal(pages[0].chapterId, pages[1].chapterId);
});

test("inherits stable learning stages without exposing metadata in content", () => {
  const pages = parseReadmeLearningPages(`<!-- ai-learning-stage: foundations -->
## Overview

Intro.

<!-- ai-learning-stage: runtime -->
## Runtime

### Planning

Details.

### Execution

More details.
`);

  assert.deepEqual(
    pages.map((page) => [page.title, page.stageId]),
    [
      ["Overview", "foundations"],
      ["Planning", "runtime"],
      ["Execution", "runtime"],
    ],
  );
  pages.forEach((page) => {
    assert.doesNotMatch(page.markdown, /ai-learning-stage/);
  });
});

test("orders stages without changing page order within each stage", () => {
  const pages = parseReadmeLearningPages(`<!-- ai-learning-stage: development -->
## Tests

Details.

<!-- ai-learning-stage: foundations -->
## Overview

Intro.

<!-- ai-learning-stage: development -->
## Release

Steps.
`);

  const ordered = orderLearningPagesByStage(pages, ["foundations", "development"]);

  assert.deepEqual(
    ordered.map((page) => page.title),
    ["Overview", "Tests", "Release"],
  );
});

test("bundled learning sources use supported stage markers", () => {
  const root = repositoryRoot();
  const architectureRoot = resolve(root, "docs/architecture");
  const sources = [
    resolve(root, "README.md"),
    resolve(root, "README.zh-CN.md"),
    ...readdirSync(architectureRoot)
      .filter((file) => /^[0-9]{2}-.*\.md$/.test(file))
      .map((file) => resolve(architectureRoot, file)),
  ];
  const supported = new Set<string>(LEARNING_STAGE_ORDER);
  const unknown = sources.flatMap((source) => {
    const markdown = readFileSync(source, "utf8");
    return [...markdown.matchAll(/<!--\s*ai-learning-stage:\s*([a-z0-9_-]+)\s*-->/gi)]
      .map((match) => match[1].toLowerCase())
      .filter((stageId) => !supported.has(stageId))
      .map((stageId) => `${source}:${stageId}`);
  });

  assert.deepEqual(unknown, []);
});

test("task artifact guide is visible in operator and developer learning routes", () => {
  const root = repositoryRoot();
  for (const file of [
    "11-task-artifact-delivery.md",
    "11-task-artifact-delivery.zh-CN.md",
  ]) {
    const page = parseStandaloneLearningDocument({
      id: file,
      chapterId: "architecture-guide",
      chapterTitle: "Architecture Guide",
      markdown: readFileSync(resolve(root, "docs/architecture", file), "utf8"),
    });
    assert.equal(page.stageId, "capabilities-artifacts");
    assert.deepEqual(page.audiences, ["operator", "developer"]);
  }
});

test("applies explicit audience metadata and stage defaults", () => {
  const pages = parseReadmeLearningPages(`<!-- ai-learning-stage: foundations -->
## Start

Basics.

<!-- ai-learning-stage: agent-runtime -->
<!-- ai-learning-audience: beginner,developer -->
## Runtime

Details.

<!-- ai-learning-stage: development-release -->
## Release

Checks.
`);

  assert.deepEqual(pages[0].audiences, ["beginner", "operator", "developer"]);
  assert.deepEqual(pages[1].audiences, ["beginner", "developer"]);
  assert.deepEqual(pages[2].audiences, ["developer"]);
  pages.forEach((page) => assert.doesNotMatch(page.markdown, /ai-learning-audience/));
});

test("standalone documents own their stage and audience metadata", () => {
  const page = parseStandaloneLearningDocument({
    id: "architecture",
    chapterId: "guide",
    chapterTitle: "Guide",
    stageId: "wrong-fallback",
    markdown: `# Runtime

<!-- ai-learning-stage: agent-runtime -->
<!-- ai-learning-audience: operator,developer -->

## Loop

Details.
`,
  });

  assert.equal(page.stageId, "agent-runtime");
  assert.deepEqual(page.audiences, ["operator", "developer"]);
  assert.doesNotMatch(page.markdown, /ai-learning-(stage|audience)/);
});

test("extracts page headings and reading estimates outside code fences", () => {
  const [page] = parseReadmeLearningPages([
    "## Guide",
    "",
    "Introduction text.",
    "",
    "#### First step",
    "",
    "Read this.",
    "",
    "```md",
    "## Hidden heading",
    "```",
  ].join("\n"));

  assert.equal(page.estimatedMinutes, 1);
  assert.deepEqual(page.headings.map((heading) => heading.title), ["Guide", "First step"]);
  assert.equal(page.headings[1].id, learningHeadingId("First step"));
});

test("searches titles and content with all query tokens", () => {
  const pages = parseReadmeLearningPages(`## Runtime planning

Capability resolver details.

## Storage

Private database ownership.
`);

  assert.deepEqual(
    searchLearningPages(pages, "runtime resolver").map((page) => page.title),
    ["Runtime planning"],
  );
  assert.deepEqual(searchLearningPages(pages, "missing term"), []);
  assert.equal(searchLearningPages(pages, "").length, 2);
});
