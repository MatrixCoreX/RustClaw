import type { NniBancorCandle, NniBancorCandlesResponse } from "../types/api";

const CACHE_DATABASE_NAME = "agent-runtime-public-market-cache";
const CACHE_DATABASE_VERSION = 1;
const CACHE_STORE_NAME = "bancor-candles";
const CACHE_RECORD_SCHEMA_VERSION = 1;
const CACHE_MAX_AGE_MS = 30 * 24 * 60 * 60 * 1_000;
const CACHE_FUTURE_TOLERANCE_MS = 5 * 60 * 1_000;

export const BANCOR_CANDLE_REQUEST_MAX_CANDLES = 300;
export const BANCOR_CANDLE_INCREMENTAL_MIN_LIMIT = 2;

export interface BancorCandleCacheRecord {
  key: string;
  schemaVersion: 1;
  scope: string;
  intervalSeconds: number;
  cachedAtMs: number;
  response: NniBancorCandlesResponse;
  etag: string | null;
  etagRequestLimit: number | null;
}

let databasePromise: Promise<IDBDatabase> | null = null;

function normalizedScope(scope: string): string {
  return scope.trim().replace(/\/+$/, "") || "same-origin";
}

function cacheKey(scope: string, intervalSeconds: number): string {
  return `${normalizedScope(scope)}\n${intervalSeconds}`;
}

function openCacheDatabase(): Promise<IDBDatabase> | null {
  if (typeof indexedDB === "undefined") return null;
  if (databasePromise) return databasePromise;
  databasePromise = new Promise((resolve, reject) => {
    const request = indexedDB.open(CACHE_DATABASE_NAME, CACHE_DATABASE_VERSION);
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains(CACHE_STORE_NAME)) {
        const store = database.createObjectStore(CACHE_STORE_NAME, { keyPath: "key" });
        store.createIndex("cachedAtMs", "cachedAtMs");
      }
    };
    request.onsuccess = () => {
      const database = request.result;
      database.onversionchange = () => {
        database.close();
        databasePromise = null;
      };
      resolve(database);
    };
    request.onerror = () => {
      databasePromise = null;
      reject(request.error ?? new Error("bancor_candle_cache_open_failed"));
    };
    request.onblocked = () => {
      databasePromise = null;
      reject(new Error("bancor_candle_cache_open_blocked"));
    };
  });
  return databasePromise;
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("bancor_candle_cache_request_failed"));
  });
}

function transactionComplete(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("bancor_candle_cache_transaction_failed"));
    transaction.onabort = () => reject(transaction.error ?? new Error("bancor_candle_cache_transaction_aborted"));
  });
}

function isCandle(value: unknown): value is NniBancorCandle {
  if (!value || typeof value !== "object") return false;
  const candle = value as Partial<NniBancorCandle>;
  return Number.isSafeInteger(candle.bucket_start_unix)
    && Number.isSafeInteger(candle.bucket_end_unix)
    && typeof candle.open === "string"
    && typeof candle.high === "string"
    && typeof candle.low === "string"
    && typeof candle.close === "string"
    && typeof candle.point_volume_units === "string"
    && typeof candle.point_volume === "string"
    && typeof candle.usd_volume_units === "string"
    && typeof candle.usd_volume === "string"
    && Number.isSafeInteger(candle.trade_count)
    && (candle.trade_count ?? -1) >= 0
    && typeof candle.has_trades === "boolean"
    && candle.has_trades === ((candle.trade_count ?? 0) > 0);
}

export function isBancorCandleResponse(value: unknown, intervalSeconds: number): value is NniBancorCandlesResponse {
  if (!value || typeof value !== "object") return false;
  const response = value as Partial<NniBancorCandlesResponse>;
  return response.schema_version === 1
    && response.status === "bancor_candles"
    && typeof response.market_id === "string"
    && response.market_id.length > 0
    && Number.isSafeInteger(response.market_version)
    && (response.market_version ?? -1) >= 0
    && Number.isSafeInteger(response.market_created_at_unix)
    && (response.market_created_at_unix ?? -1) >= 0
    && response.price_kind === "execution_average_usd_per_point"
    && response.interval_seconds === intervalSeconds
    && Array.isArray(response.candles)
    && response.candles.every(isCandle);
}

function validatedRecord(
  value: unknown,
  scope: string,
  intervalSeconds: number,
  nowMs: number,
): BancorCandleCacheRecord | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Partial<BancorCandleCacheRecord>;
  const expectedScope = normalizedScope(scope);
  if (
    record.schemaVersion !== CACHE_RECORD_SCHEMA_VERSION
    || record.key !== cacheKey(scope, intervalSeconds)
    || record.scope !== expectedScope
    || record.intervalSeconds !== intervalSeconds
    || !Number.isFinite(record.cachedAtMs)
    || record.cachedAtMs < nowMs - CACHE_MAX_AGE_MS
    || record.cachedAtMs > nowMs + CACHE_FUTURE_TOLERANCE_MS
    || !isBancorCandleResponse(record.response, intervalSeconds)
  ) {
    return null;
  }
  return {
    key: record.key,
    schemaVersion: CACHE_RECORD_SCHEMA_VERSION,
    scope: expectedScope,
    intervalSeconds,
    cachedAtMs: record.cachedAtMs,
    response: record.response,
    etag: typeof record.etag === "string" && record.etag.trim() ? record.etag : null,
    etagRequestLimit: Number.isSafeInteger(record.etagRequestLimit) && (record.etagRequestLimit ?? 0) > 0
      ? record.etagRequestLimit!
      : null,
  };
}

