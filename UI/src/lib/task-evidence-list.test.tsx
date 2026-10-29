import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { renderToStaticMarkup } from "react-dom/server";
import { TaskEvidenceList } from "../components/TaskEvidenceList";

const t = (_zh: string, en: string) => en;
const rows = Array.from({ length: 25 }, (_, i) => ({
  sourceKind: "knowledge", sourceRef: `kb:fixture/${i}:` + "x".repeat(200),
  score: 0.5, reason: `reason-${i}`,
}));

test("evidence preview is bounded but exact references and totals remain visible", () => {
  const html = renderToStaticMarkup(<TaskEvidenceList rows={rows} t={t} />);
  assert.match(html, /Included evidence/);
  assert.match(html, /1–12 \/ 25/);
  assert.equal((html.match(/<details/g) ?? []).length, 12);
  assert.ok(html.includes(rows[11].sourceRef));
  assert.ok(!html.includes(rows[12].sourceRef));
  assert.ok(!html.includes("href="), "data references must not become navigable untrusted URLs");
});

test("evidence pagination handles final page, shrink and task switch", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let root: ReactTestRenderer | undefined;
  const button = (name: string) => root!.root.findByProps({ "aria-label": name });
  try {
    await act(async () => { root = create(<TaskEvidenceList key="task-one" rows={rows} t={t} />); });
    assert.equal(button("Previous evidence page").props.disabled, true);
    await act(async () => { button("Next evidence page").props.onClick(); });
    assert.equal(root!.root.findAllByType("details").length, 12);
    assert.ok(JSON.stringify(root!.toJSON()).includes(rows[12].sourceRef));
    await act(async () => { button("Next evidence page").props.onClick(); });
    assert.equal(root!.root.findAllByType("details").length, 1);
    assert.equal(button("Next evidence page").props.disabled, true);
    await act(async () => { root!.update(<TaskEvidenceList key="task-one" rows={rows.slice(0, 1)} t={t} />); });
    assert.equal(root!.root.findAllByType("details").length, 1);
    assert.ok(JSON.stringify(root!.toJSON()).includes(rows[0].sourceRef));
    await act(async () => { root!.update(<TaskEvidenceList key="task-two" rows={rows} t={t} />); });
    assert.equal(button("Previous evidence page").props.disabled, true);
    await act(async () => { root!.update(<TaskEvidenceList key="task-two" rows={[]} t={t} />); });
    assert.equal(root!.root.findAllByType("button").length, 0);
    assert.ok(JSON.stringify(root!.toJSON()).includes("No memory or knowledge-base evidence"));
  } finally {
    await act(async () => root?.unmount());
  }
});
