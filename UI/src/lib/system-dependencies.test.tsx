import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { SystemDependenciesPanel } from "../components/SystemDependenciesPanel";
import { UiDialogProvider } from "../components/UiDialogProvider";
import type { HostDependenciesSnapshot } from "../types/api";

const t = (zh: string, _en: string) => zh;

function snapshot(): HostDependenciesSnapshot {
  return {
    schema_version: 1,
    collected_at_ts: 1,
    platform: "linux",
    package_manager: "apt",
    summary: {
      total: 2,
      installed: 1,
      missing_required: 1,
      missing_optional: 0,
    },
    dependencies: [
      {
        id: "git",
        category: "runtime",
        required: true,
        installed: false,
        version: null,
        executable: null,
        package_manager: "apt",
        installable: true,
        used_by: ["workspace"],
        status_code: "missing_required",
      },
      {
        id: "ffmpeg",
        category: "skill",
        required: false,
        installed: true,
        version: "ffmpeg version 7.1",
        executable: "ffmpeg",
        package_manager: "apt",
        installable: true,
        used_by: ["audio_transcribe"],
        status_code: "installed",
      },
    ],
    operations: [],
  };
}

test("shows missing dependencies, versions, capability ownership, and install control", () => {
  const markup = renderToStaticMarkup(
    <UiDialogProvider>
      <SystemDependenciesPanel
        t={t}
        snapshot={snapshot()}
        loading={false}
        errorCode={null}
        isAdmin
        installingId={null}
        onRefresh={() => {}}
        onInstall={() => {}}
      />
    </UiDialogProvider>,
  );

  assert.match(markup, /系统依赖检查/);
  assert.match(markup, /系统必需缺失 1 项/);
  assert.match(markup, /Git 版本管理/);
  assert.match(markup, /ffmpeg version 7\.1/);
  assert.match(markup, /技能依赖/);
  assert.match(markup, /语音转写/);
  assert.match(markup, />安装</);
});

test("does not expose installation controls to non-admin users", () => {
  const markup = renderToStaticMarkup(
    <UiDialogProvider>
      <SystemDependenciesPanel
        t={t}
        snapshot={snapshot()}
        loading={false}
        errorCode={null}
        isAdmin={false}
        installingId={null}
        onRefresh={() => {}}
        onInstall={() => {}}
      />
    </UiDialogProvider>,
  );

  assert.doesNotMatch(markup, />安装</);
  assert.match(markup, /管理员可以/);
});
