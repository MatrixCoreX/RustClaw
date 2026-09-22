import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import type { AippTaskActivityArtifact } from "../types/api";
import type { ArtifactFetch } from "../lib/task-artifact-content";
import { ActivityVideoPlayer } from "./AippTaskActivityMedia";

export function AippVideoViewer({ artifact, apiFetch, t, onClose }: {
  artifact: AippTaskActivityArtifact;
  apiFetch: ArtifactFetch;
  t: (zh: string, en: string) => string;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    const previousOverflow = document.body.style.overflow;
    dialog?.showModal();
    document.body.style.overflow = "hidden";
    return () => {
      dialog?.close();
      document.body.style.overflow = previousOverflow;
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);
  return createPortal(
    <dialog ref={dialogRef} tabIndex={-1} aria-labelledby={titleId}
      className="theme-dialog-panel m-auto h-[min(90dvh,900px)] max-h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-6xl overflow-hidden rounded-lg p-0 text-[var(--theme-text-strong)] backdrop:bg-black/70"
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose();
      }}>
      <div className="flex h-full min-h-0 flex-col">
        <header className="flex shrink-0 items-center gap-3 border-b border-[var(--theme-border)] px-3 py-2 sm:px-4">
          <h2 id={titleId} className="min-w-0 flex-1 truncate text-sm font-medium" title={artifact.filename}>{artifact.filename}</h2>
          <button type="button" autoFocus className="theme-icon-btn h-9 w-9 shrink-0" onClick={onClose} title={t("关闭", "Close")} aria-label={t("关闭", "Close")}><X className="h-5 w-5" /></button>
        </header>
        <div className="min-h-0 flex-1 overflow-hidden p-2 sm:p-4">
          <ActivityVideoPlayer key={artifact.id} artifact={artifact} apiFetch={apiFetch} t={t} />
        </div>
      </div>
    </dialog>, document.body,
  );
}
