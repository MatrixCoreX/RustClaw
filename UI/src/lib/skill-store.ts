import type { SkillStoreItem } from "../types/api";

export function filterSkillStoreItems(items: SkillStoreItem[], query: string): SkillStoreItem[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return items;
  return items.filter((item) =>
    [item.name, item.description, item.group, item.source_kind]
      .filter(Boolean)
      .some((value) => String(value).toLocaleLowerCase().includes(normalized)),
  );
}

export function skillStoreInstallState(item: SkillStoreItem): "installed" | "not_installed" {
  return item.installed ? "installed" : "not_installed";
}
