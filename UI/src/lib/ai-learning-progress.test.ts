import assert from "node:assert/strict";
import test from "node:test";

import {
  emptyLearningProgress,
  loadLearningProgress,
  saveLearningProgress,
} from "./ai-learning-progress";

class MemoryStorage {
  values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

test("round-trips language-specific learning progress", () => {
  const storage = new MemoryStorage();
  saveLearningProgress(storage, "zh", {
    audience: "operator",
    visitedPageIds: ["overview", "runtime"],
    lastPageByAudience: { operator: "runtime" },
  });

  assert.deepEqual(
    loadLearningProgress(storage, "zh", new Set(["overview", "runtime"])),
    {
      audience: "operator",
      visitedPageIds: ["overview", "runtime"],
      lastPageByAudience: { operator: "runtime" },
    },
  );
  assert.deepEqual(loadLearningProgress(storage, "en", new Set()), emptyLearningProgress());
});

test("drops corrupt, unknown, and removed progress values", () => {
  const storage = new MemoryStorage();
  storage.setItem("rustclaw.ai-learning.progress.v1.zh", JSON.stringify({
    audience: "unknown",
    visitedPageIds: ["kept", "removed", 1],
    lastPageByAudience: { beginner: "removed", developer: "kept" },
  }));

  assert.deepEqual(loadLearningProgress(storage, "zh", new Set(["kept"])), {
    audience: "beginner",
    visitedPageIds: ["kept"],
    lastPageByAudience: { developer: "kept" },
  });

  storage.setItem("rustclaw.ai-learning.progress.v1.zh", "not-json");
  assert.deepEqual(loadLearningProgress(storage, "zh", new Set()), emptyLearningProgress());
});
