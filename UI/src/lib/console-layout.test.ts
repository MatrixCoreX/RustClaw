import assert from "node:assert/strict";
import test from "node:test";

import { shouldCollapseNavigationForTarget } from "../components/ConsoleLayout";

function actionTarget(options: {
  disabled?: boolean;
  ariaDisabled?: boolean;
  keepNavigationOpen?: boolean;
} = {}): EventTarget {
  const interactive = {
    closest: (selector: string) => options.keepNavigationOpen
      && selector.includes("data-keep-navigation-open")
      ? interactive
      : null,
    matches: (selector: string) => selector === ":disabled" && Boolean(options.disabled),
    getAttribute: (name: string) => name === "aria-disabled" && options.ariaDisabled
      ? "true"
      : null,
  };
  return {
    closest: () => interactive,
  } as unknown as EventTarget;
}

test("collapses navigation for enabled content actions", () => {
  assert.equal(shouldCollapseNavigationForTarget(actionTarget()), true);
});

test("keeps navigation for disabled or explicitly exempt actions", () => {
  assert.equal(shouldCollapseNavigationForTarget(actionTarget({ disabled: true })), false);
  assert.equal(shouldCollapseNavigationForTarget(actionTarget({ ariaDisabled: true })), false);
  assert.equal(
    shouldCollapseNavigationForTarget(actionTarget({ keepNavigationOpen: true })),
    false,
  );
  assert.equal(shouldCollapseNavigationForTarget(new EventTarget()), false);
});
