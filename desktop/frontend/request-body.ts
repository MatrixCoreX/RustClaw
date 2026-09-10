const encoder = new TextEncoder();
const chunkSize = 65536;

async function* blobChunks(blob: Blob) {
  for (let offset = 0; offset < blob.size; offset += chunkSize) {
    yield new Uint8Array(await blob.slice(offset, offset + chunkSize).arrayBuffer());
  }
}

function quoted(value: string) {
  return value.replace(/\r\n|\r|\n/g, '\r\n').replace(/\r/g, '%0D').replace(/\n/g, '%0A').replace(/"/g, '%22');
}

async function* multipart(form: FormData, boundary: string) {
  for (const [name, value] of form) {
    const disposition = `--${boundary}\r\nContent-Disposition: form-data; name="${quoted(name)}"`;
    if (typeof value === 'string') {
      yield encoder.encode(`${disposition}\r\n\r\n${value.replace(/\r\n|\r|\n/g, '\r\n')}\r\n`);
    } else {
      yield encoder.encode(`${disposition}; filename="${quoted(value.name)}"\r\nContent-Type: ${value.type || 'application/octet-stream'}\r\n\r\n`);
      yield* blobChunks(value);
      yield encoder.encode('\r\n');
    }
  }
  yield encoder.encode(`--${boundary}--\r\n`);
}

function stream(iterator: AsyncGenerator<Uint8Array>): ReadableStream<Uint8Array> {
  return new ReadableStream({
    async pull(controller) {
      try {
        const item = await iterator.next();
        if (item.done) controller.close();
        else controller.enqueue(item.value);
      } catch (error) { controller.error(error); }
    },
    async cancel() { await iterator.return(undefined); },
  }, {highWaterMark: 1});
}

export function requestBody(init: RequestInit, headers: Headers) {
  // WebKitGTK Request(FormData).body can fail while reading blob entries. Encode
  // multipart explicitly, reading files in bounded slices without duplicating them.
  if (init.body instanceof FormData) {
    const boundary = `----desktop-${crypto.randomUUID()}`;
    headers.set('content-type', `multipart/form-data; boundary=${boundary}`);
    return stream(multipart(init.body, boundary));
  }
  if (init.body instanceof Blob) {
    if (!headers.has('content-type') && init.body.type) headers.set('content-type', init.body.type);
    return stream(blobChunks(init.body));
  }
  const request = new Request('https://desktop.invalid', {...init, headers});
  request.headers.forEach((value, key) => headers.set(key, value));
  return request.body;
}
