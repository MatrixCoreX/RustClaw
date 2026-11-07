import {createServer} from 'node:http';
import {once} from 'node:events';
import {readFile} from 'node:fs/promises';
import {extname, resolve, sep} from 'node:path';

export async function builtUiServer(directory) {
  const root = resolve(directory);
  const mime = {'.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
    '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2'};
  const server = createServer(async (request, response) => {
    const pathname = new URL(request.url, 'http://fixture.invalid').pathname;
    const file = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
    if (!['GET', 'HEAD'].includes(request.method) || !file.startsWith(root + sep)) {
      response.writeHead(404).end(); return;
    }
    try {
      const bytes = await readFile(file);
      response.writeHead(200, {'Content-Type': mime[extname(file)] || 'application/octet-stream'});
      response.end(request.method === 'HEAD' ? undefined : bytes);
    } catch { response.writeHead(404).end(); }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {base: `http://127.0.0.1:${server.address().port}`,
    async close() {
      await new Promise((done, reject) => {
        server.close(error => error ? reject(error) : done());
        server.closeAllConnections();
      });
    }};
}
