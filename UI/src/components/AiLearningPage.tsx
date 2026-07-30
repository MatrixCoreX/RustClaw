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
  Search,
  ShieldCheck,
  Workflow,
  X,
  ZoomIn,
  ZoomOut,
  type LucideIcon,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";

import readmeEn from "../../../README.md?raw";
import readmeZh from "../../../README.zh-CN.md?raw";
import {
  classifyLearningLink,
  learningHeadingId,
  orderLearningPagesByStage,
  parseReadmeLearningPages,
  parseStandaloneLearningPages,
  searchLearningPages,
  type AiLearningPage as LearningPage,
  type LearningAudience,
} from "../lib/ai-learning";
import {
  loadLearningProgress,
  saveLearningProgress,
} from "../lib/ai-learning-progress";
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

interface ArchitectureDocument {
  id: string;
  markdown: string;
}

function architectureDocuments(lang: UiLanguage): ArchitectureDocument[] {
  return Object.entries(ARCHITECTURE_DOCUMENT_MODULES)
    .filter(([file]) => file.endsWith(".zh-CN.md") === (lang === "zh"))
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([file, markdown]) => ({
      id: file.split("/").pop()?.replace(/\.zh-CN\.md$|\.md$/g, "") ?? file,
      markdown,
    }));
}

const ARCHITECTURE_DOCUMENTS = {
  en: architectureDocuments("en"),
  zh: architectureDocuments("zh"),
} satisfies Record<UiLanguage, ArchitectureDocument[]>;

