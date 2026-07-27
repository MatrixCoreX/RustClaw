import assert from "node:assert/strict";
import test from "node:test";

import { shouldCollapseNavigationForTarget } from "../components/ConsoleLayout";

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
