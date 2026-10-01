import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");

test("navigation pages remain route-level lazy imports behind Suspense", () => {
  const pages = [
    "AiLearningPage",
    "AippPage",
    "AssetsPage",
    "BancorPage",
    "ChatPage",
    "CommunicationSetupPage",
    "DashboardPage",
    "LogsPage",
    "MemoryPage",
    "ModelConfigPage",
    "NniPage",
    "SkillStorePage",
    "SkillsPage",
    "TasksPage",
  ];

  for (const page of pages) {
    assert.match(source, new RegExp(`const\\s+${page}\\s*=\\s*lazy\\(`));
    assert.doesNotMatch(source, new RegExp(`import\\s+\\{[^}]*${page}[^}]*\\}\\s+from`));
  }
  assert.match(source, /<Suspense\b/);
});
