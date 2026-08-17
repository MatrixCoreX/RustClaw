type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

const DEFAULT_RETRY_DELAYS_MS = [750, 1_500, 3_000] as const;
const TRANSIENT_RESPONSE_STATUSES = new Set([408, 425, 429, 503, 504]);

export interface ResilientReadOptions {
  retryDelaysMs?: readonly number[];
}

function retryAfterMilliseconds(response: Response): number | null {
  const value = response.headers.get("retry-after")?.trim();
  if (!value) return null;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.min(seconds * 1_000, 10_000);
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return null;
  return Math.min(Math.max(0, timestamp - Date.now()), 10_000);
}

function shouldRetryResponse(response: Response): boolean {
  if (TRANSIENT_RESPONSE_STATUSES.has(response.status)) return true;
  if (response.status !== 502) return false;
  const contentType = response.headers.get("content-type")?.toLowerCase() ?? "";
  return !contentType.includes("application/json");
}

function isAbort(error: unknown, signal?: AbortSignal | null): boolean {
  if (signal?.aborted) return true;
  return error instanceof DOMException && error.name === "AbortError";
}

async function delay(milliseconds: number, signal?: AbortSignal | null): Promise<void> {
  if (milliseconds <= 0) return;
  await new Promise<void>((resolve, reject) => {
    const abort = () => {
      globalThis.clearTimeout(timeout);
      signal?.removeEventListener("abort", abort);
      reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
    };
    const timeout = globalThis.setTimeout(() => {
      signal?.removeEventListener("abort", abort);
      resolve();
    }, milliseconds);
    if (!signal) return;
    if (signal.aborted) {
      abort();
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
  });
}

export async function fetchResilientRead(
  apiFetch: ApiFetch,
  path: string,
  init?: RequestInit,
  options?: ResilientReadOptions,
): Promise<Response> {
  if (init?.method && init.method.toUpperCase() !== "GET") {
    throw new Error("resilient_read_requires_get");
  }
  const retryDelays = options?.retryDelaysMs ?? DEFAULT_RETRY_DELAYS_MS;
  let lastError: unknown = null;

  for (let attempt = 0; attempt <= retryDelays.length; attempt += 1) {
    try {
      const response = await apiFetch(path, init);
      if (attempt >= retryDelays.length || !shouldRetryResponse(response)) return response;
      const retryDelay = retryAfterMilliseconds(response) ?? retryDelays[attempt];
      await response.body?.cancel().catch(() => undefined);
      await delay(retryDelay, init?.signal);
    } catch (error) {
      if (isAbort(error, init?.signal) || attempt >= retryDelays.length) throw error;
      lastError = error;
      await delay(retryDelays[attempt], init?.signal);
    }
  }

  throw lastError ?? new Error("resilient_read_failed");
}

export function runCoalescedRead<T>(
  inFlight: Map<string, Promise<unknown>>,
  key: string,
  start: () => Promise<T>,
): Promise<T> {
  const existing = inFlight.get(key);
  if (existing) return existing as Promise<T>;
  const request = start();
  inFlight.set(key, request);
  const cleanUp = () => {
    if (inFlight.get(key) === request) inFlight.delete(key);
  };
  void request.then(cleanUp, cleanUp);
  return request;
}

export function runCoalescedResponseRead(
  inFlight: Map<string, Promise<Response>>,
  key: string,
  start: () => Promise<Response>,
): Promise<Response> {
  let request = inFlight.get(key);
  if (!request) {
    request = start();
    inFlight.set(key, request);
    const cleanUp = () => {
      if (inFlight.get(key) === request) inFlight.delete(key);
    };
    void request.then(cleanUp, cleanUp);
  }
  return request.then((response) => response.clone());
}
