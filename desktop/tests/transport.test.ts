import test from 'node:test';
import assert from 'node:assert/strict';
import { createTransport, relativeApiPath, type Invoke } from '../frontend/transport';
import { scopedStorage } from '../frontend/storage';

test('relative paths reject URL normalization attacks', () => {
  for (const path of ['/v1/../outside', '/v1/%2e%2e/outside', '//evil.test/v1/tasks', 'https://evil.test/v1/tasks', '/v1/x#fragment', '/v1/a%2fb']) {
    assert.throws(() => relativeApiPath(path, 'https://device.local'), undefined, path);
  }
  assert.equal(relativeApiPath('https://device.local/v1/tasks?cursor=1', 'https://device.local'), '/v1/tasks?cursor=1');
});

test('native response is pulled incrementally and cancellation reaches native', async () => {
  let reads = 0; let cancelled = false;
  const invoke: Invoke = async (cmd) => {
    if (cmd === 'request_start') return 'transfer' as never;
    if (cmd === 'request_headers') return {status:200, headers:{'content-type':'text/event-stream'}} as never;
    if (cmd === 'request_read') {reads++; return new TextEncoder().encode(`data: ${reads}\n\n`).buffer as never;}
    if (cmd === 'request_cancel') cancelled = true;
    return undefined as never;
  };
  const fetch = createTransport(invoke, 'session', 'https://device.local', async () => {});
  const response = await fetch('/v1/tasks/a/events');
  assert.ok(reads <= 1);
  const reader = response.body!.getReader();
  assert.match(new TextDecoder().decode((await reader.read()).value), /data: 1/);
  await reader.cancel(); assert.ok(cancelled);
});

test('FormData is streamed in bounded raw binary IPC chunks with generated boundary', async () => {
  let start: any; const chunks: Uint8Array[] = [];
  const invoke: Invoke = async (cmd, args) => {
    if (cmd === 'request_start') {start = args; return 'transfer' as never;}
    if (cmd === 'upload_chunk') chunks.push(args as Uint8Array);
    if (cmd === 'request_headers') return {status:204, headers:{}} as never;
    return undefined as never;
  };
  const form = new FormData(); form.append('file', new Blob([new Uint8Array(180000)]), '附件.txt');
  form.append('caption', '第一行\n第二行');
  form.append('escaped"\r\nname', 'value');
  await createTransport(invoke, 'session', 'https://device.local', async () => {})('/v1/attachments', {method:'POST', body:form, headers:{'X-Agent-Key':'renderer-key','Origin':'https://evil.test'}});
  assert.ok(chunks.length >= 3); assert.ok(chunks.every(c => c.length <= 65536));
  assert.match(start.spec.headers['content-type'], /multipart\/form-data; boundary=/);
  assert.equal(start.spec.headers['x-agent-key'], undefined);
  assert.equal(start.spec.headers.origin, undefined);
  const decoded = await new Response(new Blob(chunks), {headers:start.spec.headers}).formData();
  assert.equal((decoded.get('file') as File).name, '附件.txt');
  assert.equal((decoded.get('file') as File).size, 180000);
  assert.equal(decoded.get('caption'), '第一行\r\n第二行');
  assert.equal(decoded.get('escaped"\r\nname'), 'value');
  assert.match(new TextDecoder().decode(Buffer.concat(chunks)), /name="escaped%22%0D%0Aname"/);
});

test('abort and HTTP failures are preserved without automatic mutation retry', async () => {
  let starts = 0; let cancelled = false;
  const invoke: Invoke = async cmd => {
    if (cmd === 'request_start') { starts++; return 't' as never; }
    if (cmd === 'request_headers') return {status:403, headers:{}} as never;
    if (cmd === 'request_read') return new ArrayBuffer(0) as never;
    if (cmd === 'request_cancel') cancelled = true;
    return undefined as never;
  };
  const controller = new AbortController();
  const response = await createTransport(invoke, 's', 'https://device.local', async () => {})('/v1/tasks', {method:'POST', body:'{}', signal:controller.signal});
  assert.equal(response.status, 403); assert.equal(starts, 1);
  controller.abort(); await new Promise(r => setTimeout(r, 0)); assert.ok(cancelled);
});

test('storage separates devices and actors while global appearance stays shared', () => {
  const values = new Map<string,string>();
  const storage: Storage = {get length(){return values.size;}, key(i){return [...values.keys()][i] ?? null;}, getItem(k){return values.get(k) ?? null;},setItem(k,v){values.set(k,v);},removeItem(k){values.delete(k);},clear(){values.clear();}};
  const a = scopedStorage(storage, 'device-a.user-a.'); const b = scopedStorage(storage, 'device-b.user-a.'); const c = scopedStorage(storage, 'device-a.user-b.');
  a.setItem('draft', 'one'); b.setItem('draft', 'two');
  assert.equal(a.getItem('draft'), 'one'); assert.equal(b.getItem('draft'), 'two'); assert.equal(c.getItem('draft'), null);
  a.setItem('agent-runtime.monitor.themeMode', 'light'); assert.equal(b.getItem('agent-runtime.monitor.themeMode'), 'light');
  a.setItem('agent-runtime.monitor.userKey', 'secret'); assert.equal(a.getItem('agent-runtime.monitor.userKey'), null);
});
