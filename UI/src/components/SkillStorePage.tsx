import { SkillImportPanel, type SkillImportPanelProps } from "./SkillImportPanel";
import { SkillStoreCatalog, type SkillStoreCatalogProps } from "./SkillStoreCatalog";

export type SkillStorePageProps = SkillImportPanelProps & SkillStoreCatalogProps;

export function SkillStorePage(props: SkillStorePageProps) {
  return (
    <section className="space-y-5">
      <SkillStoreCatalog {...props} />
      <SkillImportPanel {...props} />
    </section>
  );
}
