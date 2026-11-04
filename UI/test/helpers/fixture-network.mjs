// Install before fixture mocks: unmocked APIs must never reach a live service.
export async function guardFixtureRequests(page, base) {
  const server = new URL(base);
  if (server.protocol !== "http:" || server.hostname !== "127.0.0.1" || !server.port) {
    throw new Error("fixture_server_must_be_ephemeral_loopback");
  }
  await page.context().route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin === server.origin && !/^\/(?:v1|webd)(?:\/|$)/.test(url.pathname)) {
      return route.continue();
    }
    return route.abort("blockedbyclient");
  });
}
