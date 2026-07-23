export type MermaidTheme = "dark" | "neutral";

export interface DiagramSize {
  width: number;
  height: number;
}

export interface MermaidRenderResult {
  svg: string;
  bindFunctions?: (element: Element) => void;
}

let renderQueue: Promise<void> = Promise.resolve();
const MERMAID_RENDER_TIMEOUT_MS = 60_000;

export function fitDiagramScale(
  viewportWidth: number,
  diagramWidth: number,
  minScale = 0.1,
  maxScale = 1,
): number {
  if (!Number.isFinite(viewportWidth) || !Number.isFinite(diagramWidth) || diagramWidth <= 0) {
    return 1;
  }
  return Math.min(maxScale, Math.max(minScale, viewportWidth / diagramWidth));
}

export function scaledDiagramSize(size: DiagramSize, scale: number): DiagramSize {
  const safeScale = Number.isFinite(scale) && scale > 0 ? scale : 1;
  return {
    width: Math.max(1, Math.ceil(size.width * safeScale)),
    height: Math.max(1, Math.ceil(size.height * safeScale)),
  };
}

export function readDiagramSize(svgMarkup: string): DiagramSize | null {
  const template = document.createElement("template");
  template.innerHTML = svgMarkup.trim();
  const svg = template.content.querySelector("svg");
  if (!svg) return null;
  const viewBox = svg.getAttribute("viewBox")?.trim().split(/[\s,]+/).map(Number);
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2] > 0 && viewBox[3] > 0) {
    return { width: viewBox[2], height: viewBox[3] };
  }

  const width = Number.parseFloat(svg.getAttribute("width") ?? "");
  const height = Number.parseFloat(svg.getAttribute("height") ?? "");
  return Number.isFinite(width) && width > 0 && Number.isFinite(height) && height > 0
    ? { width, height }
    : null;
}

export function renderMermaid(
  id: string,
  source: string,
  theme: MermaidTheme,
): Promise<MermaidRenderResult> {
  const render = async () => {
    const { default: mermaid } = await import("mermaid");
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme,
      flowchart: { htmlLabels: true, curve: "basis", useMaxWidth: false },
    });
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([
        mermaid.render(id, source),
        new Promise<never>((_, reject) => {
          timeoutId = setTimeout(
            () => reject(new Error("mermaid_render_timeout")),
            MERMAID_RENDER_TIMEOUT_MS,
          );
        }),
      ]);
    } finally {
      if (timeoutId) clearTimeout(timeoutId);
    }
  };
  const result = renderQueue.then(render, render);
  renderQueue = result.then(() => undefined, () => undefined);
  return result;
}
