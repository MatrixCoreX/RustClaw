export async function copyAuthKeyValue(options: {
  keyId?: number | null;
  plaintextKey?: string | null;
  fetchFullAuthKey: (keyId: number) => Promise<string>;
  writeClipboard: (value: string) => Promise<void>;
}): Promise<string> {
  const plaintextKey = options.plaintextKey?.trim() ?? "";
  if (plaintextKey) {
    await options.writeClipboard(plaintextKey);
    return plaintextKey;
  }

  if (options.keyId == null) {
    throw new Error("missing auth key id");
  }

  const fullKey = (await options.fetchFullAuthKey(options.keyId)).trim();
  if (!fullKey) {
    throw new Error("empty auth key");
  }

  await options.writeClipboard(fullKey);
  return fullKey;
}

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

export async function writeTextToClipboard(
  value: string,
  env: {
    clipboard?: {
      writeText?: (text: string) => Promise<void>;
    };
    document?: ClipboardDocument;
  } = {
    clipboard: globalThis.navigator?.clipboard,
    document: typeof document === "undefined" ? undefined : (document as unknown as ClipboardDocument),
  },
): Promise<void> {
  if (env.clipboard?.writeText) {
    await env.clipboard.writeText(value);
    return;
  }

  const doc = env.document;
  if (!doc?.body || !doc.createElement || typeof doc.execCommand !== "function") {
    throw new Error("当前环境不支持自动复制，请手动复制 Key");
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
      throw new Error("复制失败，请手动复制 Key");
    }
  } finally {
    doc.body.removeChild(textarea);
  }
}
