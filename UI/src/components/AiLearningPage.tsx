import {
  Children,
  isValidElement,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
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
import { classifyLearningLink, parseReadmeLearningPages } from "../lib/ai-learning";

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
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const panOriginRef = useRef<{
    pointerId: number;
    clientX: number;
    clientY: number;
    scrollLeft: number;
    scrollTop: number;
  } | null>(null);
  const [theme, setTheme] = useState<"dark" | "neutral">(() => currentMermaidTheme());
  const [zoom, setZoom] = useState(1);
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState(false);
  const [isPanning, setIsPanning] = useState(false);

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

  const resetView = () => {
    setZoom(1);
    viewportRef.current?.scrollTo({ left: 0, top: 0 });
  };

  const beginPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const viewport = viewportRef.current;
    if (
      event.button !== 0
      || !viewport
      || (
        viewport.scrollWidth <= viewport.clientWidth
        && viewport.scrollHeight <= viewport.clientHeight
      )
    ) {
      return;
    }
    panOriginRef.current = {
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      scrollLeft: viewport.scrollLeft,
      scrollTop: viewport.scrollTop,
    };
    viewport.setPointerCapture(event.pointerId);
    setIsPanning(true);
    event.preventDefault();
  };

  const movePan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const viewport = viewportRef.current;
    const origin = panOriginRef.current;
    if (!viewport || !origin || origin.pointerId !== event.pointerId) return;
    viewport.scrollLeft = origin.scrollLeft - (event.clientX - origin.clientX);
    viewport.scrollTop = origin.scrollTop - (event.clientY - origin.clientY);
    event.preventDefault();
  };

  const endPan = (event: ReactPointerEvent<HTMLDivElement>) => {
    const viewport = viewportRef.current;
    const origin = panOriginRef.current;
    if (!origin || origin.pointerId !== event.pointerId) return;
    if (viewport?.hasPointerCapture(event.pointerId)) {
      viewport.releasePointerCapture(event.pointerId);
    }
    panOriginRef.current = null;
    setIsPanning(false);
  };

  const panWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const direction = {
      ArrowLeft: [-80, 0],
      ArrowRight: [80, 0],
      ArrowUp: [0, -80],
      ArrowDown: [0, 80],
    }[event.key];
    if (!direction || !viewportRef.current) return;
    viewportRef.current.scrollBy({ left: direction[0], top: direction[1], behavior: "smooth" });
    event.preventDefault();
  };

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
            onClick={resetView}
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
      <div
        ref={viewportRef}
        className={`theme-scrollbar overflow-auto p-4 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-orange-400/60 sm:p-6 ${
          isPanning ? "cursor-grabbing select-none" : "cursor-grab"
        } ${expanded ? "min-h-0 flex-1" : "max-h-[70vh]"}`}
        style={{ touchAction: zoom > 1 || expanded ? "none" : "pan-y" }}
        role="region"
        tabIndex={0}
        aria-label={t("可缩放和拖动的流程图", "Zoomable and pannable flow diagram")}
        onPointerDown={beginPan}
        onPointerMove={movePan}
        onPointerUp={endPan}
        onPointerCancel={endPan}
        onKeyDown={panWithKeyboard}
      >
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
  const chapters = useMemo(() => {
    const grouped: Array<{
      id: string;
      title: string;
      pages: Array<{ index: number; page: (typeof pages)[number] }>;
    }> = [];
    pages.forEach((item, index) => {
      const current = grouped[grouped.length - 1];
      if (!current || current.id !== item.chapterId) {
        grouped.push({
          id: item.chapterId,
          title: item.chapterTitle,
          pages: [{ index, page: item }],
        });
      } else {
        current.pages.push({ index, page: item });
      }
    });
    return grouped;
  }, [pages]);
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
      a: ({ href, children }) => {
        const linkKind = classifyLearningLink(href);
        if (linkKind === "external") {
          return <a href={href} target="_blank" rel="noreferrer">{children}</a>;
        }
        if (linkKind === "internal") {
          return <a href={href}>{children}</a>;
        }
        return (
          <span
            className="learning-reference"
            title={t("仓库内参考资料", "Repository reference")}
          >
            {children}
          </span>
        );
      },
    }),
    [lang, t],
  );

  if (!page) return null;
  const chapterIndex = chapters.findIndex((chapter) => chapter.id === page.chapterId);

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
                {t("内容整理自项目说明，按主题与具体流程细分；流程图支持缩放和全屏查看。", "Project documentation is organized into focused topics and flows, with zoomable full-screen diagrams.")}
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
            {chapters.map((chapter) => (
              <optgroup key={chapter.id} label={chapter.title}>
                {chapter.pages.map(({ index, page: item }) => (
                  <option key={item.id} value={index}>
                    {index + 1}. {item.kind === "chapter" && chapter.pages.length > 1
                      ? t("主题概览", "Topic overview")
                      : item.title}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          <nav
            className="theme-scrollbar hidden max-h-[calc(100vh-10rem)] space-y-3 overflow-y-auto pr-1 lg:block"
            aria-label={t("学习主题", "Learning topics")}
          >
            {chapters.map((chapter) => (
              <section key={chapter.id}>
                {chapter.pages.length > 1 && (
                  <p className="px-3 pb-1 text-[11px] font-medium leading-5 text-[var(--theme-text-soft)]">
                    {chapter.title}
                  </p>
                )}
                <div className="space-y-1">
                  {chapter.pages.map(({ index, page: item }) => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => setPageIndex(index)}
                      className={`w-full rounded-md px-3 py-2 text-left text-sm leading-5 transition ${index === pageIndex ? "bg-orange-400/12 font-medium text-[var(--theme-text-strong)]" : "text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"}`}
                      aria-current={index === pageIndex ? "page" : undefined}
                    >
                      <span className="mr-2 font-mono text-[10px] text-[var(--theme-text-faint)]">
                        {String(index + 1).padStart(2, "0")}
                      </span>
                      {item.kind === "chapter" && chapter.pages.length > 1
                        ? t("主题概览", "Topic overview")
                        : item.title}
                    </button>
                  ))}
                </div>
              </section>
            ))}
          </nav>
        </aside>

        <main className="min-w-0 px-4 py-5 sm:px-7 sm:py-7">
          <div className="mb-5 flex flex-wrap items-center gap-2 text-[11px] text-[var(--theme-text-faint)]">
            <span>{t("主题", "Topic")} {chapterIndex + 1} / {chapters.length}</span>
            <span aria-hidden="true">/</span>
            <span>{t("内容页", "Page")} {pageIndex + 1} / {pages.length}</span>
            {page.diagramCount > 0 && (
              <>
                <span aria-hidden="true">/</span>
                <span>{page.diagramCount} {t("张流程图", "diagrams")}</span>
              </>
            )}
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
