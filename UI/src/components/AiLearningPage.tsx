import {
  Children,
  isValidElement,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  Blocks,
  BookOpenCheck,
  BrainCircuit,
  ChevronLeft,
  ChevronRight,
  Code2,
  Compass,
  LoaderCircle,
  Maximize2,
  Minimize2,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Workflow,
  ZoomIn,
  ZoomOut,
  type LucideIcon,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";

import readmeEn from "../../../README.md?raw";
import readmeZh from "../../../README.zh-CN.md?raw";
import {
  classifyLearningLink,
  orderLearningPagesByStage,
  parseReadmeLearningPages,
  parseStandaloneLearningDocument,
  type AiLearningPage as LearningPage,
} from "../lib/ai-learning";
import {
  fitDiagramScale,
  readDiagramSize,
  renderMermaid,
  scaledDiagramSize,
  type DiagramSize,
} from "../lib/mermaid-viewer";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

export interface AiLearningPageProps {
  lang: UiLanguage;
  t: Translate;
}

const ARCHITECTURE_DOCUMENT_MODULES = import.meta.glob(
  "../../../docs/architecture/[0-9][0-9]-*.md",
  { eager: true, import: "default", query: "?raw" },
) as Record<string, string>;

function architectureDocuments(lang: UiLanguage): string[] {
  return Object.entries(ARCHITECTURE_DOCUMENT_MODULES)
    .filter(([file]) => file.endsWith(".zh-CN.md") === (lang === "zh"))
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([, markdown]) => markdown);
}

const ARCHITECTURE_DOCUMENTS = {
  en: architectureDocuments("en"),
  zh: architectureDocuments("zh"),
} satisfies Record<UiLanguage, string[]>;

const LEARNING_STAGE_ORDER = [
  "foundations",
  "agent-runtime",
  "context-memory",
  "safety-operations",
  "capabilities-artifacts",
  "development-release",
] as const;

const ARCHITECTURE_STAGE_IDS = [
  "agent-runtime",
  "safety-operations",
  "context-memory",
  "development-release",
  "capabilities-artifacts",
  "development-release",
  "capabilities-artifacts",
  "capabilities-artifacts",
] as const;

interface LearningStageDefinition {
  id: string;
  title: string;
  level: string;
  description: string;
  icon: LucideIcon;
}

interface IndexedLearningPage {
  index: number;
  page: LearningPage;
}

interface LearningChapter {
  id: string;
  title: string;
  pages: IndexedLearningPage[];
}

interface LearningStage extends LearningStageDefinition {
  chapters: LearningChapter[];
  pages: IndexedLearningPage[];
}

function stageDefinitions(t: Translate): LearningStageDefinition[] {
  return [
    {
      id: "foundations",
      title: t("认识 RustClaw", "Meet RustClaw"),
      level: t("入门", "Start"),
      description: t(
        "先建立产品边界和整体认识，知道 RustClaw 能做什么，以及各部分如何协作。",
        "Build a clear product-level mental model before moving into runtime details.",
      ),
      icon: Compass,
    },
    {
      id: "agent-runtime",
      title: t("Agent 核心", "Agent Core"),
      level: t("核心", "Core"),
      description: t(
        "沿着一次请求理解 planner、capability、工具调用、验证和最终回复之间的关系。",
        "Follow one request through planning, capabilities, tool execution, verification, and response synthesis.",
      ),
      icon: Workflow,
    },
    {
      id: "context-memory",
      title: t("上下文与记忆", "Context & Memory"),
      level: t("进阶", "Deeper"),
      description: t(
        "理解任务状态、上下文预算、知识召回、记忆写入以及后台续跑。",
        "Understand task state, context budgets, retrieval, memory writes, and background resume.",
      ),
      icon: BrainCircuit,
    },
    {
      id: "safety-operations",
      title: t("安全与运行", "Safety & Operations"),
      level: t("实践", "Operate"),
      description: t(
        "掌握身份、权限、沙箱、部署入口和 UI/API 边界，安全地运行系统。",
        "Learn identity, permissions, sandboxing, deployment entry points, and UI/API boundaries.",
      ),
      icon: ShieldCheck,
    },
    {
      id: "capabilities-artifacts",
      title: t("能力与工件", "Capabilities & Artifacts"),
      level: t("扩展", "Extend"),
      description: t(
        "了解技能、模型、多媒体、Office 工件和技能独立存储如何接入 Agent Loop。",
        "See how skills, models, media, Office artifacts, and skill-owned storage join the agent loop.",
      ),
      icon: Blocks,
    },
    {
      id: "development-release",
      title: t("开发与发布", "Build & Release"),
      level: t("维护", "Maintain"),
      description: t(
        "面向开发和维护，覆盖代码观测、回归验证、目录边界与发布门禁。",
        "For maintainers: coding observability, regression checks, repository boundaries, and release gates.",
      ),
      icon: Code2,
    },
  ];
}

