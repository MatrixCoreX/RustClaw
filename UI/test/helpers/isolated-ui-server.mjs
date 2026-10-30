import { createServer as createHttpServer } from "node:http";
import { once } from "node:events";
import { createServer } from "vite";

export async function isolatedUiServer(root) {
  const http = createHttpServer();
  const vite = await createServer({ root, server: { middlewareMode: true, hmr: { server: http } } });
  http.on("request", vite.middlewares);
  try {
    http.listen(0, "127.0.0.1");
    await once(http, "listening");
  } catch (error) {
    await vite.close();
    throw error;
  }
  return {
    base: `http://127.0.0.1:${http.address().port}`,
    async close() {
      await vite.close();
      await new Promise((resolve, reject) => {
        http.close(error => error ? reject(error) : resolve());
        http.closeAllConnections();
      });
    },
  };
}
