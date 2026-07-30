import type { LearningAudience } from "./ai-learning";
import { appStorageKey } from "./product-identity";

export interface AiLearningProgress {
  audience: LearningAudience;
  visitedPageIds: string[];
  lastPageByAudience: Partial<Record<LearningAudience, string>>;
}

interface LearningProgressStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const AUDIENCES: LearningAudience[] = ["beginner", "operator", "developer"];
const STORAGE_PREFIX = appStorageKey("ai-learning.progress.v1");

function storageKey(language: string): string {
  return `${STORAGE_PREFIX}.${language}`;
}

export function emptyLearningProgress(): AiLearningProgress {
  return {
    audience: "beginner",
    visitedPageIds: [],
    lastPageByAudience: {},
  };
}

export function loadLearningProgress(
  storage: LearningProgressStorage,
  language: string,
  validPageIds: Set<string>,
): AiLearningProgress {
  const fallback = emptyLearningProgress();
  try {
    const canonicalKey = storageKey(language);
    const raw = storage.getItem(canonicalKey);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as Partial<AiLearningProgress>;
    const audience = AUDIENCES.includes(parsed.audience as LearningAudience)
      ? parsed.audience as LearningAudience
      : fallback.audience;
    const visitedPageIds = Array.isArray(parsed.visitedPageIds)
      ? [...new Set(parsed.visitedPageIds.filter(
          (id): id is string => typeof id === "string" && validPageIds.has(id),
        ))]
      : [];
    const lastPageByAudience: Partial<Record<LearningAudience, string>> = {};
    for (const item of AUDIENCES) {
      const pageId = parsed.lastPageByAudience?.[item];
      if (typeof pageId === "string" && validPageIds.has(pageId)) {
        lastPageByAudience[item] = pageId;
      }
    }
    return { audience, visitedPageIds, lastPageByAudience };
  } catch {
    return fallback;
  }
}

export function saveLearningProgress(
  storage: LearningProgressStorage,
  language: string,
  progress: AiLearningProgress,
): void {
  try {
    storage.setItem(storageKey(language), JSON.stringify(progress));
  } catch {
    // Learning progress is optional; private/incognito storage may reject writes.
  }
}