export async function readBancorCandleCache(
  scope: string,
  intervalSeconds: number,
  nowMs = Date.now(),
): Promise<BancorCandleCacheRecord | null> {
  const database = openCacheDatabase();
  if (!database) return null;
  try {
    const resolved = await database;
    const transaction = resolved.transaction(CACHE_STORE_NAME, "readonly");
    const completed = transactionComplete(transaction);
    const [raw] = await Promise.all([
      requestResult(transaction.objectStore(CACHE_STORE_NAME).get(cacheKey(scope, intervalSeconds))),
      completed,
    ]);
    return validatedRecord(raw, scope, intervalSeconds, nowMs);
  } catch {
    return null;
  }
}

export async function writeBancorCandleCache({
  scope,
  intervalSeconds,
  response,
  etag,
  etagRequestLimit,
  cachedAtMs = Date.now(),
}: {
  scope: string;
  intervalSeconds: number;
  response: NniBancorCandlesResponse;
  etag: string | null;
  etagRequestLimit: number | null;
  cachedAtMs?: number;
}): Promise<void> {
  if (!isBancorCandleResponse(response, intervalSeconds)) return;
  const database = openCacheDatabase();
  if (!database) return;
  try {
    const resolved = await database;
    const transaction = resolved.transaction(CACHE_STORE_NAME, "readwrite");
    const completed = transactionComplete(transaction);
    try {
      const store = transaction.objectStore(CACHE_STORE_NAME);
      store.put({
        key: cacheKey(scope, intervalSeconds),
        schemaVersion: CACHE_RECORD_SCHEMA_VERSION,
        scope: normalizedScope(scope),
        intervalSeconds,
        cachedAtMs,
        response,
        etag: typeof etag === "string" && etag.trim() ? etag : null,
        etagRequestLimit: Number.isSafeInteger(etagRequestLimit) && (etagRequestLimit ?? 0) > 0
          ? etagRequestLimit
          : null,
      } satisfies BancorCandleCacheRecord);
      const expiryRange = IDBKeyRange.upperBound(cachedAtMs - CACHE_MAX_AGE_MS);
      const cursorRequest = store.index("cachedAtMs").openKeyCursor(expiryRange);
      cursorRequest.onsuccess = () => {
        const cursor = cursorRequest.result;
        if (!cursor) return;
        store.delete(cursor.primaryKey);
        cursor.continue();
      };
    } catch (cause) {
      transaction.abort();
      await completed.catch(() => undefined);
      throw cause;
    }
    await completed;
  } catch {
    // Persistent market data is an optimization. A blocked or full browser
    // store must never prevent the live chart from loading from the server.
  }
}

function responseSeriesChanged(
  cached: NniBancorCandlesResponse,
  incoming: NniBancorCandlesResponse,
): boolean {
  if (cached.market_id !== incoming.market_id || cached.interval_seconds !== incoming.interval_seconds) {
    return true;
  }
  return cached.market_created_at_unix !== incoming.market_created_at_unix
    || cached.price_kind !== incoming.price_kind;
}

export function mergeBancorCandleResponses(
  cached: NniBancorCandlesResponse | null,
  incoming: NniBancorCandlesResponse,
  maxCandles?: number,
): NniBancorCandlesResponse {
  const seriesChanged = !cached || responseSeriesChanged(cached, incoming);
  const incomingIsStale = !seriesChanged && incoming.market_version < cached.market_version;
  const candidates = seriesChanged
    ? incoming.candles
    : incomingIsStale
      ? [...incoming.candles, ...cached.candles]
      : [...cached.candles, ...incoming.candles];
  const byBucketStart = new Map<number, NniBancorCandle>();
  for (const candle of candidates) {
    if (isCandle(candle)) byBucketStart.set(candle.bucket_start_unix, candle);
  }
  const candles = [...byBucketStart.values()]
    .sort((left, right) => left.bucket_start_unix - right.bucket_start_unix)
    .slice(maxCandles === undefined ? 0 : -Math.max(1, Math.floor(maxCandles)));
  const newestEnvelope = incomingIsStale && cached ? cached : incoming;
  return {
    ...newestEnvelope,
    node_url: incoming.node_url ?? newestEnvelope.node_url,
    start_time_unix: candles[0]?.bucket_start_unix ?? incoming.start_time_unix,
    end_time_unix: candles.at(-1)?.bucket_end_unix ?? incoming.end_time_unix,
    candles,
  };
}

export function calculateBancorCandleRefreshLimit(
  cached: NniBancorCandlesResponse | null,
  intervalSeconds: number,
  nowUnix = Math.floor(Date.now() / 1_000),
  maxLimit = BANCOR_CANDLE_REQUEST_MAX_CANDLES,
): number {
  const normalizedMax = Math.max(BANCOR_CANDLE_INCREMENTAL_MIN_LIMIT, Math.floor(maxLimit));
  if (!cached || cached.interval_seconds !== intervalSeconds || cached.candles.length === 0) {
    return normalizedMax;
  }
  const latestBucketStart = cached.candles.reduce(
    (latest, candle) => Math.max(latest, candle.bucket_start_unix),
    -1,
  );
  if (latestBucketStart < 0 || latestBucketStart > nowUnix + intervalSeconds) return normalizedMax;
  const elapsedIntervals = Math.floor(Math.max(0, nowUnix - latestBucketStart) / intervalSeconds);
  return Math.min(normalizedMax, Math.max(BANCOR_CANDLE_INCREMENTAL_MIN_LIMIT, elapsedIntervals + 1));
}
