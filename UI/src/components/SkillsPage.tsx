import { SkillImportPanel, type SkillImportPanelProps } from "./SkillImportPanel";
import { SkillSwitchPanel, type SkillSwitchPanelProps } from "./SkillSwitchPanel";

export type SkillsPageProps = SkillImportPanelProps & SkillSwitchPanelProps;

export function SkillsPage(props: SkillsPageProps) {
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
      <SkillImportPanel {...props} />
      <SkillSwitchPanel {...props} />
    </section>
  );
}
