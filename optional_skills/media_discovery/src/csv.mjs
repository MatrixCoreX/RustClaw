import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";

export const VIDEO_COLUMNS = [
  "sequence",
  "global_sequence",
  "platform",
  "browser_mode",
  "source_mode",
  "search_keyword",
  "discovery_source_url",
  "title",
  "platform_text",
  "recognized_text",
  "cover_screenshot_path",
  "video_page_url",
  "discovered_at",
];

export const IMAGE_COLUMNS = [
  "sequence",
  "global_sequence",
  "post_sequence",
  "image_sequence",
  "platform",
  "browser_mode",
  "source_mode",
  "search_keyword",
  "discovery_source_url",
  "title",
  "platform_text",
  "recognized_text",
  "image_url",
  "source_page_url",
  "discovered_at",
];

function spreadsheetSafe(value) {
  const text = value == null ? "" : String(value);
  return /^[\t\r\n ]*[=+\-@]/u.test(text) ? `'${text}` : text;
}

export function csvCell(value) {
  return `"${spreadsheetSafe(value).replaceAll('"', '""')}"`;
}

export function renderCsv(columns, records) {
  const lines = [columns.map(csvCell).join(",")];
  for (const record of records) {
    lines.push(columns.map((column) => csvCell(record[column])).join(","));
  }
  return `\uFEFF${lines.join("\r\n")}\r\n`;
}

export async function writeAtomic(filePath, content) {
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  const temporary = `${filePath}.tmp-${process.pid}-${randomUUID()}`;
  const handle = await fs.open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(content, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
  await fs.rename(temporary, filePath);
}

export async function exportRecordCsv(root, records) {
  const exportRoot = path.join(root, "exports");
  const videos = records
    .filter((record) => record.kind === "video")
    .sort((left, right) => left.sequence - right.sequence);
  const images = records
    .filter((record) => record.kind === "image")
    .sort((left, right) => left.sequence - right.sequence);
  const videoPath = path.join(exportRoot, "videos.csv");
  const imagePath = path.join(exportRoot, "images.csv");
  await writeAtomic(videoPath, renderCsv(VIDEO_COLUMNS, videos));
  await writeAtomic(imagePath, renderCsv(IMAGE_COLUMNS, images));
  return { videoPath, imagePath, videoCount: videos.length, imageCount: images.length };
}
