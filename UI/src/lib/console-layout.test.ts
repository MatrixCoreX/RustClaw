import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { shouldCollapseNavigationForTarget } from "../components/ConsoleLayout";

test("places AiAPP between Agent and NNI in the shared navigation", () => {
  const source = readFileSync(new URL("../hooks/useConsoleProjections.tsx", import.meta.url), "utf8");
  const navigation = source.match(/const navItems = useMemo\(([\s\S]*?)const onboardingSteps/);
  assert.ok(navigation);
  const ids = [...navigation[1].matchAll(/id: "([^"]+)" as const/g)].map((match) => match[1]);
  assert.deepEqual(ids, [
    "dashboard", "chat", "aipps", "nni", "bancor", "assets", "channels",
    "skill_store", "memory", "logs", "tasks", "ai_learning",
  ]);
});

function contentTarget(keepNavigationOpen = false): EventTarget {
  const candidate = {
    closest: (selector: string) => keepNavigationOpen
      && selector.includes("data-keep-navigation-open")
      ? candidate
      : null,
  };
  return {
    closest: candidate.closest,
  } as unknown as EventTarget;
}

test("collapses navigation for any click in the main content area", () => {
  assert.equal(shouldCollapseNavigationForTarget(contentTarget()), true);
});

test("keeps navigation only for explicitly exempt content", () => {
  assert.equal(shouldCollapseNavigationForTarget(contentTarget(true)), false);
  assert.equal(shouldCollapseNavigationForTarget(new EventTarget()), false);
});
