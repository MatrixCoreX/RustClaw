import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { renderToStaticMarkup } from "react-dom/server";

import { DependencyList, SkillStoreCatalog } from "../components/SkillStoreCatalog";
import type { SkillStoreItem } from "../types/api";

function storeItem(overrides: Partial<SkillStoreItem> = {}): SkillStoreItem {
  return {
    name: "media_download",
    catalog_section: "other",
    kind: "runner",
    source_kind: "bundled_optional",
    installed: false,
    enabled: false,
    host_dependencies: ["git", "ffmpeg", "tesseract_chi_sim"],
    runtime_assets: ["modelscope_sensevoice_small", "modelscope_fsmn_vad"],
    min_memory_mb: 2048,
    min_free_disk_mb: 4096,
    skill: { name: "media_download" },
    ...overrides,
  };
}

const renderCatalog = (item: SkillStoreItem) =>
  renderToStaticMarkup(
    <SkillStoreCatalog
      lang="zh"
      t={(zh) => zh}
      data={{ items: [item], uninstalled_skill_names: [item.name] }}
      loading={false}
      error={null}
      message={null}
      actionName={null}
      onRefresh={() => undefined}
      onCheckDependencies={async () => ({
        schema_version: 1,
        skill_name: item.name,
        checked_at_unix: 1,
        all_installed: false,
        dependencies: [],
      })}
      onInstall={() => undefined}
      onRemove={() => undefined}
      onCancel={() => undefined}
    />,
  );

test("shows manifest-declared dependencies in Skill Store install details", () => {
  const markup = renderCatalog(storeItem());

  assert.match(markup, />系统工具</);
  assert.match(markup, /Git 版本管理/);
  assert.match(markup, /FFmpeg 音视频处理/);
  assert.match(markup, /Tesseract 简体中文识别包/);
  assert.match(markup, />本地模型\/资源</);
  assert.match(markup, /SenseVoice Small 语音识别模型/);
  assert.match(markup, /FSMN 语音活动检测模型/);
  assert.match(markup, />最低可用内存</);
  assert.match(markup, /2048 MiB/);
  assert.match(markup, />最低可用存储</);
  assert.match(markup, /4096 MiB/);
});

test("makes the absence of extra dependencies explicit", () => {
  const markup = renderCatalog(storeItem({ host_dependencies: [], runtime_assets: [] }));

  assert.match(markup, /无需额外安装/);
  assert.match(markup, /无需额外下载/);
});

test("installed model resources are checked and repaired only after explicit user actions", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const item = storeItem({ installed: true, name: "fixture-local-recognition" });
  let checks = 0;
  let installs = 0;
  let renderer: ReactTestRenderer;
  await act(async () => { renderer = create(<SkillStoreCatalog lang="zh" t={zh => zh}
    data={{ items: [item], uninstalled_skill_names: [] }} loading={false} error={null}
    message={null} actionName={null} onRefresh={() => undefined}
    onCheckDependencies={async name => {
      checks += 1;
      return { schema_version: 1, skill_name: name, checked_at_unix: 1, all_installed: false,
        dependencies: [{ id: "modelscope_sensevoice_small", kind: "runtime_asset", applicable: true, installed: false, status_code: "missing" }] };
    }} onInstall={() => { installs += 1; }} onRemove={() => undefined} onCancel={() => undefined} />); });
  try {
    assert.equal(checks, 0, "opening the store never opens browsers or prepares models");
    assert.equal(installs, 0);
    assert.doesNotMatch(JSON.stringify(renderer!.toJSON()), /修复安装并准备资源/);
    const check = renderer!.root.findAllByType("button").find(node => node.children.includes("检查本地资源"));
    await act(async () => { check!.props.onClick(); });
    assert.equal(checks, 1);
    assert.equal(installs, 0, "a missing model is not download permission");
    const repair = renderer!.root.findAllByType("button").find(node => node.children.includes("修复安装并准备资源"));
    assert.ok(repair);
    await act(async () => { repair!.props.onClick(); });
    assert.equal(installs, 1, "reuse the existing host installation/consent/progress transaction");
  } finally { await act(async () => { renderer!.unmount(); }); }
});

test("unsupported target resources are not mislabeled as missing models", () => {
  const markup = renderToStaticMarkup(<DependencyList values={["fixture-model"]} kind="runtime_asset"
    labels={{}} emptyLabel="none" t={zh => zh} check={{ loading: false, error: null,
      data: { schema_version: 1, skill_name: "fixture", checked_at_unix: 1, all_installed: true,
        dependencies: [{ id: "fixture-model", kind: "runtime_asset", applicable: false, installed: false, status_code: "not_applicable" }] } }} />);
  assert.match(markup, /此平台无需安装/);
  assert.doesNotMatch(markup, />未安装</);
});

test("renders server-owned catalog pagination without materializing every card", () => {
  const item = storeItem();
  const markup = renderToStaticMarkup(
    <SkillStoreCatalog
      lang="zh"
      t={(zh) => zh}
      data={{
        items: [item],
        uninstalled_skill_names: [item.name],
        page: 2,
        page_size: 1,
        total: 3,
        has_more: true,
        query: "",
      }}
      loading={false}
      error={null}
      message={null}
      actionName={null}
      onRefresh={() => undefined}
      onBrowse={() => undefined}
      onCheckDependencies={async () => ({
        schema_version: 1,
        skill_name: item.name,
        checked_at_unix: 1,
        all_installed: true,
        dependencies: [],
      })}
      onInstall={() => undefined}
      onRemove={() => undefined}
      onCancel={() => undefined}
    />,
  );

  assert.match(markup, /共 3/);
  assert.match(markup, /2\/3/);
  assert.match(markup, />上一页</);
  assert.match(markup, />下一页</);
});

test("renders every dependency with its observed installation state", () => {
  const markup = renderToStaticMarkup(
    <DependencyList
      values={["git", "ffmpeg"]}
      kind="host"
      labels={{ git: ["Git 版本管理", "Git version control"], ffmpeg: ["FFmpeg", "FFmpeg"] }}
      emptyLabel="无需额外安装"
      check={{
        loading: false,
        error: null,
        data: {
          schema_version: 1,
          skill_name: "media_download",
          checked_at_unix: 1,
          all_installed: false,
          dependencies: [
            { id: "git", kind: "host", installed: true, status_code: "installed", version: "2.0" },
            { id: "ffmpeg", kind: "host", installed: false, status_code: "missing" },
          ],
        },
      }}
      t={(zh) => zh}
    />,
  );

  assert.match(markup, /Git 版本管理/);
  assert.match(markup, /已正确安装/);
  assert.match(markup, /FFmpeg/);
  assert.match(markup, /未安装/);
});
