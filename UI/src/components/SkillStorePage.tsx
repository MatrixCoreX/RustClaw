import { SkillImportPanel, type SkillImportPanelProps } from "./SkillImportPanel";
import { SkillStoreCatalog, type SkillStoreCatalogProps } from "./SkillStoreCatalog";
import { GitRemoteSetupPanel, type GitRemoteSetupPanelProps } from "./GitRemoteSetupPanel";

export type SkillStorePageProps = SkillImportPanelProps & SkillStoreCatalogProps & GitRemoteSetupPanelProps;

export function SkillStorePage(props: SkillStorePageProps) {
  return (
    <section className="space-y-5">
      <SkillStoreCatalog {...props} />
      <GitRemoteSetupPanel apiFetch={props.apiFetch} t={props.t} canManage={props.canManage} />
      <SkillImportPanel {...props} />
    </section>
  );
}
