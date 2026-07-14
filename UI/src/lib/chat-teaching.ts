export interface MessageTeachingRunLink {
  id: string;
  userMessageId: string;
  assistantMessageId?: string | null;
}

export function teachingRunByMessageId<T extends MessageTeachingRunLink>(
  runs: readonly T[],
): Map<string, T> {
  const runsByMessage = new Map<string, T>();
  runs.forEach((run) => {
    runsByMessage.set(run.userMessageId, run);
    if (run.assistantMessageId) {
      runsByMessage.set(run.assistantMessageId, run);
    }
  });
  return runsByMessage;
}

export function teachingMessageInteractive(
  teachingMode: boolean,
  run: MessageTeachingRunLink | null | undefined,
): boolean {
  return teachingMode && Boolean(run);
}
