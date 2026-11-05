import type { SkillsConfigResponse } from "../types/api";
import { isVisibleSkillName } from "./skill-display";
import { skillSwitchChanges, skillSwitchIsChanged, skillSwitchLocks, type SkillSwitchAcknowledgement } from "./skill-switches";

/** One view owner's pending intent; neither authorization nor a shared cache. */
export class SkillSwitchDraft {
  baseline: SkillsConfigResponse | null = null;
  private edits = new Map<string, { enabled: boolean; sequence: number }>();
  private sequence = 0;
  private writing = false;

  values(): Record<string, boolean> {
    const locked = skillSwitchLocks(this.baseline);
    const entries = Object.entries(this.baseline?.skill_switches ?? {}).map(([name, enabled]) =>
      [name, locked.has(name) ? true : enabled] as const);
    return Object.fromEntries([...entries, ...[...this.edits].map(([name, edit]) => [name, edit.enabled] as const)]);
  }

  changes(): Record<string, boolean> {
    return skillSwitchChanges(this.baseline, this.values(), skillSwitchLocks(this.baseline));
  }

  set(name: string, enabled: boolean): void {
    const locked = skillSwitchLocks(this.baseline);
    if (!this.baseline || !isVisibleSkillName(name) || locked.has(name)) return;
    const changed = skillSwitchIsChanged(this.baseline, { ...this.baseline.skill_switches, [name]: enabled }, name, locked);
    if (!this.writing && !changed) this.edits.delete(name);
    else this.edits.set(name, { enabled, sequence: ++this.sequence });
  }

  rebase(baseline: SkillsConfigResponse): void {
    this.baseline = baseline;
    // Reads are excluded while a write is pending. An acknowledged write has
    // already supplied its authoritative baseline before the follow-up read.
    if (!this.writing) this.pruneSatisfied();
  }

  beginWrite(): number {
    this.writing = true;
    return this.sequence;
  }

  acknowledge(ack: SkillSwitchAcknowledgement, sequence: number, changes: Readonly<Record<string, boolean>>): void {
    if (!this.baseline) return;
    for (const [name, edit] of this.edits) {
      if (Object.prototype.hasOwnProperty.call(changes, name) && edit.sequence <= sequence) this.edits.delete(name);
    }
    this.baseline = { ...this.baseline, ...ack };
    this.writing = false;
    this.pruneSatisfied();
  }

  finishWrite(): void {
    this.writing = false;
    // A failed/ambiguous response does not prove that the server did nothing.
    // Retain later intent until a fresh read can reconcile the actual state.
  }

  private pruneSatisfied(): void {
    const baseline = this.baseline;
    if (!baseline) return;
    const locked = skillSwitchLocks(baseline);
    for (const [name, edit] of this.edits) {
      if (!skillSwitchIsChanged(baseline, { ...baseline.skill_switches, [name]: edit.enabled }, name, locked)) {
        this.edits.delete(name);
      }
    }
  }
}
