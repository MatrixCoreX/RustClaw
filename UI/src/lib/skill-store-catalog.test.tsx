import assert from "node:assert/strict";
import test from "node:test";
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