function groupLearningChapters(pages: IndexedLearningPage[]): LearningChapter[] {
  const chapters: LearningChapter[] = [];
  pages.forEach((entry) => {
    const current = chapters[chapters.length - 1];
    if (!current || current.id !== entry.page.chapterId) {
      chapters.push({
        id: entry.page.chapterId,
        title: entry.page.chapterTitle,
        pages: [entry],
      });
    } else {
      current.pages.push(entry);
    }
  });
  return chapters;
}

function currentMermaidTheme(): "dark" | "neutral" {
  return document.documentElement.dataset.theme === "light" ? "neutral" : "dark";
}

function MermaidDiagram({ source, lang }: { source: string; lang: UiLanguage }) {
  const t = (zh: string, en: string) => (lang === "zh" ? zh : en);
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
  const [fitToViewport, setFitToViewport] = useState(true);
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(true);
  const [renderAttempt, setRenderAttempt] = useState(0);
  const [diagramSize, setDiagramSize] = useState<DiagramSize | null>(null);
  const [renderedSvg, setRenderedSvg] = useState<string | null>(null);
  const bindFunctionsRef = useRef<((element: Element) => void) | undefined>(undefined);
  const renderSequenceRef = useRef(0);
  const [isPanning, setIsPanning] = useState(false);

  useEffect(() => {
    const observer = new MutationObserver(() => setTheme(currentMermaidTheme()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const renderSequence = ++renderSequenceRef.current;
    setError(false);
    setLoading(true);
    setDiagramSize(null);
    setRenderedSvg(null);
    bindFunctionsRef.current = undefined;
    void renderMermaid(
      `rustclaw-${diagramId}-${theme}-${renderSequence}-${renderAttempt}`,
      source,
      theme,
    )
      .then((result) => {
        if (renderSequenceRef.current !== renderSequence) return;
        const size = readDiagramSize(result.svg);
        if (!size) throw new Error("invalid_svg_dimensions");
        bindFunctionsRef.current = result.bindFunctions;
        setRenderedSvg(result.svg);
        setDiagramSize(size);
        setLoading(false);
      })
      .catch(() => {
        if (renderSequenceRef.current !== renderSequence) return;
        setLoading(false);
        setError(true);
      });
    return () => {
      if (renderSequenceRef.current === renderSequence) {
        renderSequenceRef.current += 1;
      }
    };
  }, [diagramId, renderAttempt, source, theme]);

  useLayoutEffect(() => {
    if (renderedSvg && containerRef.current) {
      bindFunctionsRef.current?.(containerRef.current);
    }
  }, [renderedSvg]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !diagramSize || !fitToViewport) return;
    const updateFit = () => {
      const horizontalPadding = window.innerWidth >= 640 ? 48 : 32;
      setZoom(fitDiagramScale(viewport.clientWidth - horizontalPadding, diagramSize.width));
    };
    updateFit();
    const observer = new ResizeObserver(updateFit);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [diagramSize, expanded, fitToViewport]);

  useEffect(() => {
    if (!expanded) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [expanded]);

  const resetView = () => {
    setFitToViewport(true);
    viewportRef.current?.scrollTo({ left: 0, top: 0 });
  };

  const updateZoom = (delta: number) => {
    setFitToViewport(false);
    setZoom((value) => Math.min(2.5, Math.max(0.1, Number((value + delta).toFixed(1)))));
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

  const canvasSize = diagramSize ? scaledDiagramSize(diagramSize, zoom) : null;
  const diagram = (
    <figure
      className={
        expanded
          ? "fixed inset-3 z-[200] flex min-h-0 flex-col overflow-hidden rounded-lg border border-white/15 bg-[var(--theme-body-bg)] shadow-2xl sm:inset-6"
          : "my-6 overflow-hidden rounded-lg border border-white/10 bg-[var(--theme-card-strong)]"
      }
      role={expanded ? "dialog" : undefined}
      aria-modal={expanded || undefined}
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
            onClick={() => updateZoom(-0.2)}
          >
            <ZoomOut className="h-4 w-4" />
          </button>
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={t("适应窗口", "Fit to window")}
            aria-label={t("让流程图适应窗口", "Fit diagram to window")}
            onClick={resetView}
          >
            <RotateCcw className="h-4 w-4" />
          </button>
          <button
            type="button"
            className="theme-topbar-nav-btn !min-h-8 !px-2"
            title={t("放大", "Zoom in")}
            aria-label={t("放大流程图", "Zoom in diagram")}
            onClick={() => updateZoom(0.2)}
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
        {loading ? (
          <div className="flex min-h-40 items-center justify-center text-[var(--theme-text-muted)]">
            <LoaderCircle className="h-5 w-5 animate-spin" aria-hidden="true" />
            <span className="ml-2 text-sm">{t("正在生成流程图...", "Rendering diagram...")}</span>
          </div>
        ) : error ? (
          <div className="rounded-lg border border-amber-500/25 bg-amber-500/10 p-4">
            <p className="text-sm text-amber-100">{t("流程图暂时无法渲染，下面保留原始 Mermaid 定义。", "The diagram could not be rendered. Its Mermaid source is preserved below.")}</p>
            <button
              type="button"
              className="theme-secondary-btn mt-3 !px-3"
              onClick={() => setRenderAttempt((value) => value + 1)}
            >
              <RefreshCw className="h-4 w-4" />
              {t("重新加载", "Retry")}
            </button>
            <pre className="mt-3 overflow-auto text-xs text-[var(--theme-text-body)]"><code>{source}</code></pre>
          </div>
        ) : null}
        {!error ? (
          <div
            className={`relative mx-auto ${loading ? "hidden" : ""}`}
            style={canvasSize ? { width: canvasSize.width, height: canvasSize.height } : undefined}
          >
            <div
              ref={containerRef}
              className="mermaid-canvas absolute left-0 top-0 origin-top-left"
              style={diagramSize ? {
                width: diagramSize.width,
                height: diagramSize.height,
                transform: `scale(${zoom})`,
              } : undefined}
              dangerouslySetInnerHTML={renderedSvg ? { __html: renderedSvg } : undefined}
            />
          </div>
        ) : null}
      </div>
    </figure>
  );
  return expanded ? createPortal(diagram, document.body) : diagram;
}

function mermaidSource(children: ReactNode): string | null {
  const child = Children.toArray(children)[0];
  if (!isValidElement<{ className?: string; children?: ReactNode }>(child)) return null;
  if (!child.props.className?.split(" ").includes("language-mermaid")) return null;
  return String(child.props.children ?? "").replace(/\n$/, "");
}

export function AiLearningPage({ lang, t }: AiLearningPageProps) {
  const pages = useMemo(() => {
    const readmePages = parseReadmeLearningPages(lang === "zh" ? readmeZh : readmeEn);
    const chapterTitle = lang === "zh" ? "架构指南" : "Architecture Guide";
    const architecturePages = ARCHITECTURE_DOCUMENTS[lang].map((markdown, index) =>
      parseStandaloneLearningDocument({
        id: `architecture-guide-${index + 1}`,
        chapterId: "architecture-guide",
        chapterTitle,
        stageId: ARCHITECTURE_STAGE_IDS[index] ?? "development-release",
        markdown,
      }));
    return orderLearningPagesByStage(
      [...readmePages, ...architecturePages],
      [...LEARNING_STAGE_ORDER],
    );
  }, [lang]);
  const stages = useMemo<LearningStage[]>(() => {
    const definitions = stageDefinitions(t);
    return definitions
      .map((definition) => {
        const stagePages = pages
          .map((page, index) => ({ index, page }))
          .filter(({ page }) => page.stageId === definition.id);
        return {
          ...definition,
          pages: stagePages,
          chapters: groupLearningChapters(stagePages),
        };
      })
      .filter((stage) => stage.pages.length > 0);
  }, [pages, t]);
  const [pageIndex, setPageIndex] = useState(0);
  const stageNavRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    setPageIndex((index) => Math.min(index, Math.max(0, pages.length - 1)));
  }, [pages.length]);

  useEffect(() => {
    window.scrollTo({ top: 0, behavior: "smooth" });
  }, [pageIndex]);

  const page = pages[pageIndex];

  useEffect(() => {
    const stageId = page?.stageId;
    const scroller = stageNavRef.current;
    if (!stageId || !scroller) return;
    const activeButton = scroller.querySelector<HTMLElement>(
      `[data-learning-stage="${stageId}"]`,
    );
    if (!activeButton) return;
    const targetLeft =
      activeButton.offsetLeft - (scroller.clientWidth - activeButton.clientWidth) / 2;
    scroller.scrollTo({ left: Math.max(0, targetLeft), behavior: "smooth" });
  }, [page?.stageId]);

  const markdownComponents = useMemo<Components>(
    () => ({
      pre: ({ children }) => {
        const source = mermaidSource(children);
        return source ? <MermaidDiagram source={source} lang={lang} /> : <pre>{children}</pre>;
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
    [lang],
  );

  if (!page || stages.length === 0) return null;
  const stageIndex = stages.findIndex((stage) => stage.id === page.stageId);
  const activeStage = stages[Math.max(0, stageIndex)] ?? stages[0];
  const previousPage = pages[pageIndex - 1];
  const nextPage = pages[pageIndex + 1];

  return (
    <section className="overflow-hidden rounded-lg border border-[var(--theme-border)] bg-[var(--theme-card)]">
      <header className="border-b border-[var(--theme-border)] px-4 py-5 sm:px-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-[var(--theme-border)] bg-[var(--theme-card-strong)] text-[var(--theme-icon-accent-color)]">
              <BookOpenCheck className="h-5 w-5" />
            </span>
            <div>
              <p className="theme-kicker text-[10px] uppercase">{t("AI 学习", "AI Learning")}</p>
              <h2 className="mt-1 text-lg font-semibold text-[var(--theme-text-strong)]">
                {t("从使用到架构，分阶段理解 RustClaw", "Learn RustClaw from everyday use to architecture")}
              </h2>
              <p className="mt-1 max-w-3xl text-sm leading-6 text-[var(--theme-text-muted)]">
                {t("先建立整体认识，再逐步进入 Agent、记忆、安全、技能和开发细节。内容与仓库文档同步，流程图支持缩放、拖动和全屏查看。", "Start with the product view, then move through the agent, memory, safety, capabilities, and development layers. Content stays synchronized with repository docs, with zoomable and pannable diagrams.")}
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

      <nav
        ref={stageNavRef}
        className="theme-scrollbar overflow-x-auto border-b border-[var(--theme-border)] px-3 py-3 sm:px-5"
        aria-label={t("学习路线", "Learning path")}
      >
        <div className="grid min-w-[780px] grid-cols-6 overflow-hidden rounded-md border border-[var(--theme-border)]">
          {stages.map((stage, index) => {
            const StageIcon = stage.icon;
            const isActive = stage.id === activeStage.id;
            return (
              <button
                key={stage.id}
                type="button"
                data-learning-stage={stage.id}
                className={`min-w-0 border-r border-[var(--theme-border)] px-3 py-3 text-left transition last:border-r-0 ${
                  isActive
                    ? "bg-orange-400/12 text-[var(--theme-text-strong)]"
                    : "bg-[var(--theme-card-strong)] text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"
                }`}
                onClick={() => setPageIndex(stage.pages[0].index)}
                aria-current={isActive ? "step" : undefined}
              >
                <span className="flex items-center gap-2">
                  <StageIcon className={`h-4 w-4 shrink-0 ${isActive ? "text-[var(--theme-icon-accent-color)]" : "text-[var(--theme-text-faint)]"}`} />
                  <span className="text-[10px] uppercase text-[var(--theme-text-faint)]">
                    {index + 1}. {stage.level}
                  </span>
                </span>
                <span className="mt-1 block truncate text-xs font-medium">{stage.title}</span>
              </button>
            );
          })}
        </div>
      </nav>

      <div className="grid min-h-[65vh] lg:grid-cols-[280px_minmax(0,1fr)]">
        <aside className="border-b border-[var(--theme-border)] p-3 lg:border-b-0 lg:border-r">
          <label className="mb-2 block px-2 text-[10px] uppercase text-[var(--theme-text-faint)]" htmlFor="ai-learning-page">
            {t("学习目录", "Learning contents")}
          </label>
          <select
            id="ai-learning-page"
            className="theme-input w-full lg:hidden"
            value={pageIndex}
            onChange={(event) => setPageIndex(Number(event.target.value))}
          >
            {stages.map((stage) => (
              <optgroup key={stage.id} label={`${stage.level} · ${stage.title}`}>
                {stage.pages.map(({ index, page: item }) => (
                  <option key={item.id} value={index}>
                    {index + 1}. {item.title}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          <nav
            className="theme-scrollbar hidden max-h-[calc(100vh-10rem)] space-y-2 overflow-y-auto pr-1 lg:block"
            aria-label={t("学习主题", "Learning topics")}
          >
            {stages.map((stage, index) => {
              const StageIcon = stage.icon;
              const isActive = stage.id === activeStage.id;
              return (
                <section key={stage.id}>
                  <button
                    type="button"
                    className={`flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left transition ${
                      isActive
                        ? "bg-white/5 text-[var(--theme-text-strong)]"
                        : "text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"
                    }`}
                    onClick={() => setPageIndex(stage.pages[0].index)}
                    aria-expanded={isActive}
                  >
                    <StageIcon className={`h-4 w-4 shrink-0 ${isActive ? "text-[var(--theme-icon-accent-color)]" : "text-[var(--theme-text-faint)]"}`} />
                    <span className="min-w-0 flex-1">
                      <span className="block text-[10px] uppercase text-[var(--theme-text-faint)]">
                        {index + 1}. {stage.level}
                      </span>
                      <span className="block truncate text-xs font-medium">{stage.title}</span>
                    </span>
                    <span className="font-mono text-[10px] text-[var(--theme-text-faint)]">{stage.pages.length}</span>
                    <ChevronRight className={`h-3.5 w-3.5 shrink-0 transition ${isActive ? "rotate-90" : ""}`} />
                  </button>
                  {isActive && (
                    <div className="ml-4 mt-1 space-y-3 border-l border-[var(--theme-border)] pl-3">
                      {stage.chapters.map((chapter) => (
                        <div key={chapter.id}>
                          {(stage.chapters.length > 1 || chapter.pages.length > 1) && (
                            <p className="px-2 pb-1 text-[10px] font-medium leading-5 text-[var(--theme-text-faint)]">
                              {chapter.title}
                            </p>
                          )}
                          <div className="space-y-1">
                            {chapter.pages.map(({ index: itemIndex, page: item }) => (
                              <button
                                key={item.id}
                                type="button"
                                onClick={() => setPageIndex(itemIndex)}
                                className={`flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-xs leading-5 transition ${
                                  itemIndex === pageIndex
                                    ? "bg-orange-400/12 font-medium text-[var(--theme-text-strong)]"
                                    : "text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"
                                }`}
                                aria-current={itemIndex === pageIndex ? "page" : undefined}
                              >
                                <span className="mt-px w-5 shrink-0 font-mono text-[9px] text-[var(--theme-text-faint)]">
                                  {String(itemIndex + 1).padStart(2, "0")}
                                </span>
                                <span>{item.title}</span>
                              </button>
                            ))}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </section>
              );
            })}
          </nav>
        </aside>

        <main className="min-w-0 px-4 py-5 sm:px-7 sm:py-7">
          <div className="mb-7 border-b border-[var(--theme-border)] pb-5">
            <div className="flex flex-wrap items-center gap-2 text-[10px] uppercase text-[var(--theme-text-faint)]">
              <span>{activeStage.level}</span>
              <span aria-hidden="true">/</span>
              <span>{t("阶段", "Stage")} {stageIndex + 1} / {stages.length}</span>
              <span aria-hidden="true">/</span>
              <span>{t("内容", "Page")} {pageIndex + 1} / {pages.length}</span>
              {page.diagramCount > 0 && (
                <>
                  <span aria-hidden="true">/</span>
                  <span>{page.diagramCount} {t("张流程图", "diagrams")}</span>
                </>
              )}
            </div>
            <h3 className="mt-2 text-base font-semibold text-[var(--theme-text-strong)]">
              {activeStage.title}
            </h3>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-[var(--theme-text-muted)]">
              {activeStage.description}
            </p>
            <div className="mt-4 h-1 overflow-hidden rounded-full bg-white/8" aria-hidden="true">
              <div
                className="h-full bg-[var(--theme-icon-accent-color)] transition-[width]"
                style={{ width: `${((stageIndex + 1) / stages.length) * 100}%` }}
              />
            </div>
          </div>
          <article className="learning-markdown mx-auto max-w-4xl">
            <ReactMarkdown components={markdownComponents}>{page.markdown}</ReactMarkdown>
          </article>
          <footer className="mt-10 flex items-center justify-between gap-3 border-t border-[var(--theme-border)] pt-5">
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={pageIndex === 0}
              onClick={() => setPageIndex((index) => Math.max(0, index - 1))}
            >
              <ChevronLeft className="h-4 w-4" />
              <span className="min-w-0 text-left">
                <span className="block text-[10px] text-[var(--theme-text-faint)]">{t("上一页", "Previous")}</span>
                <span className="block max-w-[34vw] truncate text-xs sm:max-w-56">{previousPage?.title ?? t("已到开头", "Start")}</span>
              </span>
            </button>
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={pageIndex >= pages.length - 1}
              onClick={() => setPageIndex((index) => Math.min(pages.length - 1, index + 1))}
            >
              <span className="min-w-0 text-right">
                <span className="block text-[10px] text-[var(--theme-text-faint)]">{t("下一页", "Next")}</span>
                <span className="block max-w-[34vw] truncate text-xs sm:max-w-56">{nextPage?.title ?? t("已完成", "Complete")}</span>
              </span>
              <ChevronRight className="h-4 w-4" />
            </button>
          </footer>
        </main>
      </div>
    </section>
  );
}
