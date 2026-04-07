export type FeishuBindStatus = "pending" | "detected" | "bound" | "failed" | "expired";

export interface FeishuBindSessionResponse {
  session_id: number;
  channel: string;
  bind_token: string;
  status: string;
  external_user_id?: string | null;
  external_chat_id?: string | null;
  error_text?: string | null;
  created_at: string;
  updated_at: string;
  expires_at: string;
  entry_url?: string | null;
}

export interface FeishuStepStatusInput {
  bindReady: boolean;
  serviceHealthy: boolean;
  session: FeishuBindSessionResponse | null;
}

export interface FeishuBindStatusCopy {
  tone: "pending" | "attention" | "success" | "error";
  zhLabel: string;
  enLabel: string;
  zhDescription: string;
  enDescription: string;
}

export interface FeishuSetupGuidanceInput {
  bindReady: boolean;
  hasUnsavedConfigChanges: boolean;
  serviceHealthy: boolean;
  hasActiveSession: boolean;
  bound: boolean;
}

export interface FeishuSetupGuidance {
  status:
    | "scan_to_setup"
    | "ready_to_start"
    | "ready_to_bind"
    | "binding"
    | "bound";
  zhSummary: string;
  enSummary: string;
  zhHint: string;
  enHint: string;
  canStartService: boolean;
  canStartBind: boolean;
}

type ApiFetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export function getFeishuBindStatusCopy(status: string): FeishuBindStatusCopy {
  switch (status) {
    case "detected":
      return {
        tone: "attention",
        zhLabel: "已识别账号",
        enLabel: "Recognized",
        zhDescription: "已识别飞书账号，正在完成绑定。",
        enDescription: "Your Feishu account has been recognized and binding is being completed.",
      };
    case "bound":
      return {
        tone: "success",
        zhLabel: "绑定成功",
        enLabel: "Bound",
        zhDescription: "现在可以直接在飞书里发消息了。",
        enDescription: "You can now chat in Feishu directly.",
      };
    case "failed":
      return {
        tone: "error",
        zhLabel: "绑定失败",
        enLabel: "Failed",
        zhDescription: "这次接入没有完成，请重新生成二维码。",
        enDescription: "Setup did not finish. Please regenerate the QR code.",
      };
    case "expired":
      return {
        tone: "error",
        zhLabel: "已超时",
        enLabel: "Expired",
        zhDescription: "二维码已超时，请重新生成。",
        enDescription: "The QR code expired. Please generate a new one.",
      };
    case "pending":
    default:
      return {
        tone: "pending",
        zhLabel: "等待扫码",
        enLabel: "Waiting for scan",
        zhDescription: "先扫码打开机器人，再把下方绑定码发给它完成绑定。",
        enDescription: "Scan to open the bot, then send the bind code below to finish binding.",
      };
  }
}

export function isFeishuBindTerminalStatus(status: string): boolean {
  return status === "bound" || status === "failed" || status === "expired";
}

export function getFeishuStepStatus(input: FeishuStepStatusInput): "done" | "attention" | "todo" {
  if (input.session?.status === "bound") return "done";
  if (input.serviceHealthy) return "attention";
  if (input.session && !isFeishuBindTerminalStatus(input.session.status)) return "attention";
  if (input.bindReady) return "attention";
  return "todo";
}

export function getFeishuSetupGuidance(input: FeishuSetupGuidanceInput): FeishuSetupGuidance {
  if (input.bound) {
    return {
      status: "bound",
      zhSummary: "飞书已经接入完成。",
      enSummary: "Feishu setup is complete.",
      zhHint: "后续直接在飞书里发消息就可以。",
      enHint: "You can now chat in Feishu directly.",
      canStartService: false,
      canStartBind: false,
    };
  }
  if (input.hasActiveSession) {
    return {
      status: "binding",
      zhSummary: "二维码已就绪。",
      enSummary: "The QR code is ready.",
      zhHint: "扫码打开机器人后，把页面里的绑定码发送给机器人。",
      enHint: "After scanning, send the bind code shown on this page to the bot.",
      canStartService: false,
      canStartBind: true,
    };
  }
  if (!input.bindReady) {
    return {
      status: "scan_to_setup",
      zhSummary: "点击开始后会生成二维码。",
      enSummary: "A QR code will appear after you start.",
      zhHint: "先扫码完成接入，接着按页面提示把绑定码发给机器人。",
      enHint: "Scan to finish setup first, then send the bind code shown on the page.",
      canStartService: false,
      canStartBind: true,
    };
  }
  if (!input.serviceHealthy) {
    return {
      status: "ready_to_start",
      zhSummary: "飞书服务还没启动。",
      enSummary: "The Feishu service is not running yet.",
      zhHint: "先启动服务，再开始扫码。",
      enHint: "Start the service first, then scan.",
      canStartService: true,
      canStartBind: false,
    };
  }
  return {
    status: "ready_to_bind",
    zhSummary: "飞书已经就绪。",
    enSummary: "Feishu is ready.",
    zhHint: "开始后扫码打开机器人，再发送绑定码。",
    enHint: "Start, scan to open the bot, then send the bind code.",
    canStartService: true,
    canStartBind: true,
  };
}

export async function startFeishuBindSession(apiFetch: ApiFetchLike): Promise<FeishuBindSessionResponse> {
  const res = await apiFetch("/v1/admin/channel-binds/feishu/start", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({}),
  });
  const body = (await res.json()) as {
    ok: boolean;
    data?: FeishuBindSessionResponse;
    error?: string | null;
  };
  if (!res.ok || !body.ok || !body.data) {
    throw new Error(body.error || `飞书绑定启动失败 (${res.status})`);
  }
  return body.data;
}

export async function fetchFeishuBindSession(
  apiFetch: ApiFetchLike,
  sessionId: number,
): Promise<FeishuBindSessionResponse> {
  const res = await apiFetch(`/v1/admin/channel-binds/feishu/${sessionId}`);
  const body = (await res.json()) as {
    ok: boolean;
    data?: FeishuBindSessionResponse;
    error?: string | null;
  };
  if (!res.ok || !body.ok || !body.data) {
    throw new Error(body.error || `飞书绑定状态获取失败 (${res.status})`);
  }
  return body.data;
}
