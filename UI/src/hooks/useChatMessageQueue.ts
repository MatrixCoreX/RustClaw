import { useEffect, useMemo, useSyncExternalStore } from "react";
import { ChatMessageQueue } from "../lib/chat-message-queue";

export function useChatMessageQueue(scope: string, blockedThreadIds: string[]) {
  const queue = useMemo(() => new ChatMessageQueue(), [scope]);
  const snapshot = useSyncExternalStore(queue.subscribe, queue.getSnapshot, queue.getSnapshot);
  const blockedKey = JSON.stringify(blockedThreadIds);
  useEffect(() => { queue.setBlockedThreads(JSON.parse(blockedKey)); }, [queue, blockedKey]);
  useEffect(() => { queue.activate(); return () => queue.dispose(); }, [queue]);
  useEffect(() => {
    if (!snapshot.messages.length || typeof window === "undefined") return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [snapshot.messages.length]);
  return { queue, ...snapshot };
}
