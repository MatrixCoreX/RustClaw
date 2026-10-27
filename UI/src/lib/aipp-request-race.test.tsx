import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AippPage, writeCachedAippCatalog } from "../components/AippPage";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { appStorageKey } from "./product-identity";
import type { AippCatalogItem } from "../types/api";

test("AiAPP polling cannot leave a slow foreground request permanently loading", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  };
  const app: AippCatalogItem = {
    skill_name: "fixture", package_version: "1.0.0", renderer: "task_activity_v1",
    data_contract: "skill_task_activity_v1", icon: "gallery_vertical_end", default_locale: "en",
    titles: { en: "Fixture" }, descriptions: { en: "Fixture" }, installed: true,
    entrypoint: null, bridge_capabilities: [], task_channel_scope: "all",
  };
  storage.setItem(appStorageKey("monitor.aipp.selectedSkill"), "fixture");
  writeCachedAippCatalog(storage, [app], Date.now(), "fixture");
  let poll: (() => void) | undefined;
  const pending: Array<(response: Response) => void> = [];
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    localStorage: storage, sessionStorage: storage, setTimeout, clearTimeout,
    setInterval: (callback: () => void) => { poll = callback; return 1; },
    clearInterval: () => undefined,
  } });
  Object.defineProperty(globalThis, "document", { configurable: true, value: {
    visibilityState: "visible", addEventListener: () => undefined, removeEventListener: () => undefined,
  } });
  const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
  let root: ReactTestRenderer | undefined;
  try {
    await act(async () => {
      root = create(<UiDialogProvider><AippPage requestScope="fixture" lang="en" t={(_zh, en) => en}
        apiFetch={(path) => path === "/v1/aipps"
          ? Promise.resolve(response({ apps: [app] }))
          : new Promise(resolve => pending.push(resolve))}
        onOpenAgent={() => undefined} onOpenSkillStore={() => undefined} />
      </UiDialogProvider>);
    });
    assert.equal(pending.length, 1);
    await act(async () => { poll?.(); });
    assert.equal(pending.length, 1, "polling must not supersede the pending foreground request");
    await act(async () => { pending[0](response({
      schema_version: 1, items: [], page_item_count: 0, total_item_count: 0,
      sort_order: "newest", next_cursor_sequence: null, updated_at_ms: null,
    })); });
    assert.equal(root!.root.findAll(node =>
      typeof node.type === "string" && String(node.props.className || "").includes("animate-spin"),
    ).length, 0, "all loading indicators must settle");
    await act(async () => { poll?.(); });
    assert.equal(pending.length, 2, "polling resumes after foreground completion");
    await act(async () => { pending[1](response({ items: [], page_item_count: 0, total_item_count: 0 })); });
  } finally {
    await act(async () => root?.unmount());
    if (previousWindow) Object.defineProperty(globalThis, "window", previousWindow);
    else Reflect.deleteProperty(globalThis, "window");
    if (previousDocument) Object.defineProperty(globalThis, "document", previousDocument);
    else Reflect.deleteProperty(globalThis, "document");
  }
});
