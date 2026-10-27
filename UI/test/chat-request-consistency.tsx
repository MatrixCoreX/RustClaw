import { useState } from "react";
import { createRoot } from "react-dom/client";
import { FileText, UserRound } from "lucide-react";
import { UiDialogProvider } from "../src/components/UiDialogProvider";
import { useChatRuntime } from "../src/hooks/useChatRuntime";
import { DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS } from "../src/lib/chat-attachments";
import "../src/index.css";

const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
async function apiFetch(path: string) {
  if (path.startsWith("/v1/tasks/conversation-history")) return response({
    schema_version: 1, status: "ok", turns: [], truncated: false,
    content_sha256: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
  });
  if (path === "/v1/ui/attachment-constraints") return response(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
  if (path.includes("/events?")) return new Response(new ReadableStream(), { headers: { "content-type": "text/event-stream" } });
  if (path.startsWith("/v1/conversation-inputs?")) return response({ schema_version: 1, items: [], next_after_input_seq: null });
  await new Promise(resolve => setTimeout(resolve, path.includes("slow") ? 550 : 20));
  return response({ calls: [], call_count: path.includes("slow") ? 99 : 2 });
}

function Fixture() {
  const [owner, setOwner] = useState("one");
  const runtime = useChatRuntime({ apiFetch, t: (_zh, en) => en, lang: "en",
    interactionAdapter: "generic", interactionChannel: "ui", activeUserKey: owner,
    activeIdentityIds: {}, conversationHistoryScope: `fixture:${owner}`,
    interactionExternalUserId: "", interactionExternalChatId: "",
    availableAgents: [{ id: "main", name: "Main" }],
    fetchTaskById: async id => ({ task_id: id, status: "succeeded", result_json: null, error_text: null }),
    onTaskSubmitted: () => undefined, onTaskResult: () => undefined,
  });
  return <main className="mx-auto max-w-3xl space-y-5 p-4">
    <h1 className="text-lg">Conversation teaching history</h1>
    <div className="flex flex-wrap gap-2">
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => void runtime.queryChatTeachingLlmDebug("slow")}><FileText size={16} />Slow trace</button>
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => void runtime.queryChatTeachingLlmDebug("fast")}><FileText size={16} />Fast trace</button>
      <button className="theme-btn-secondary inline-flex items-center gap-2 rounded-md border px-3 py-2" onClick={() => setOwner(owner === "one" ? "two" : "one")}><UserRound size={16} />Switch owner</button>
    </div>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>Owner</dt><dd data-testid="owner">{owner}</dd>
      <dt>Call count</dt><dd data-testid="trace">{runtime.chatTeachingLlmDebug?.call_count ?? "none"}</dd>
      <dt>Loading</dt><dd data-testid="loading">{String(runtime.chatTeachingLlmDebugLoading)}</dd>
    </dl>
    <output data-testid="error">{runtime.chatTeachingLlmDebugError ?? runtime.chatError}</output>
  </main>;
}
createRoot(document.getElementById("root")!).render(<UiDialogProvider><Fixture /></UiDialogProvider>);
