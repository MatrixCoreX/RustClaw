import assert from "node:assert/strict";
import test from "node:test";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { renderToStaticMarkup } from "react-dom/server";
import { ExpandableMemoryText } from "../components/ExpandableMemoryText";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const t = (_zh: string, en: string) => en;
async function mount(text: string, dimensions = { scrollHeight: 300, clientHeight: 72 }) {
  let renderer!: ReactTestRenderer;
  const tree = (content: string) => <ExpandableMemoryText text={content} t={t} />;
  await act(async () => { renderer = create(tree(text), {
    createNodeMock: element => element.type === "p" ? dimensions : null,
  }); });
  return { renderer, paragraph: () => renderer.root.findByType("p"),
    buttons: () => renderer.root.findAllByType("button"),
    update: async (content: string) => { await act(async () => renderer.update(tree(content))); },
    close: async () => { await act(async () => renderer.unmount()); } };
}

test("memory display disclosure never slices long Unicode, emoji, newlines or the retained tail", async () => {
  const text = `${"离线校验🙂\n".repeat(6000)} retained-tail`;
  const view = await mount(text);
  try {
    assert.equal(view.paragraph().children.join(""), text);
    assert.match(view.paragraph().props.className, /line-clamp-3/);
    assert.equal(view.buttons()[0].props["aria-expanded"], false);
    assert.equal(view.buttons()[0].props["aria-controls"], view.paragraph().props.id);
    for (const expanded of [true, false, true]) {
      await act(async () => view.buttons()[0].props.onClick());
      assert.equal(view.buttons()[0].props["aria-expanded"], expanded);
      assert.equal(view.paragraph().props.className.includes("line-clamp-3"), !expanded);
      assert.equal(view.paragraph().children.join(""), text);
    }
  } finally { await view.close(); }
});

test("short rendered text does not get an unnecessary disclosure control", async () => {
  const view = await mount("short complete text", { scrollHeight: 24, clientHeight: 24 });
  try {
    assert.equal(view.buttons().length, 0);
    assert.equal(view.paragraph().children.join(""), "short complete text");
  } finally { await view.close(); }
});

test("replaced memory content starts collapsed rather than inheriting a previous item expansion", async () => {
  const view = await mount("first complete content");
  try {
    await act(async () => view.buttons()[0].props.onClick());
    assert.equal(view.buttons()[0].props["aria-expanded"], true);
    await view.update("different complete content");
    assert.equal(view.buttons()[0].props["aria-expanded"], false);
    assert.equal(view.paragraph().children.join(""), "different complete content");
  } finally { await view.close(); }
});

test("memory markup stays text-only, including when disclosure is not yet measured", () => {
  const markup = renderToStaticMarkup(<ExpandableMemoryText text={'<img src=x onerror="alert(1)">'} t={t} />);
  assert.match(markup, /&lt;img/);
  assert.doesNotMatch(markup, /<img|<script/);
});
