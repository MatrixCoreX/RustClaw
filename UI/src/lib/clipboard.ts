type ClipboardDocument = {
  body?: {
    appendChild: (node: unknown) => void;
    removeChild: (node: unknown) => void;
  };
  createElement?: (tag: string) => {
    value: string;
    setAttribute: (name: string, value: string) => void;
    style: Record<string, string>;
    focus: () => void;
    select: () => void;
  };
  execCommand?: (command: string) => boolean;
};

export interface ClipboardEnvironment {
  clipboard?: {
    writeText?: (text: string) => Promise<void>;
  };
  document?: ClipboardDocument;
}

export class ClipboardWriteError extends Error {
  readonly code = "clipboard_write_failed";

  constructor() {
    super("clipboard_write_failed");
    this.name = "ClipboardWriteError";
  }
}

export async function writeTextToClipboard(
  value: string,
  env: ClipboardEnvironment = {
    clipboard: globalThis.navigator?.clipboard,
    document: typeof document === "undefined" ? undefined : (document as unknown as ClipboardDocument),
  },
): Promise<void> {
  if (env.clipboard?.writeText) {
    try {
      await env.clipboard.writeText(value);
      return;
    } catch {
      // Some browsers expose the API but reject it outside a permitted context.
    }
  }

  const doc = env.document;
  if (!doc?.body || !doc.createElement || typeof doc.execCommand !== "function") {
    throw new ClipboardWriteError();
  }

  const textarea = doc.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.top = "0";
  textarea.style.left = "-9999px";
  textarea.style.opacity = "0";

  doc.body.appendChild(textarea);
  try {
    textarea.focus();
    textarea.select();
    if (!doc.execCommand("copy")) {
      throw new ClipboardWriteError();
    }
  } finally {
    doc.body.removeChild(textarea);
  }
}
