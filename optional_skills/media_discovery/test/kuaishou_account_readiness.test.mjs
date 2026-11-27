import assert from "node:assert/strict";
import test from "node:test";
import { kuaishouToolbarNeedsLogin } from "../src/browser_kuaishou_search.mjs";

const authCookie = { name: "kuaishou.server.webday7_st", value: "fixture", expires: -1 };

function toolbar({ cookies = [], signedIn = false, settles = false } = {}) {
  let guest = !signedIn;
  const calls = { cookieReads: 0, waits: 0 };
  return {
    calls,
    context: () => ({
      async cookies(url) {
        assert.equal(new URL(url).hostname, "www.kuaishou.com");
        calls.cookieReads += 1;
        return cookies;
      },
    }),
    locator: () => ({
      count: async () => Number(guest),
      first: () => ({
        async waitFor({ state, timeout }) {
          calls.waits += 1;
          assert.equal(state, "hidden");
          assert.ok(timeout > 0 && timeout <= 5_000);
          if (settles) guest = false;
          else throw Object.assign(new Error("guest remained visible"), { name: "TimeoutError" });
        },
      }),
    }),
  };
}

test("a saved Kuaishou session can hydrate before opening a QR prompt", async () => {
  const page = toolbar({ cookies: [authCookie], settles: true });
  assert.equal(await kuaishouToolbarNeedsLogin(page), false);
  assert.deepEqual(page.calls, { cookieReads: 1, waits: 1 });
});

test("an already signed-in Kuaishou toolbar skips the session check", async () => {
  const page = toolbar({ signedIn: true });
  assert.equal(await kuaishouToolbarNeedsLogin(page), false);
  assert.deepEqual(page.calls, { cookieReads: 0, waits: 0 });
});

test("expired or unrelated cookies do not delay the Kuaishou login prompt", async () => {
  const page = toolbar({ cookies: [{ ...authCookie, expires: 1 }, { ...authCookie, name: "userId" }] });
  assert.equal(await kuaishouToolbarNeedsLogin(page), true);
  assert.deepEqual(page.calls, { cookieReads: 1, waits: 0 });
});

test("Kuaishou still prompts when the site declines a saved session", async () => {
  const page = toolbar({ cookies: [authCookie] });
  assert.equal(await kuaishouToolbarNeedsLogin(page), true);
  assert.deepEqual(page.calls, { cookieReads: 1, waits: 1 });
});
