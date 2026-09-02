import { ArrowLeftRight, Check, Copy, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { writeTextToClipboard } from "../lib/clipboard";
import { nniPublicKeyFormats, shortenHex } from "../lib/nni-display";

type Translate = (zh: string, en: string) => string;
type PublicKeyFormat = "compact" | "raw";
type CopyState = "idle" | "copied" | "error";

export interface NniPublicKeyDisplayProps {
  value?: string | null;
  t: Translate;
  className?: string;
  valueClassName?: string;
  shorten?: { head: number; tail: number };
  showByteSize?: boolean;
  allowFormatSwitch?: boolean;
  copyButton?: "compact" | "labeled";
  onCopy?: (value: string) => void;
}

export function NniPublicKeyDisplay({
  value,
  t,
  className = "",
  valueClassName = "",
  shorten,
  showByteSize = false,
  allowFormatSwitch = true,
  copyButton,
  onCopy,
}: NniPublicKeyDisplayProps) {
  const formats = useMemo(() => nniPublicKeyFormats(value), [value]);
  const [format, setFormat] = useState<PublicKeyFormat>("compact");
  const [copyState, setCopyState] = useState<CopyState>("idle");

  useEffect(() => {
    setFormat("compact");
    setCopyState("idle");
  }, [value]);

  const displayedValue = formats?.[format] ?? value?.trim() ?? "--";
  const visibleValue = shorten
    ? shortenHex(displayedValue, shorten.head, shorten.tail)
    : displayedValue;
  const targetFormat: PublicKeyFormat = format === "compact" ? "raw" : "compact";
  const targetLabel = targetFormat === "raw" ? t("原始", "Raw") : "Base58";
  const targetTitle = targetFormat === "raw"
    ? t("切换为原始十六进制公钥", "Show the raw hexadecimal public key")
    : t("切换为 Base58 编码公钥", "Show the Base58-encoded public key");
  const copyMode = copyButton ?? (onCopy ? "labeled" : null);
  const copyLabel = copyState === "copied"
    ? t("已复制", "Copied")
    : copyState === "error"
      ? t("复制失败", "Copy failed")
      : t("复制", "Copy");
  const copyTitle = copyState === "copied"
    ? t("已复制完整公钥", "Full public key copied")
    : copyState === "error"
      ? t("复制失败，请手动选择公钥复制", "Copy failed. Select and copy the public key manually")
      : t("复制完整公钥", "Copy full public key");
  const CopyIcon = copyState === "copied" ? Check : copyState === "error" ? TriangleAlert : Copy;

  const copyPublicKey = async () => {
    if (onCopy) {
      onCopy(displayedValue);
      return;
    }
    try {
      await writeTextToClipboard(displayedValue);
      setCopyState("copied");
    } catch {
      setCopyState("error");
    }
  };

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
          {allowFormatSwitch ? (
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
          ) : null}
        </>
      ) : null}
      {copyMode ? (
        <button
          type="button"
          onClick={() => void copyPublicKey()}
          title={copyTitle}
          aria-label={copyTitle}
          aria-live="polite"
          className={copyMode === "compact"
            ? "theme-icon-btn h-7 w-7 shrink-0 p-0"
            : "inline-flex shrink-0 items-center gap-1 rounded-lg border border-white/10 bg-white/5 px-2 py-1 text-[11px] font-medium text-white/60 transition hover:border-white/20 hover:bg-white/10 hover:text-white"}
        >
          <CopyIcon className="h-3 w-3" />
          {copyMode === "labeled" ? copyLabel : <span className="sr-only">{copyLabel}</span>}
        </button>
      ) : null}
    </div>
  );
}
