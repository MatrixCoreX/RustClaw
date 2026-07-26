export interface AiLearningPage {
  id: string;
  title: string;
  chapterId: string;
  chapterTitle: string;
  stageId: string;
  kind: "chapter" | "section";
  markdown: string;
  diagramCount: number;
}

export type LearningLinkKind = "external" | "internal" | "reference";

export interface StandaloneLearningDocument {
  id: string;
  chapterId: string;
  chapterTitle: string;
  stageId?: string;
  markdown: string;
}

interface Heading {
  level: 2 | 3;
  title: string;
  start: number;
  stageId: string;
}

const DEFAULT_STAGE_ID = "general";
const LEARNING_STAGE_PATTERN =
  /^<!--\s*ai-learning-stage:\s*([a-z0-9_-]+)\s*-->\s*$/i;

function withoutLearningExcludedBlocks(markdown: string): string {
  return markdown.replace(
    /<!-- ai-learning-exclude:start -->[\s\S]*?<!-- ai-learning-exclude:end -->\s*/g,
    "",
  );
}

function pageId(title: string, index: number): string {
  const token = title
    .toLowerCase()
    .replace(/[`*_]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return token || `section-${index + 1}`;
}

function pageMetrics(markdown: string): Pick<AiLearningPage, "diagramCount"> {
  return {
    diagramCount: (markdown.match(/^```mermaid\s*$/gm) ?? []).length,
  };
}

function withoutLearningMetadata(markdown: string): string {
  return markdown
    .replace(/<!--\s*ai-learning-stage:\s*[a-z0-9_-]+\s*-->\s*/gi, "")
    .trim();
}

export function parseStandaloneLearningDocument(
  document: StandaloneLearningDocument,
): AiLearningPage {
  const normalized = document.markdown.replace(/\r\n/g, "\n").trim();
  const markdown = normalized
    .replace(
      /<!-- ai-learning-navigation:start -->[\s\S]*?<!-- ai-learning-navigation:end -->\s*/g,
      "",
    )
    .trim();
  const titleMatch = /^#\s+(.+?)\s*$/m.exec(normalized);
  const title = cleanTitle(titleMatch?.[1] ?? document.id);
  const content = withoutLearningMetadata(markdown);

  return {
    id: document.id,
    title,
    chapterId: document.chapterId,
    chapterTitle: document.chapterTitle,
    stageId: document.stageId ?? DEFAULT_STAGE_ID,
    kind: "section",
    markdown: content,
    ...pageMetrics(content),
  };
}

function markdownHeadings(lines: string[]): Heading[] {
  const headings: Heading[] = [];
  let fence: "```" | "~~~" | null = null;
  let stageId = DEFAULT_STAGE_ID;

  lines.forEach((line, index) => {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
      const marker = trimmed.slice(0, 3) as "```" | "~~~";
      fence = fence === marker ? null : fence ?? marker;
      return;
    }
    if (fence) return;
    const stageMatch = LEARNING_STAGE_PATTERN.exec(trimmed);
    if (stageMatch) {
      stageId = stageMatch[1].toLowerCase();
      return;
    }
    const match = /^(##|###)\s+(.+?)\s*$/.exec(line);
    if (match) {
      headings.push({
        level: match[1].length as 2 | 3,
        title: match[2],
        start: index,
        stageId,
      });
    }
  });

  return headings;
}

function cleanTitle(title: string): string {
  return title.replace(/[`*_]/g, "");
}

function pageMarkdown(lines: string[], start: number, end: number): string {
  return withoutLearningMetadata(lines.slice(start, end).join("\n"));
}

function hasChapterIntroduction(lines: string[], chapterStart: number, firstSectionStart: number): boolean {
  return lines
    .slice(chapterStart + 1, firstSectionStart)
    .some((line) => line.trim().length > 0);
}

export function classifyLearningLink(href?: string): LearningLinkKind {
  const value = href?.trim();
  if (!value) return "reference";
  if (value.startsWith("#")) return "internal";

  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:" ? "external" : "reference";
  } catch {
    return "reference";
  }
}

export function parseReadmeLearningPages(markdown: string): AiLearningPage[] {
  const lines = withoutLearningExcludedBlocks(markdown)
    .replace(/\r\n/g, "\n")
    .split("\n");
  const headings = markdownHeadings(lines);
  const chapters = headings.filter((heading) => heading.level === 2);

  if (chapters.length === 0) {
    const content = lines.join("\n").trim();
    return content
      ? [{
          id: "readme",
          title: "README",
          chapterId: "readme",
          chapterTitle: "README",
          stageId: DEFAULT_STAGE_ID,
          kind: "chapter",
          markdown: withoutLearningMetadata(content),
          ...pageMetrics(withoutLearningMetadata(content)),
        }]
      : [];
  }

  return chapters.flatMap((chapter, chapterIndex) => {
    const chapterEnd = chapters[chapterIndex + 1]?.start ?? lines.length;
    const chapterId = pageId(chapter.title, chapterIndex);
    const chapterTitle = cleanTitle(chapter.title);
    const sections = headings.filter(
      (heading) => heading.level === 3
        && heading.start > chapter.start
        && heading.start < chapterEnd,
    );

    if (sections.length === 0) {
      const content = pageMarkdown(lines, chapter.start, chapterEnd);
      return [{
        id: chapterId,
        title: chapterTitle,
        chapterId,
        chapterTitle,
        stageId: chapter.stageId,
        kind: "chapter" as const,
        markdown: content,
        ...pageMetrics(content),
      }];
    }

    const pages: AiLearningPage[] = [];
    if (hasChapterIntroduction(lines, chapter.start, sections[0].start)) {
      const content = pageMarkdown(lines, chapter.start, sections[0].start);
      pages.push({
        id: chapterId,
        title: chapterTitle,
        chapterId,
        chapterTitle,
        stageId: chapter.stageId,
        kind: "chapter",
        markdown: content,
        ...pageMetrics(content),
      });
    }

    sections.forEach((section, sectionIndex) => {
      const sectionEnd = sections[sectionIndex + 1]?.start ?? chapterEnd;
      const sectionBody = pageMarkdown(lines, section.start, sectionEnd);
      const content = `## ${chapter.title}\n\n${sectionBody}`;
      pages.push({
        id: `${chapterId}--${pageId(section.title, sectionIndex)}`,
        title: cleanTitle(section.title),
        chapterId,
        chapterTitle,
        stageId: section.stageId,
        kind: "section",
        markdown: content,
        ...pageMetrics(content),
      });
    });

    return pages;
  });
}

export function orderLearningPagesByStage(
  pages: AiLearningPage[],
  stageOrder: string[],
): AiLearningPage[] {
  const rank = new Map(stageOrder.map((stageId, index) => [stageId, index]));
  const fallbackRank = stageOrder.length;

  return pages
    .map((page, sourceIndex) => ({ page, sourceIndex }))
    .sort((left, right) => {
      const stageDifference =
        (rank.get(left.page.stageId) ?? fallbackRank)
        - (rank.get(right.page.stageId) ?? fallbackRank);
      return stageDifference || left.sourceIndex - right.sourceIndex;
    })
    .map(({ page }) => page);
}
