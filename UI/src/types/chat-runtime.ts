import type { ChatMessage, TaskLlmDebugResponse, TaskQueryResponse } from './api';

export interface ChatThreadSummary {
  id: string;
  agentId: string;
  title: string;
  preview: string;
  updatedAt: number;
  messageCount: number;
  teachingMode: boolean;
  taskId: string | null;
  taskStatus: TaskQueryResponse["status"] | "running" | null;
  llmCallCount: number | null;
}

export interface ChatTeachingRunSummary {
  id: string;
  taskId: string | null;
  userMessageId: string;
  assistantMessageId: string | null;
  userText: string;
  assistantText: string | null;
  status: TaskQueryResponse["status"] | "running";
  startedAt: number;
  completedAt: number | null;
  callCount: number | null;
  hasTrace: boolean;
  traceError: string | null;
  selected: boolean;
}

export interface ChatTeachingRunRecord {
  id: string;
  taskId: string | null;
  userMessageId: string;
  assistantMessageId?: string | null;
  userText: string;
  assistantText?: string | null;
  status: TaskQueryResponse["status"] | "running";
  startedAt: number;
  completedAt?: number | null;
  taskResult?: TaskQueryResponse | null;
  llmDebug?: TaskLlmDebugResponse | null;
  llmDebugError?: string | null;
  callCount?: number | null;
}

export interface ChatThreadRecord {
  id: string;
  agentId: string;
  title: string;
  messages: ChatMessage[];
  input: string;
  createdAt: number;
  updatedAt: number;
  teachingMode: boolean;
  externalChatId: string;
  lastTaskId?: string | null;
  teachingTaskResult?: TaskQueryResponse | null;
  teachingLlmDebug?: TaskLlmDebugResponse | null;
  teachingLlmDebugError?: string | null;
  activeTeachingRunId?: string | null;
  teachingRuns?: ChatTeachingRunRecord[];
}

export interface ChatThreadState {
  activeThreadId: string;
  threads: ChatThreadRecord[];
}
