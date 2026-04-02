function parseDateInput(raw: string): Date | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  if (/^\d{10}$/.test(trimmed)) {
    const date = new Date(Number(trimmed) * 1000);
    return Number.isNaN(date.getTime()) ? null : date;
  }

  if (/^\d{13}$/.test(trimmed)) {
    const date = new Date(Number(trimmed));
    return Number.isNaN(date.getTime()) ? null : date;
  }

  const date = new Date(trimmed);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatDateOnlyHuman(raw: string | null | undefined, locale: string): string {
  if (!raw) return "--";
  const date = parseDateInput(raw);
  if (!date) return raw;

  const parts = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);

  const year = parts.find((part) => part.type === "year")?.value;
  const month = parts.find((part) => part.type === "month")?.value;
  const day = parts.find((part) => part.type === "day")?.value;
  if (!year || !month || !day) return raw;
  return `${year}-${month}-${day}`;
}
