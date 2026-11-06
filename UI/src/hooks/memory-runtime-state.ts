import type {
  MemoryFactItem, MemoryOverviewResponse, MemoryPageResult, MemoryPreferenceItem,
  MemoryRecentItem, MemorySettingsResult, MemoryVectorStatus, RemoteMemoryDisclosure,
} from "../types/api";

export type MemoryOwner = () => boolean;
export type MemoryClearScope = "recent" | "all";
export type MemorySettingScope = "principal" | "conversation";
export interface MemoryListFilters {
  search: string;
  scope: string;
  origin: string;
  kind: string;
  status: string;
  freshness: string;
}

export interface MemoryView {
  owner: MemoryOwner;
  settingsConversationId: string | null;
  requestedPage: number;
  memoryOverview: MemoryOverviewResponse | null;
  memorySettings: MemorySettingsResult | null;
  memoryPreferences: MemoryPreferenceItem[];
  memoryFacts: MemoryFactItem[];
  memoryRecent: MemoryRecentItem[];
  memoryLoading: boolean;
  memoryError: string | null;
  memoryMessage: string | null;
  memoryActionLoading: string | null;
  memorySettingsSaving: boolean;
  memorySettingScope: MemorySettingScope;
  memoryClearScope: MemoryClearScope;
  memoryPage: MemoryPageResult | null;
  memoryRemoteDisclosure: RemoteMemoryDisclosure | null;
  memoryVectorStatus: MemoryVectorStatus | null;
  memoryUndoRevisionId: string | null;
  memoryFilters: MemoryListFilters;
}

export function emptyMemoryView(owner: MemoryOwner): MemoryView {
  return {
    owner, settingsConversationId: null, requestedPage: 1,
    memoryOverview: null, memorySettings: null, memoryPreferences: [], memoryFacts: [], memoryRecent: [],
    memoryLoading: false, memoryError: null, memoryMessage: null, memoryActionLoading: null,
    memorySettingsSaving: false, memorySettingScope: "principal", memoryClearScope: "recent",
    memoryPage: null, memoryRemoteDisclosure: null, memoryVectorStatus: null, memoryUndoRevisionId: null,
    memoryFilters: { search: "", scope: "", origin: "", kind: "", status: "", freshness: "" },
  };
}

export function settingsDomain(view: MemoryView): string {
  return `settings:${JSON.stringify([view.memorySettingScope, view.settingsConversationId])}`;
}

/** Request order is separate from the existing authenticated-owner identity. */
export function memoryRequests(owns: MemoryOwner) {
  const lanes = new Map<string, object>();
  const domains = new Map<string, object>();
  const domain = (key: string) => {
    if (!domains.has(key)) domains.set(key, {});
    return domains.get(key);
  };
  const invalidate = (...keys: string[]) => {
    if (owns()) for (const key of keys) domains.set(key, {});
  };
  return {
    token(lane: string, keys: string[] = []): MemoryOwner {
      if (!owns()) return () => false;
      const ticket = {};
      lanes.set(lane, ticket);
      const versions = keys.map(key => [key, domain(key)] as const);
      return () => owns() && lanes.get(lane) === ticket && versions.every(([key, value]) => domains.get(key) === value);
    },
    write(...keys: string[]) {
      invalidate(...keys);
      let finished = false;
      return () => {
        if (!finished) { finished = true; invalidate(...keys); }
      };
    },
    selectSettingsDomain(key: string) {
      // Invalidate departed views, not an arbitrary count of conversations.
      for (const name of domains.keys()) if (name.startsWith("settings:") && name !== key) domains.delete(name);
    },
  };
}
