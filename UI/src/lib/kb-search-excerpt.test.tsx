import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { KbSearchExcerpt } from "../components/KbSearchExcerpt";

const t = (_zh: string, en: string) => en;

test("short and boundary-length KB excerpts remain complete without an expand button", () => {
  for (const length of [0, 7_999, 8_000]) {
    const text = "文".repeat(length);
    const markup = renderToStaticMarkup(<KbSearchExcerpt text={text} t={t} />);
    assert.ok(markup.includes(text));
    assert.doesNotMatch(markup, /<button/);
  }
});

test("KB long excerpts explicitly disclose a preview and expand or collapse without losing the tail", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const text = `${"文".repeat(8_000)} retained-tail`;
  let renderer: ReactTestRenderer | undefined;
  try {
    await act(async () => { renderer = create(<KbSearchExcerpt text={text} t={t} />); });
    const paragraph = () => renderer!.root.findByType("p").children.join("");
    const button = () => renderer!.root.findByType("button");
    assert.equal(paragraph(), text.slice(0, 8_000));
    assert.equal(button().props["aria-expanded"], false);
    assert.ok(button().children.join("").includes("Preview shown"));
    await act(async () => button().props.onClick());
    assert.equal(paragraph(), text);
    assert.equal(button().props["aria-expanded"], true);
    await act(async () => button().props.onClick());
    assert.equal(paragraph(), text.slice(0, 8_000));
    await act(async () => button().props.onClick());
    assert.equal(paragraph(), text);
  } finally {
    await act(async () => renderer?.unmount());
  }
});

test("KB preview does not split an emoji at its display boundary", () => {
  const text = `${"文".repeat(7_999)}🙂 tail`;
  const markup = renderToStaticMarkup(<KbSearchExcerpt text={text} t={t} />);
  assert.ok(markup.includes("文".repeat(7_999)));
  assert.doesNotMatch(markup, /[\ud800-\udfff]/u);
  assert.match(markup, /Show full excerpt/);
});
