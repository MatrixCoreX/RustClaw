import { useId, useLayoutEffect, useRef, useState } from "react";

interface Props {
  text: string;
  t: (zh: string, en: string) => string;
  className?: string;
}

/** Presentation-only disclosure: the complete string remains available, never sliced. */
export function ExpandableMemoryText({ text, t, className }: Props) {
  const id = useId();
  const paragraph = useRef<HTMLParagraphElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => { setExpanded(false); setOverflows(false); }, [text]);
  useLayoutEffect(() => {
    const node = paragraph.current;
    if (!node || expanded) return;
    const measure = () => setOverflows(node.scrollHeight > node.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [text, expanded]);

  return <div className={className}>
    <p id={id} ref={paragraph} className={`whitespace-pre-wrap break-words${expanded ? "" : " line-clamp-3"}`}>{text}</p>
    {overflows ? <button type="button" className="theme-secondary-btn mt-2 px-2.5 py-1.5 text-xs"
      aria-controls={id} aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>
      {expanded ? t("收起全文", "Collapse full text") : t("展开全文", "Show full text")}
    </button> : null}
  </div>;
}
