import { ArrowLeftRight, Copy } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { nniPublicKeyFormats, shortenHex } from "../lib/nni-display";

type Translate = (zh: string, en: string) => string;
type PublicKeyFormat = "compact" | "raw";

export interface NniPublicKeyDisplayProps {
  value?: string | null;
  t: Translate;
  className?: string;
  valueClassName?: string;
  shorten?: { head: number; tail: number };
  showByteSize?: boolean;
  onCopy?: (value: string) => void;
}

export function NniPublicKeyDisplay({
  value,
  t,
  className = "",
  valueClassName = "",
  shorten,
  showByteSize = false,
  onCopy,
}: NniPublicKeyDisplayProps) {
  const formats = useMemo(() => nniPublicKeyFormats(value), [value]);
  const [format, setFormat] = useState<PublicKeyFormat>("compact");

  useEffect(() => setFormat("compact"), [value]);

  const displayedValue = formats?.[format] ?? value?.trim() ?? "--";
  const visibleValue = shorten
    ? shortenHex(displayedValue, shorten.head, shorten.tail)
    : displayedValue;
  const targetFormat: PublicKeyFormat = format === "compact" ? "raw" : "compact";
  const targetLabel = targetFormat === "raw" ? t("原始", "Raw") : t("紧凑", "Compact");
  const targetTitle = targetFormat === "raw"
    ? t("切换为原始十六进制公钥", "Show the raw hexadecimal public key")
    : t("切换为紧凑 Base58 公钥", "Show the compact Base58 public key");

  return (
    <div className={`flex min-w-0 flex-wrap items-center gap-2 ${className}`}>
      <code className={`min-w-0 break-all font-mono ${valueClassName}`} title={displayedValue}>
        {visibleValue}
      </code>
      {formats ? (
        <>
          {showByteSize ? (
            <span className="shrink-0 rounded-full border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/55">
              {format === "compact" ? 33 : 64} bytes
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => setFormat(targetFormat)}
            title={targetTitle}
            aria-label={targetTitle}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg border border-white/10 bg-white/5 px-2 py-1 text-[11px] font-medium text-white/60 transition hover:border-white/20 hover:bg-white/10 hover:text-white"
          >
            <ArrowLeftRight className="h-3 w-3" />
            {targetLabel}
          </button>
        </>
      ) : null}
      {onCopy ? (
        <button
          type="button"
          onClick={() => onCopy(displayedValue)}
          title={t("复制当前格式公钥", "Copy the public key in the current format")}
          aria-label={t("复制当前格式公钥", "Copy the public key in the current format")}
          className="inline-flex shrink-0 items-center gap-1 rounded-lg border border-white/10 bg-white/5 px-2 py-1 text-[11px] font-medium text-white/60 transition hover:border-white/20 hover:bg-white/10 hover:text-white"
        >
          <Copy className="h-3 w-3" />
          {t("复制", "Copy")}
        </button>
      ) : null}
    </div>
  );
}
