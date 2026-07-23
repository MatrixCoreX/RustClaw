export interface AiLearningPage {
  id: string;
  title: string;
  markdown: string;
  diagramCount: number;
  subsectionCount: number;
}

function pageId(title: string, index: number): string {
  const token = title
    .toLowerCase()
    .replace(/[`*_]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return token || `section-${index + 1}`;
}

function pageMetrics(markdown: string): Pick<AiLearningPage, "diagramCount" | "subsectionCount"> {
  return {
    diagramCount: (markdown.match(/^```mermaid\s*$/gm) ?? []).length,
    subsectionCount: (markdown.match(/^###\s+.+$/gm) ?? []).length,
  };
}

export function parseReadmeLearningPages(markdown: string): AiLearningPage[] {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const sections: Array<{ title: string; start: number }> = [];
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
    if (match) sections.push({ title: match[1], start: index });
  });

  if (sections.length === 0) {
    const content = lines.join("\n").trim();
    return content
      ? [{ id: "readme", title: "README", markdown: content, ...pageMetrics(content) }]
      : [];
  }

  return sections.map((section, index) => {
    const start = index === 0 ? 0 : section.start;
    const end = sections[index + 1]?.start ?? lines.length;
    const content = lines.slice(start, end).join("\n").trim();
    return {
      id: pageId(section.title, index),
      title: section.title.replace(/[`*_]/g, ""),
      markdown: content,
      ...pageMetrics(content),
    };
  });
}
