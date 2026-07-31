import { UI_BUILD_VERSION } from "../lib/build-info";

type Translate = (zh: string, en: string) => string;

export interface UiBuildBadgeProps {
  t: Translate;
  className?: string;
}

export function UiBuildBadge({ t, className = "" }: UiBuildBadgeProps) {
  const title = t(
    `前端版本 ${UI_BUILD_VERSION}。两个页面显示相同版本时，使用的是同一份前端代码。`,
    `UI version ${UI_BUILD_VERSION}. Matching versions mean both pages use the same UI source.`,
  );
  return (
    <span
      className={`inline-flex shrink-0 items-center rounded-md border border-white/10 bg-white/5 px-1.5 py-0.5 font-mono text-[10px] font-medium tracking-wide text-white/55 ${className}`.trim()}
      data-ui-build-version={UI_BUILD_VERSION}
      title={title}
      aria-label={title}
    >
      UI {UI_BUILD_VERSION}
    </span>
  );
}
