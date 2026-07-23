import { Children, isValidElement, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import {
  BookOpenCheck,
  ChevronLeft,
  ChevronRight,
  Maximize2,
  Minimize2,
  RotateCcw,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";

import readmeEn from "../../../README.md?raw";
import readmeZh from "../../../README.zh-CN.md?raw";
import { parseReadmeLearningPages } from "../lib/ai-learning";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

export interface AiLearningPageProps {
  lang: UiLanguage;
  t: Translate;
}

function currentMermaidTheme(): "dark" | "neutral" {
  return document.documentElement.dataset.theme === "light" ? "neutral" : "dark";
}

function MermaidDiagram({ source, t }: { source: string; t: Translate }) {
  const diagramId = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [theme, setTheme] = useState<"dark" | "neutral">(() => currentMermaidTheme());
  const [zoom, setZoom] = useState(1);
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState(false);

  useEffect(() => {
    const observer = new MutationObserver(() => setTheme(currentMermaidTheme()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let active = true;
    setError(false);
    void import("mermaid")
      .then(async ({ default: mermaid }) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme,
          flowchart: { htmlLabels: true, curve: "basis", useMaxWidth: true },
        });
        const result = await mermaid.render(`rustclaw-${diagramId}-${theme}`, source);
        if (!active || !containerRef.current) return;
        containerRef.current.innerHTML = result.svg;
        result.bindFunctions?.(containerRef.current);
      })
      .catch(() => {
        if (active) setError(true);
      });
    return () => {
      active = false;
    };
  }, [diagramId, source, theme]);

  useEffect(() => {
    if (!expanded) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [expanded]);

  return (
    <figure
      className={
        expanded
          ? "fixed inset-3 z-[70] flex min-h-0 flex-col overflow-hidden rounded-lg border border-white/15 bg-[#12161f] shadow-2xl sm:inset-6"
          : "my-6 overflow-hidden rounded-lg border border-white/10 bg-[var(--theme-card-strong)]"
      }
    >
      <figcaption className="flex min-h-11 items-center justify-between gap-3 border-b border-white/10 px-3 py-2">
        <div className="flex min-w-0 items-center gap-2 text-xs font-medium text-[var(--theme-text-muted)]">
          <span className="h-2 w-2 shrink-0 rounded-full bg-emerald-400" />
          <span className="truncate">{t("交互流程图", "Interactive flow diagram")}</span>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={t("缩小", "Zoom out")}
            aria-label={t("缩小流程图", "Zoom out diagram")}
            onClick={() => setZoom((value) => Math.max(0.6, Number((value - 0.2).toFixed(1))))}
          >
            <ZoomOut className="h-4 w-4" />
          </button>
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={t("恢复大小", "Reset zoom")}
            aria-label={t("恢复流程图大小", "Reset diagram zoom")}
            onClick={() => setZoom(1)}
          >
            <RotateCcw className="h-4 w-4" />
          </button>
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={t("放大", "Zoom in")}
            aria-label={t("放大流程图", "Zoom in diagram")}
            onClick={() => setZoom((value) => Math.min(2, Number((value + 0.2).toFixed(1))))}
          >
            <ZoomIn className="h-4 w-4" />
          </button>
          <span className="w-10 text-center font-mono text-[10px] text-[var(--theme-text-faint)]">
            {Math.round(zoom * 100)}%
          </span>
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={expanded ? t("退出全屏", "Exit full screen") : t("全屏查看", "View full screen")}
            aria-label={expanded ? t("退出全屏流程图", "Exit full-screen diagram") : t("全屏查看流程图", "View diagram full screen")}
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}
          </button>
        </div>
      </figcaption>
      <div className={`theme-scrollbar overflow-auto p-4 sm:p-6 ${expanded ? "min-h-0 flex-1" : "max-h-[70vh]"}`}>
        {error ? (
          <div className="rounded-lg border border-amber-500/25 bg-amber-500/10 p-4">
            <p className="text-sm text-amber-100">{t("流程图暂时无法渲染，下面保留原始 Mermaid 定义。", "The diagram could not be rendered. Its Mermaid source is preserved below.")}</p>
            <pre className="mt-3 overflow-auto text-xs text-[var(--theme-text-body)]"><code>{source}</code></pre>
          </div>
        ) : (
          <div
            ref={containerRef}
            className="mermaid-canvas mx-auto min-h-28 origin-top-left transition-[width] duration-150"
            style={{ width: `${zoom * 100}%` }}
          />
        )}
      </div>
    </figure>
  );
}

function mermaidSource(children: ReactNode): string | null {
  const child = Children.toArray(children)[0];
  if (!isValidElement<{ className?: string; children?: ReactNode }>(child)) return null;
  if (!child.props.className?.split(" ").includes("language-mermaid")) return null;
  return String(child.props.children ?? "").replace(/\n$/, "");
}

export function AiLearningPage({ lang, t }: AiLearningPageProps) {
  const pages = useMemo(
    () => parseReadmeLearningPages(lang === "zh" ? readmeZh : readmeEn),
    [lang],
  );
  const [pageIndex, setPageIndex] = useState(0);

  useEffect(() => {
    setPageIndex((index) => Math.min(index, Math.max(0, pages.length - 1)));
  }, [pages.length]);

  useEffect(() => {
    window.scrollTo({ top: 0, behavior: "smooth" });
  }, [pageIndex]);

  const page = pages[pageIndex];
  const markdownComponents = useMemo<Components>(
    () => ({
      pre: ({ children }) => {
        const source = mermaidSource(children);
        return source ? <MermaidDiagram source={source} t={t} /> : <pre>{children}</pre>;
      },
      a: ({ href, children }) => (
        <a href={href} target={href?.startsWith("http") ? "_blank" : undefined} rel="noreferrer">
          {children}
        </a>
      ),
    }),
    [lang],
  );

  if (!page) return null;

  return (
    <section className="overflow-hidden rounded-lg border border-white/10 bg-[var(--theme-card)]">
      <header className="border-b border-white/10 px-4 py-5 sm:px-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-orange-400/25 bg-orange-400/10 text-orange-200">
              <BookOpenCheck className="h-5 w-5" />
            </span>
            <div>
              <p className="theme-kicker text-[10px] uppercase">{t("AI 学习", "AI Learning")}</p>
              <h2 className="mt-1 text-lg font-semibold text-[var(--theme-text-strong)]">
                {t("理解 RustClaw 的设计与运行流程", "Understand RustClaw design and runtime flows")}
              </h2>
              <p className="mt-1 max-w-3xl text-sm leading-6 text-[var(--theme-text-muted)]">
                {t("内容直接来自中文 README，按主题分页；流程图可以缩放和全屏查看。", "Content comes directly from the English README, organized by topic with zoomable flow diagrams.")}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="theme-topbar-btn !px-2.5 disabled:opacity-35"
              disabled={pageIndex === 0}
              title={t("上一页", "Previous page")}
              aria-label={t("上一页", "Previous page")}
              onClick={() => setPageIndex((index) => Math.max(0, index - 1))}
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <span className="min-w-16 text-center font-mono text-xs text-[var(--theme-text-muted)]">
              {pageIndex + 1} / {pages.length}
            </span>
            <button
              type="button"
              className="theme-topbar-btn !px-2.5 disabled:opacity-35"
              disabled={pageIndex >= pages.length - 1}
              title={t("下一页", "Next page")}
              aria-label={t("下一页", "Next page")}
              onClick={() => setPageIndex((index) => Math.min(pages.length - 1, index + 1))}
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        </div>
      </header>

      <div className="grid min-h-[65vh] lg:grid-cols-[230px_minmax(0,1fr)]">
        <aside className="border-b border-white/10 p-3 lg:border-b-0 lg:border-r">
          <label className="mb-2 block px-2 text-[10px] uppercase text-[var(--theme-text-faint)]" htmlFor="ai-learning-page">
            {t("学习目录", "Learning contents")}
          </label>
          <select
            id="ai-learning-page"
            className="theme-input w-full lg:hidden"
            value={pageIndex}
            onChange={(event) => setPageIndex(Number(event.target.value))}
          >
            {pages.map((item, index) => <option key={item.id} value={index}>{index + 1}. {item.title}</option>)}
          </select>
          <nav className="hidden space-y-1 lg:block" aria-label={t("README 分页", "README pages")}>
            {pages.map((item, index) => (
              <button
                key={item.id}
                type="button"
                onClick={() => setPageIndex(index)}
                className={`w-full rounded-md px-3 py-2.5 text-left text-sm transition ${index === pageIndex ? "bg-orange-400/12 font-medium text-[var(--theme-text-strong)]" : "text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"}`}
                aria-current={index === pageIndex ? "page" : undefined}
              >
                <span className="mr-2 font-mono text-[10px] text-[var(--theme-text-faint)]">{String(index + 1).padStart(2, "0")}</span>
                {item.title}
              </button>
            ))}
          </nav>
        </aside>

        <main className="min-w-0 px-4 py-5 sm:px-7 sm:py-7">
          <div className="mb-5 flex flex-wrap items-center gap-2 text-[11px] text-[var(--theme-text-faint)]">
            <span>{t("章节", "Section")} {pageIndex + 1}</span>
            <span aria-hidden="true">/</span>
            <span>{page.subsectionCount} {t("个小节", "subsections")}</span>
            <span aria-hidden="true">/</span>
            <span>{page.diagramCount} {t("张流程图", "diagrams")}</span>
          </div>
          <article className="learning-markdown mx-auto max-w-5xl">
            <ReactMarkdown components={markdownComponents}>{page.markdown}</ReactMarkdown>
          </article>
          <footer className="mt-10 flex items-center justify-between gap-3 border-t border-white/10 pt-5">
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={pageIndex === 0}
              onClick={() => setPageIndex((index) => Math.max(0, index - 1))}
            >
              <ChevronLeft className="h-4 w-4" />
              {t("上一章", "Previous")}
            </button>
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={pageIndex >= pages.length - 1}
              onClick={() => setPageIndex((index) => Math.min(pages.length - 1, index + 1))}
            >
              {t("下一章", "Next")}
              <ChevronRight className="h-4 w-4" />
            </button>
          </footer>
        </main>
      </div>
    </section>
  );
}
