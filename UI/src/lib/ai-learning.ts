export interface AiLearningPage {
  id: string;
  title: string;
  chapterId: string;
  chapterTitle: string;
  stageId: string;
  kind: "chapter" | "section";
  markdown: string;
  diagramCount: number;
  audiences: LearningAudience[];
  estimatedMinutes: number;
  headings: AiLearningHeading[];
}

export type LearningAudience = "beginner" | "operator" | "developer";

export const LEARNING_STAGE_ORDER = [
  "foundations",
  "agent-runtime",
  "context-memory",
  "safety-operations",
  "capabilities-artifacts",
  "development-release",
] as const;

export interface AiLearningHeading {
  id: string;
  title: string;
  level: 2 | 3 | 4;
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
  audiences: LearningAudience[];
}

const DEFAULT_STAGE_ID = "general";
const LEARNING_STAGE_PATTERN =
  /^<!--\s*ai-learning-stage:\s*([a-z0-9_-]+)\s*-->\s*$/im;
const LEARNING_AUDIENCE_PATTERN =
  /^<!--\s*ai-learning-audience:\s*([a-z, _-]+)\s*-->\s*$/im;
const ALL_AUDIENCES: LearningAudience[] = ["beginner", "operator", "developer"];

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

function defaultAudiences(stageId: string): LearningAudience[] {
  if (stageId === "foundations" || stageId === DEFAULT_STAGE_ID) return [...ALL_AUDIENCES];
  if (stageId === "development-release") return ["developer"];
  return ["operator", "developer"];
}

function parseAudiences(value: string, fallback: LearningAudience[]): LearningAudience[] {
  const parsed = value
    .split(",")
    .map((item) => item.trim().toLowerCase())
    .filter((item): item is LearningAudience => ALL_AUDIENCES.includes(item as LearningAudience));
  return parsed.length > 0 ? [...new Set(parsed)] : [...fallback];
}

export function learningHeadingId(title: string): string {
  return `learning-${pageId(title, 0)}`;
}

function contentHeadings(markdown: string): AiLearningHeading[] {
  const headings: AiLearningHeading[] = [];
  let fence: "```" | "~~~" | null = null;
  for (const line of markdown.split("\n")) {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
      const marker = trimmed.slice(0, 3) as "```" | "~~~";
      fence = fence === marker ? null : fence ?? marker;
      continue;
    }
    if (fence) continue;
    const match = /^(##|###|####)\s+(.+?)\s*$/.exec(line);
    if (!match) continue;
    const title = cleanTitle(match[2]);
    headings.push({ id: learningHeadingId(title), title, level: match[1].length as 2 | 3 | 4 });
  }
  return headings;
}

function estimatedReadingMinutes(markdown: string): number {
  const prose = markdown
    .replace(/```[\s\S]*?```|~~~[\s\S]*?~~~/g, " ")
    .replace(/<[^>]+>|[#>*_`|[\](){}-]/g, " ");
  const cjkCharacters = (prose.match(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/gu) ?? []).length;
  const latinWords = (prose.match(/[\p{L}\p{N}]+/gu) ?? [])
    .filter((word) => !/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(word))
    .length;
  return Math.max(1, Math.ceil(cjkCharacters / 400 + latinWords / 220));
}

function pageMetrics(
  markdown: string,
): Pick<AiLearningPage, "diagramCount" | "estimatedMinutes" | "headings"> {
  return {
    diagramCount: (markdown.match(/^```mermaid\s*$/gm) ?? []).length,
    estimatedMinutes: estimatedReadingMinutes(markdown),
    headings: contentHeadings(markdown),
  };
}

function withoutLearningMetadata(markdown: string): string {
  return markdown
    .replace(/<!--\s*ai-learning-stage:\s*[a-z0-9_-]+\s*-->\s*/gi, "")
    .replace(/<!--\s*ai-learning-audience:\s*[a-z, _-]+\s*-->\s*/gi, "")
    .trim();
}

function documentMetadata(markdown: string, fallbackStageId?: string) {
  const stageId = LEARNING_STAGE_PATTERN.exec(markdown)?.[1].toLowerCase()
    ?? fallbackStageId
    ?? DEFAULT_STAGE_ID;
  const audienceValue = LEARNING_AUDIENCE_PATTERN.exec(markdown)?.[1];
  return {
    stageId,
    audiences: audienceValue
      ? parseAudiences(audienceValue, defaultAudiences(stageId))
      : defaultAudiences(stageId),
  };
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
  const metadata = documentMetadata(normalized, document.stageId);

  return {
    id: document.id,
    title,
    chapterId: document.chapterId,
    chapterTitle: document.chapterTitle,
    stageId: metadata.stageId,
    audiences: metadata.audiences,
    kind: "section",
    markdown: content,
    ...pageMetrics(content),
  };
}

function standaloneSectionStarts(lines: string[]): Array<{ start: number; title: string }> {
  const sections: Array<{ start: number; title: string }> = [];
  let fence: "```" | "~~~" | null = null;
  lines.forEach((line, index) => {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
      const marker = trimmed.slice(0, 3) as "```" | "~~~";
      fence = fence === marker ? null : fence ?? marker;
      return;
    }
    if (fence) return;
    const match = /^##\s+(.+?)\s*$/.exec(line);
    if (match) sections.push({ start: index, title: cleanTitle(match[1]) });
  });
  return sections;
}

export function parseStandaloneLearningPages(
  document: StandaloneLearningDocument,
): AiLearningPage[] {
  const base = parseStandaloneLearningDocument(document);
  const lines = base.markdown.split("\n");
  const sections = standaloneSectionStarts(lines);
  if (sections.length <= 1) return [base];

  return sections.map((section, index) => {
    const start = index === 0 ? 0 : section.start;
    const end = sections[index + 1]?.start ?? lines.length;
    const markdown = lines.slice(start, end).join("\n").trim();
    return {
      ...base,
      id: `${base.id}--${pageId(section.title, index)}`,
      title: section.title,
      markdown,
      ...pageMetrics(markdown),
    };
  });
}

function markdownHeadings(lines: string[]): Heading[] {
  const headings: Heading[] = [];
  let fence: "```" | "~~~" | null = null;
  let stageId = DEFAULT_STAGE_ID;
  let audiences = defaultAudiences(stageId);

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
      audiences = defaultAudiences(stageId);
      return;
    }
    const audienceMatch = LEARNING_AUDIENCE_PATTERN.exec(trimmed);
    if (audienceMatch) {
      audiences = parseAudiences(audienceMatch[1], defaultAudiences(stageId));
      return;
    }
    const match = /^(##|###)\s+(.+?)\s*$/.exec(line);
    if (match) {
      headings.push({
        level: match[1].length as 2 | 3,
        title: match[2],
        start: index,
        stageId,
        audiences: [...audiences],
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
          audiences: defaultAudiences(DEFAULT_STAGE_ID),
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
        audiences: chapter.audiences,
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
        audiences: chapter.audiences,
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
        audiences: section.audiences,
        kind: "section",
        markdown: content,
        ...pageMetrics(content),
      });
    });

    return pages;
  });
}

export function searchLearningPages(
  pages: AiLearningPage[],
  query: string,
): AiLearningPage[] {
  const tokens = query
    .toLocaleLowerCase()
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (tokens.length === 0) return pages;
  return pages.filter((page) => {
    const searchable = `${page.title}\n${page.chapterTitle}\n${page.markdown}`
      .toLocaleLowerCase()
      .replace(/[`*_#[\](){}>|-]/g, " ");
    return tokens.every((token) => searchable.includes(token));
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
