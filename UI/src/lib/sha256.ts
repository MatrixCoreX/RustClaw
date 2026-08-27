import { sha256 } from "@noble/hashes/sha2.js";

const encoder = new TextEncoder();

export async function sha256Hex(value: string): Promise<string> {
  const bytes = encoder.encode(value);
  const subtle = globalThis.crypto?.subtle;
  if (subtle) {
    try {
      const digest = await subtle.digest("SHA-256", bytes);
      return bytesToHex(new Uint8Array(digest));
    } catch {
      // Some embedded browser contexts expose SubtleCrypto but reject digest calls.
    }
  }
  return bytesToHex(sha256(bytes));
}

export async function prefixedSha256(value: string): Promise<string> {
  return `sha256:${await sha256Hex(value)}`;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
