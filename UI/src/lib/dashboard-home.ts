export type DashboardStepStatus = "done" | "attention" | "todo";

export type DashboardActionKind =
  | "offline"
  | "llm_setup"
  | "llm_restart"
  | "wechat_setup"
  | "chat_test";

export type DashboardActionPage = "dashboard" | "models" | "services" | "chat";

export interface SuggestedDashboardAction {
  kind: DashboardActionKind;
  page: DashboardActionPage;
}

export interface DashboardOverviewItem {
  key: "status" | "memory" | "uptime";
  label: string;
  value: string;
  tone: "good" | "neutral" | "warning";
}

export function countCompletedDashboardSteps(statuses: DashboardStepStatus[]): number {
  return statuses.filter((status) => status === "done").length;
}

export function getDashboardOverviewItems(input: {
  isOnline: boolean;
  memoryLabel: string;
  uptimeLabel: string;
}): DashboardOverviewItem[] {
  return [
    {
      key: "status",
      label: "服务状态",
      value: input.isOnline ? "可访问" : "离线",
      tone: input.isOnline ? "good" : "warning",
    },
    {
      key: "memory",
      label: "内存占用",
      value: input.memoryLabel,
      tone: "neutral",
    },
    {
      key: "uptime",
      label: "运行时长",
      value: input.uptimeLabel,
      tone: "neutral",
    },
  ];
}

export function getSuggestedDashboardAction(input: {
  isOnline: boolean;
  llmStepStatus: DashboardStepStatus;
  testMessageStepStatus: DashboardStepStatus;
  wechatStepStatus: DashboardStepStatus;
}): SuggestedDashboardAction {
  if (!input.isOnline) {
    return {
      kind: "offline",
      page: "dashboard",
    };
  }

  if (input.llmStepStatus === "todo") {
    return {
      kind: "llm_setup",
      page: "models",
    };
  }

  if (input.llmStepStatus === "attention") {
    return {
      kind: "llm_restart",
      page: "models",
    };
  }

  if (input.testMessageStepStatus !== "done") {
    return {
      kind: "chat_test",
      page: "chat",
    };
  }

  if (input.wechatStepStatus !== "done") {
    return {
      kind: "wechat_setup",
      page: "services",
    };
  }

  return {
    kind: "chat_test",
    page: "chat",
  };
}