const LEARNING_STAGE_ORDER = [
  "foundations",
  "agent-runtime",
  "context-memory",
  "safety-operations",
  "capabilities-artifacts",
  "development-release",
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

interface LearningAudienceDefinition {
  id: LearningAudience;
  title: string;
  description: string;
  icon: LucideIcon;
}

function audienceDefinitions(t: Translate): LearningAudienceDefinition[] {
  return [
    {
      id: "beginner",
      title: t("初次使用", "Getting started"),
      description: t("先理解能做什么，以及如何完成第一次任务。", "Understand the product and complete a first task."),
      icon: Compass,
    },
    {
      id: "operator",
      title: t("使用与运维", "Use & operate"),
      description: t("学习任务、记忆、能力、安全和运行状态。", "Learn tasks, memory, capabilities, safety, and operations."),
      icon: ShieldCheck,
    },
    {
      id: "developer",
      title: t("开发与维护", "Build & maintain"),
      description: t("查看完整架构、扩展合同、验证和发布细节。", "Explore architecture, extension contracts, validation, and release details."),
      icon: Code2,
    },
  ];
}

function stageDefinitions(t: Translate): LearningStageDefinition[] {
  return [
    {
      id: "foundations",
      title: t("认识 {product_name}", "Meet {product_name}"),
      level: t("入门", "Start"),
      description: t(
        "先建立产品边界和整体认识，知道 {product_name} 能做什么，以及各部分如何协作。",
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
      `agent-diagram-${diagramId}-${theme}-${renderSequence}-${renderAttempt}`,
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

function reactNodeText(node: ReactNode): string {
  return Children.toArray(node)
    .map((child) => {
      if (typeof child === "string" || typeof child === "number") return String(child);
      return isValidElement<{ children?: ReactNode }>(child)
        ? reactNodeText(child.props.children)
        : "";
    })
    .join("");
}

export function AiLearningPage({ lang, t }: AiLearningPageProps) {
  const pages = useMemo(() => {
    const readmePages = parseReadmeLearningPages(lang === "zh" ? readmeZh : readmeEn);
    const chapterTitle = lang === "zh" ? "架构指南" : "Architecture Guide";
    const architecturePages = ARCHITECTURE_DOCUMENTS[lang].flatMap((document) =>
      parseStandaloneLearningPages({
        id: `architecture-guide-${document.id}`,
        chapterId: "architecture-guide",
        chapterTitle,
        markdown: document.markdown,
      }));
    return orderLearningPagesByStage(
      [...readmePages, ...architecturePages],
      [...LEARNING_STAGE_ORDER],
    );
  }, [lang]);
  const [audience, setAudience] = useState<LearningAudience>("beginner");
  const [pageIndex, setPageIndex] = useState(0);
  const [searchQuery, setSearchQuery] = useState("");
  const [visitedPageIds, setVisitedPageIds] = useState<string[]>([]);
  const [lastPageByAudience, setLastPageByAudience] = useState<
    Partial<Record<LearningAudience, string>>
  >({});
  const [loadedLanguage, setLoadedLanguage] = useState<UiLanguage | null>(null);
  const routePages = useMemo(
    () => pages
      .map((page, index) => ({ index, page }))
      .filter(({ page }) => page.audiences.includes(audience)),
    [audience, pages],
  );
  const stages = useMemo<LearningStage[]>(() => {
    const definitions = stageDefinitions(t);
    return definitions
      .map((definition) => {
        const stagePages = routePages
          .filter(({ page }) => page.stageId === definition.id);
        return {
          ...definition,
          pages: stagePages,
          chapters: groupLearningChapters(stagePages),
        };
      })
      .filter((stage) => stage.pages.length > 0);
  }, [routePages, t]);
  const pageIndexById = useMemo(
    () => new Map(pages.map((page, index) => [page.id, index])),
    [pages],
  );
  const searchResults = useMemo(() => {
    const resultIds = new Set(searchLearningPages(
      routePages.map(({ page }) => page),
      searchQuery,
    ).map((page) => page.id));
    return routePages.filter(({ page }) => resultIds.has(page.id));
  }, [routePages, searchQuery]);
  const stageNavRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const validPageIds = new Set(pages.map((page) => page.id));
    const progress = loadLearningProgress(window.localStorage, lang, validPageIds);
    const preferredPageId = progress.lastPageByAudience[progress.audience];
    const preferredPageIndex = preferredPageId ? pageIndexById.get(preferredPageId) : undefined;
    const firstAudiencePage = pages.findIndex((page) => page.audiences.includes(progress.audience));
    setAudience(progress.audience);
    setVisitedPageIds(progress.visitedPageIds);
    setLastPageByAudience(progress.lastPageByAudience);
    setPageIndex(preferredPageIndex ?? Math.max(0, firstAudiencePage));
    setSearchQuery("");
    setLoadedLanguage(lang);
  }, [lang, pageIndexById, pages]);

  useEffect(() => {
    if (routePages.length === 0 || routePages.some(({ index }) => index === pageIndex)) return;
    const preferredPageId = lastPageByAudience[audience];
    const preferred = preferredPageId ? pageIndexById.get(preferredPageId) : undefined;
    const preferredIsInRoute = preferred !== undefined
      && routePages.some(({ index }) => index === preferred);
    setPageIndex(preferredIsInRoute ? preferred : routePages[0].index);
  }, [audience, lastPageByAudience, pageIndex, pageIndexById, routePages]);

  const page = pages[pageIndex];

  useEffect(() => {
    if (loadedLanguage !== lang || !page || !page.audiences.includes(audience)) return;
    setVisitedPageIds((current) => current.includes(page.id) ? current : [...current, page.id]);
    setLastPageByAudience((current) => current[audience] === page.id
      ? current
      : { ...current, [audience]: page.id });
  }, [audience, lang, loadedLanguage, page]);

  useEffect(() => {
    if (loadedLanguage !== lang) return;
    saveLearningProgress(window.localStorage, lang, {
      audience,
      visitedPageIds,
      lastPageByAudience,
    });
  }, [audience, lang, lastPageByAudience, loadedLanguage, visitedPageIds]);

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
            title={lang === "zh" ? "仓库内参考资料" : "Repository reference"}
          >
            {children}
          </span>
        );
      },
      h2: ({ children }) => <h2 id={learningHeadingId(reactNodeText(children))}>{children}</h2>,
      h3: ({ children }) => <h3 id={learningHeadingId(reactNodeText(children))}>{children}</h3>,
      h4: ({ children }) => <h4 id={learningHeadingId(reactNodeText(children))}>{children}</h4>,
    }),
    // Keep component identities stable across App health polling so an open
    // diagram portal is not unmounted and recreated.
    [lang],
  );

  if (!page || stages.length === 0 || routePages.length === 0) return null;
  const stageIndex = stages.findIndex((stage) => stage.id === page.stageId);
  const activeStage = stages[Math.max(0, stageIndex)] ?? stages[0];
  const routePageIndex = routePages.findIndex(({ index }) => index === pageIndex);
  const previousPage = routePages[routePageIndex - 1];
  const nextPage = routePages[routePageIndex + 1];
  const routePositionById = new Map(routePages.map(({ page: item }, index) => [item.id, index]));
  const visitedCount = routePages.filter(({ page: item }) => visitedPageIds.includes(item.id)).length;
  const progressPercent = Math.round((visitedCount / routePages.length) * 100);
  const audienceOptions = audienceDefinitions(t);

  return (
    <section className="overflow-hidden rounded-lg border border-[var(--theme-border)] bg-[var(--theme-card)]">
      <header className="border-b border-[var(--theme-border)] px-4 py-5 sm:px-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-[var(--theme-border)] bg-[var(--theme-card-strong)] text-[var(--theme-icon-accent-color)]">
              <BookOpenCheck className="h-5 w-5" />
            </span>
            <div>
              <p className="theme-kicker text-[10px] uppercase">
                {t("学习/维护", "Learning / Maintenance")}
              </p>
              <h2 className="mt-1 text-lg font-semibold text-[var(--theme-text-strong)]">
                {t("从使用到架构，分阶段理解 AI Agent", "Learn AI agents from everyday use to architecture")}
              </h2>
              <p className="mt-1 max-w-3xl text-sm leading-6 text-[var(--theme-text-muted)]">
                {t("选择适合你的路线，再从具体任务逐步进入 Agent、记忆、安全、技能和开发细节。阅读位置会保存在当前浏览器。", "Choose the route that fits you, then move from practical tasks into the agent, memory, safety, capabilities, and development details. Your reading position is saved in this browser.")}
              </p>
              <div className="mt-4 inline-flex max-w-full gap-1 overflow-x-auto rounded-md border border-[var(--theme-border-strong)] bg-[var(--theme-card-strong)] p-1.5 shadow-sm">
                {audienceOptions.map((option) => {
                  const AudienceIcon = option.icon;
                  return (
                    <button
                      key={option.id}
                      type="button"
                      className={`flex shrink-0 items-center gap-2 rounded border px-3.5 py-2 text-sm font-semibold transition ${
                        audience === option.id
                          ? "border-orange-300/45 bg-orange-400/15 text-[var(--theme-text-strong)] shadow-sm"
                          : "border-transparent text-[var(--theme-text-muted)] hover:border-[var(--theme-border)] hover:bg-[var(--theme-card)] hover:text-[var(--theme-text-strong)]"
                      }`}
                      title={option.description}
                      aria-pressed={audience === option.id}
                      onClick={() => {
                        setAudience(option.id);
                        setSearchQuery("");
                      }}
                    >
                      <AudienceIcon className={`h-4 w-4 ${audience === option.id ? "text-orange-300" : ""}`} />
                      {option.title}
                    </button>
                  );
                })}
              </div>
              <p className="mt-2 text-xs text-[var(--theme-text-faint)]">
                {audienceOptions.find((option) => option.id === audience)?.description}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="theme-topbar-btn !px-2.5 disabled:opacity-35"
              disabled={!previousPage}
              title={t("上一页", "Previous page")}
              aria-label={t("上一页", "Previous page")}
              onClick={() => previousPage && setPageIndex(previousPage.index)}
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <span className="min-w-16 text-center font-mono text-xs text-[var(--theme-text-muted)]">
              {routePageIndex + 1} / {routePages.length}
            </span>
            <button
              type="button"
              className="theme-topbar-btn !px-2.5 disabled:opacity-35"
              disabled={!nextPage}
              title={t("下一页", "Next page")}
              aria-label={t("下一页", "Next page")}
              onClick={() => nextPage && setPageIndex(nextPage.index)}
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
        <div
          className="grid overflow-hidden rounded-md border border-[var(--theme-border)]"
          style={{
            minWidth: `${Math.max(1, stages.length) * 130}px`,
            gridTemplateColumns: `repeat(${Math.max(1, stages.length)}, minmax(0, 1fr))`,
          }}
        >
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
          <div className="relative mb-3">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--theme-text-faint)]" />
            <input
              type="search"
              className="theme-input w-full !pl-9 !pr-9"
              value={searchQuery}
              placeholder={t("搜索主题或关键词", "Search topics or keywords")}
              aria-label={t("搜索学习内容", "Search learning content")}
              onChange={(event) => setSearchQuery(event.target.value)}
            />
            {searchQuery && (
              <button
                type="button"
                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-[var(--theme-text-faint)] hover:text-[var(--theme-text-strong)]"
                title={t("清除搜索", "Clear search")}
                aria-label={t("清除搜索", "Clear search")}
                onClick={() => setSearchQuery("")}
              >
                <X className="h-4 w-4" />
              </button>
            )}
          </div>
          <select
            id="ai-learning-page"
            className="theme-input w-full lg:hidden"
            value={pageIndex}
            onChange={(event) => setPageIndex(Number(event.target.value))}
          >
            {searchQuery.trim() ? searchResults.map(({ index, page: item }) => (
              <option key={item.id} value={index}>
                {(routePositionById.get(item.id) ?? 0) + 1}. {item.title}
              </option>
            )) : stages.map((stage) => (
              <optgroup key={stage.id} label={`${stage.level} · ${stage.title}`}>
                {stage.pages.map(({ index, page: item }) => (
                  <option key={item.id} value={index}>
                    {(routePositionById.get(item.id) ?? 0) + 1}. {item.title}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          {searchQuery.trim() && searchResults.length === 0 && (
            <p className="mt-2 px-2 text-xs text-[var(--theme-text-faint)] lg:hidden">
              {t("没有找到相关内容，请尝试更短的关键词。", "No matching content. Try a shorter keyword.")}
            </p>
          )}
          <nav
            className="theme-scrollbar hidden max-h-[calc(100vh-10rem)] space-y-2 overflow-y-auto pr-1 lg:block"
            aria-label={t("学习主题", "Learning topics")}
          >
            {searchQuery.trim() ? (
              <div className="space-y-1">
                <p className="px-2 pb-1 text-[10px] text-[var(--theme-text-faint)]">
                  {searchResults.length > 0
                    ? t(`找到 ${searchResults.length} 项`, `${searchResults.length} results`)
                    : t("没有找到相关内容", "No matching content")}
                </p>
                {searchResults.map(({ index, page: item }) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`flex w-full items-start gap-2 rounded-md px-2 py-2 text-left text-xs leading-5 transition ${
                      index === pageIndex
                        ? "bg-orange-400/12 font-medium text-[var(--theme-text-strong)]"
                        : "text-[var(--theme-text-muted)] hover:bg-white/5 hover:text-[var(--theme-text-strong)]"
                    }`}
                    onClick={() => setPageIndex(index)}
                  >
                    <Search className="mt-0.5 h-3.5 w-3.5 shrink-0 text-[var(--theme-text-faint)]" />
                    <span>
                      <span className="block">{item.title}</span>
                      <span className="block text-[10px] text-[var(--theme-text-faint)]">{item.chapterTitle}</span>
                    </span>
                  </button>
                ))}
              </div>
            ) : stages.map((stage, index) => {
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
                                  {String((routePositionById.get(item.id) ?? 0) + 1).padStart(2, "0")}
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
              <span>{t("内容", "Page")} {routePageIndex + 1} / {routePages.length}</span>
              <span aria-hidden="true">/</span>
              <span>{t(`约 ${page.estimatedMinutes} 分钟`, `${page.estimatedMinutes} min read`)}</span>
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
            <p className="mt-2 text-xs text-[var(--theme-text-faint)]">
              {t(`这条路线已读 ${visitedCount} / ${routePages.length} 项`, `${visitedCount} of ${routePages.length} read in this route`)}
            </p>
            <div className="mt-4 h-1 overflow-hidden rounded-full bg-white/8" aria-hidden="true">
              <div
                className="h-full bg-[var(--theme-icon-accent-color)] transition-[width]"
                style={{ width: `${progressPercent}%` }}
              />
            </div>
          </div>
          {page.headings.length > 1 && (
            <nav className="mx-auto mb-7 max-w-4xl border-l-2 border-[var(--theme-border)] pl-4" aria-label={t("本页内容", "On this page")}>
              <p className="text-xs font-medium text-[var(--theme-text-strong)]">{t("本页内容", "On this page")}</p>
              <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1.5">
                {page.headings.map((heading) => (
                  <a
                    key={`${heading.level}-${heading.id}`}
                    href={`#${heading.id}`}
                    className={`text-xs text-[var(--theme-text-muted)] hover:text-[var(--theme-text-strong)] ${heading.level > 2 ? "before:mr-1 before:content-['·']" : "font-medium"}`}
                  >
                    {heading.title}
                  </a>
                ))}
              </div>
            </nav>
          )}
          <article className="learning-markdown mx-auto max-w-4xl">
            <ReactMarkdown components={markdownComponents}>{page.markdown}</ReactMarkdown>
          </article>
          <footer className="mt-10 flex items-center justify-between gap-3 border-t border-[var(--theme-border)] pt-5">
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={!previousPage}
              onClick={() => previousPage && setPageIndex(previousPage.index)}
            >
              <ChevronLeft className="h-4 w-4" />
              <span className="min-w-0 text-left">
                <span className="block text-[10px] text-[var(--theme-text-faint)]">{t("上一页", "Previous")}</span>
                <span className="block max-w-[34vw] truncate text-xs sm:max-w-56">{previousPage?.page.title ?? t("已到开头", "Start")}</span>
              </span>
            </button>
            <button
              type="button"
              className="theme-secondary-btn !px-3 disabled:opacity-35"
              disabled={!nextPage}
              onClick={() => nextPage && setPageIndex(nextPage.index)}
            >
              <span className="min-w-0 text-right">
                <span className="block text-[10px] text-[var(--theme-text-faint)]">{t("下一页", "Next")}</span>
                <span className="block max-w-[34vw] truncate text-xs sm:max-w-56">{nextPage?.page.title ?? t("已完成", "Complete")}</span>
              </span>
              <ChevronRight className="h-4 w-4" />
            </button>
          </footer>
        </main>
      </div>
    </section>
  );
}
