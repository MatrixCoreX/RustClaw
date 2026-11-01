import { useState } from "react";

type Translate = (zh: string, en: string) => string;

export function KbSearchExcerpt({ text, t }: { text: string; t: Translate }) {
  const [expanded, setExpanded] = useState(false);
  const isPreview = text.length > 8_000;
  let previewEnd = 8_000;
  if (
    text.charCodeAt(previewEnd - 1) >= 0xd800
    && text.charCodeAt(previewEnd - 1) <= 0xdbff
    && text.charCodeAt(previewEnd) >= 0xdc00
    && text.charCodeAt(previewEnd) <= 0xdfff
  ) previewEnd -= 1;

  return (
    <div className="mt-2">
      <p className={`whitespace-pre-wrap text-sm leading-6 text-white/75${expanded ? " max-h-72 overflow-y-auto" : ""}`}>
        {isPreview && !expanded ? text.slice(0, previewEnd) : text}
      </p>
      {isPreview ? (
        <button
          type="button"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
          className="mt-2 rounded-lg border border-white/15 px-3 py-1.5 text-xs text-white/70 transition hover:bg-white/10"
        >
          {expanded ? t("收起片段", "Collapse excerpt") : t("当前为预览 · 查看完整片段", "Preview shown · Show full excerpt")}
        </button>
      ) : null}
    </div>
  );
}
