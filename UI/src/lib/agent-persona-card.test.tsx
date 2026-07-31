import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { AgentPersonaCard } from "../components/AgentPersonaCard";
import type { AgentConfigResponse } from "../types/api";

const t = (zh: string, _en: string) => zh;

function config(agents = 1): AgentConfigResponse {
  return {
    schema_version: 1,
    config_path: "configs/agents.toml",
    editable: true,
    applies_to: "new_tasks",
    notice_key: "agent.persona.scope_notice",
    agents: Array.from({ length: agents }, (_, index) => ({
      id: index === 0 ? "main" : `agent-${index}`,
      name: index === 0 ? "Main" : `Agent ${index}`,
      description: "",
      saved_profile: "inherit",
      effective_profile: "executor",
      custom_persona: "",
      allowed_skills: [],
      runtime_applied: true,
    })),
    preset_catalog: [
      {
        id: "inherit",
        name_key: "agent.persona.inherit.name",
        description_key: "agent.persona.inherit.description",
      },
      {
        id: "custom",
        name_key: "agent.persona.custom.name",
        description_key: "agent.persona.custom.description",
      },
    ],
    constraints: {
      custom_persona_max_chars: 37,
      allowed_control_characters: ["tab", "newline"],
    },
  };
}

const baseProps = {
  t,
  loading: false,
  saving: false,
  error: null,
  message: null,
  onRefresh: () => {},
  onSave: async () => true,
};

test("shows the fixed safety explanation and backend-provided custom limit", () => {
  const markup = renderToStaticMarkup(<AgentPersonaCard {...baseProps} config={config()} />);
  assert.match(markup, /性格只改变聊天语气，不改变它做什么、生成什么或交付什么/);
  assert.match(markup, /maxLength="37"/);
  assert.match(markup, /只影响之后新建的任务/);
  assert.doesNotMatch(markup, /<select[^>]*><option value="main"/);
});

test("shows a target selector only when more than one Agent exists", () => {
  const markup = renderToStaticMarkup(<AgentPersonaCard {...baseProps} config={config(2)} />);
  assert.match(markup, /<option value="main" selected="">Main<\/option>/);
  assert.match(markup, /<option value="agent-1">Agent 1<\/option>/);
});
