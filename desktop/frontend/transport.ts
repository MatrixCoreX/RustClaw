import { requestBody } from './request-body';
export type Invoke = <T>(command: string, args?: Record<string, unknown> | Uint8Array, options?: {headers: Record<string, string>}) => Promise<T>;
const allowedHeaders = new Set(['accept', 'content-type', 'range', 'if-range', 'if-none-match', 'last-event-id', 'x-idempotency-key']);

export function relativeApiPath(input: string, origin: string) {
  const url = new URL(input, origin);
  const raw = input.startsWith(origin) ? input.slice(origin.length) : input;
  const path = raw.split('?')[0];
  if (url.origin !== origin || url.username || url.password || url.hash || /[\\\r\n\0]/.test(raw)
    || /%2e|%2f|%5c|%25/i.test(path) || path.includes('//') || path.split('/').some(s => s === '.' || s === '..')
    || !url.pathname.startsWith('/v1/')) throw new Error('desktop_api_path_denied');
  return url.pathname + url.search;
}

export function createTransport(invoke: Invoke, sessionId: string, origin: string, expired: () => Promise<void>) {
  return async (input: string, init: RequestInit = {}): Promise<Response> => {
    if (input === `${origin}/webd/logout`) { await expired(); return new Response('{}'); }
    const path = relativeApiPath(input, origin);
    init.signal?.throwIfAborted();
    const headers = new Headers(init.headers);
    // Browser authentication headers are never forwarded over IPC. The native session owns them.
    for (const key of [...headers.keys()]) if (!allowedHeaders.has(key)) headers.delete(key);
    const method = (init.method ?? 'GET').toUpperCase();
    const reader = requestBody({...init, method}, headers)?.getReader();
    const id = await invoke<string>('request_start', {sessionId, spec: {path, method, headers: Object.fromEntries(headers), has_body: Boolean(reader)}});
    let ended = false;
    const cancel = async () => {
      if (ended) return;
      ended = true;
      try { await reader?.cancel(); } catch { /* already released */ }
      await invoke('request_cancel', {sessionId, id});
    };
    const abort = () => { void cancel().catch(() => {}); };
    init.signal?.addEventListener('abort', abort, {once: true});
    if (init.signal?.aborted) { await cancel(); init.signal.throwIfAborted(); }
    const upload = async () => {
      if (!reader) return;
      try {
        while (true) {
          const {done, value} = await reader.read();
          if (done) break;
          for (let offset = 0; offset < value.length; offset += 65536) {
            init.signal?.throwIfAborted();
            await invoke('upload_chunk', value.subarray(offset, offset + 65536), {headers: {'x-session-id': sessionId, 'x-transfer-id': id}});
          }
        }
        await invoke('upload_finish', {sessionId, id});
      } finally { reader.releaseLock(); }
    };
    try {
      const [head] = await Promise.all([invoke<{status: number; headers: Record<string, string>}>('request_headers', {sessionId, id}), upload()]);
      const finish = () => { ended = true; init.signal?.removeEventListener('abort', abort); };
      if ([204, 205, 304].includes(head.status) || method === 'HEAD') {
        await cancel(); finish();
        return new Response(null, head);
      }
      const body = new ReadableStream<Uint8Array>({
        async pull(controller) {
          try {
            init.signal?.throwIfAborted();
            const data = await invoke<ArrayBuffer>('request_read', {sessionId, id});
            const bytes = new Uint8Array(data);
            if (bytes.length === 0) { finish(); controller.close(); }
            else controller.enqueue(bytes);
          } catch (error) { await cancel().catch(() => {}); finish(); controller.error(error); }
        },
        async cancel() { await cancel(); finish(); },
      }, {highWaterMark: 1});
      return new Response(body, head);
    } catch (error) {
      await cancel().catch(() => {});
      init.signal?.removeEventListener('abort', abort);
      throw error instanceof Error ? error : new Error(String(error));
    }
  };
}
