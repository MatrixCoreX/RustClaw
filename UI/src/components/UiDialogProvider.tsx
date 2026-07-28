import {
  createContext,
  type FormEvent,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Info, X } from "lucide-react";

export interface UiConfirmOptions {
  title?: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
}

export interface UiPromptOptions extends UiConfirmOptions {
  initialValue?: string;
  inputLabel?: string;
  placeholder?: string;
}

export interface UiChoiceOption {
  value: string;
  label: string;
  description?: string;
}

export interface UiChoiceOptions extends Omit<UiConfirmOptions, "confirmLabel"> {
  choices: readonly UiChoiceOption[];
}

interface UiDialogContextValue {
  confirm: (options: UiConfirmOptions) => Promise<boolean>;
  prompt: (options: UiPromptOptions) => Promise<string | null>;
  choose: (options: UiChoiceOptions) => Promise<string | null>;
}

type DialogRequest =
  | ({ kind: "confirm"; resolve: (value: boolean) => void } & UiConfirmOptions)
  | ({ kind: "prompt"; resolve: (value: string | null) => void } & UiPromptOptions)
  | ({ kind: "choice"; resolve: (value: string | null) => void } & UiChoiceOptions);

const UiDialogContext = createContext<UiDialogContextValue | null>(null);

function browserCopy(zh: string, en: string): string {
  return document.documentElement.lang.toLowerCase().startsWith("zh") ? zh : en;
}

export function UiDialogProvider({ children }: { children: ReactNode }) {
  const [active, setActive] = useState<DialogRequest | null>(null);
  const [promptValue, setPromptValue] = useState("");
  const activeRef = useRef<DialogRequest | null>(null);
  const queueRef = useRef<DialogRequest[]>([]);
  const primaryButtonRef = useRef<HTMLButtonElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const activate = useCallback((request: DialogRequest | null) => {
    activeRef.current = request;
    setActive(request);
    setPromptValue(request?.kind === "prompt" ? request.initialValue ?? "" : "");
  }, []);

  const enqueue = useCallback((request: DialogRequest) => {
    if (activeRef.current) {
      queueRef.current.push(request);
    } else {
      activate(request);
    }
  }, [activate]);

  const finish = useCallback((value: boolean | string | null) => {
    const request = activeRef.current;
    if (!request) return;
    if (request.kind === "confirm") {
      request.resolve(value === true);
    } else {
      request.resolve(typeof value === "string" ? value : null);
    }
    activate(queueRef.current.shift() ?? null);
  }, [activate]);

  const confirm = useCallback((options: UiConfirmOptions) => new Promise<boolean>((resolve) => {
    enqueue({ ...options, kind: "confirm", resolve });
  }), [enqueue]);

  const prompt = useCallback((options: UiPromptOptions) => new Promise<string | null>((resolve) => {
    enqueue({ ...options, kind: "prompt", resolve });
  }), [enqueue]);

  const choose = useCallback((options: UiChoiceOptions) => new Promise<string | null>((resolve) => {
    enqueue({ ...options, kind: "choice", resolve });
  }), [enqueue]);

  useEffect(() => {
    if (!active) return;
    const focusTarget = active.kind === "prompt" ? inputRef.current : primaryButtonRef.current;
    const frame = window.requestAnimationFrame(() => focusTarget?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") finish(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [active, finish]);

  useEffect(() => () => {
    const pending = [activeRef.current, ...queueRef.current].filter(Boolean) as DialogRequest[];
    for (const request of pending) {
      if (request.kind === "confirm") request.resolve(false);
      else request.resolve(null);
    }
    queueRef.current = [];
    activeRef.current = null;
  }, []);

  const submitDialog = (event: FormEvent) => {
    event.preventDefault();
    finish(activeRef.current?.kind === "confirm" ? true : promptValue);
  };

  const dialog = active ? createPortal(
    <div
      className="fixed inset-0 z-[120] flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) finish(null);
      }}
    >
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="ui-dialog-title"
        aria-describedby="ui-dialog-message"
        className="theme-card w-full max-w-md border p-0 shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-white/10 px-5 py-4">
          <span className={active.tone === "danger" ? "mt-0.5 text-red-300" : "mt-0.5 text-sky-300"}>
            {active.tone === "danger" ? <AlertTriangle className="h-5 w-5" /> : <Info className="h-5 w-5" />}
          </span>
          <div className="min-w-0 flex-1">
            <h2 id="ui-dialog-title" className="text-base font-semibold text-white">
              {active.title ?? browserCopy("请确认", "Confirm action")}
            </h2>
            <p id="ui-dialog-message" className="mt-2 whitespace-pre-wrap text-sm leading-6 text-white/65">
              {active.message}
            </p>
          </div>
          <button
            type="button"
            onClick={() => finish(null)}
            className="theme-icon-btn h-8 w-8 shrink-0"
            title={browserCopy("取消", "Cancel")}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {active.kind === "choice" ? (
          <div>
            <div className="grid gap-3 px-5 pt-4">
              {active.choices.map((choice, index) => (
                <button
                  key={choice.value}
                  ref={index === 0 ? primaryButtonRef : undefined}
                  type="button"
                  onClick={() => finish(choice.value)}
                  className="theme-secondary-btn min-h-16 w-full items-start px-4 py-3 text-left"
                >
                  <span className="block text-sm font-semibold text-white">{choice.label}</span>
                  {choice.description ? (
                    <span className="mt-1 block text-xs font-normal leading-5 text-white/60">
                      {choice.description}
                    </span>
                  ) : null}
                </button>
              ))}
            </div>
            <div className="flex justify-end px-5 py-4">
              <button type="button" onClick={() => finish(null)} className="theme-secondary-btn px-4 py-2 text-sm">
                {active.cancelLabel ?? browserCopy("取消", "Cancel")}
              </button>
            </div>
          </div>
        ) : (
          <form onSubmit={submitDialog}>
            {active.kind === "prompt" ? (
              <div className="px-5 pt-4">
                <label className="block text-sm font-medium text-white/80" htmlFor="ui-dialog-input">
                  {active.inputLabel ?? browserCopy("输入内容", "Enter a value")}
                </label>
                <input
                  ref={inputRef}
                  id="ui-dialog-input"
                  value={promptValue}
                  onChange={(event) => setPromptValue(event.target.value)}
                  placeholder={active.placeholder}
                  className="theme-input mt-2 w-full"
                />
              </div>
            ) : null}

            <div className="flex flex-wrap justify-end gap-2 px-5 py-4">
              <button type="button" onClick={() => finish(null)} className="theme-secondary-btn px-4 py-2 text-sm">
                {active.cancelLabel ?? browserCopy("取消", "Cancel")}
              </button>
              <button
                ref={primaryButtonRef}
                type="submit"
                className={active.tone === "danger"
                  ? "inline-flex min-h-10 items-center justify-center rounded-md border border-red-400/35 bg-red-500/15 px-4 py-2 text-sm font-medium text-red-100 transition hover:bg-red-500/25"
                  : "theme-accent-btn px-4 py-2 text-sm"}
              >
                {active.confirmLabel ?? browserCopy("确认", "Confirm")}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>,
    document.body,
  ) : null;

  return (
    <UiDialogContext.Provider value={{ confirm, prompt, choose }}>
      {children}
      {dialog}
    </UiDialogContext.Provider>
  );
}

export function useUiDialog(): UiDialogContextValue {
  const context = useContext(UiDialogContext);
  if (!context) throw new Error("useUiDialog must be used inside UiDialogProvider");
  return context;
}
