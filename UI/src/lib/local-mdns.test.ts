import assert from "node:assert/strict";
import test from "node:test";

import { buildLocalMdnsAddresses, normalizeLocalMdnsHostname } from "./local-mdns";
import type { LocalMdnsStatus, NginxUiStatus, WebdExposureStatus } from "../types/api";

const mdnsStatus: LocalMdnsStatus = {
  supported: true,
  platform: "linux",
  hostname: "home-agent",
  mdns_name: "home-agent.local",
  responder_installed: true,
  responder_running: true,
};

const nginxStatus: NginxUiStatus = {
  supported: true,
  platform: "linux",
  installed: true,
  running: true,
  configured: true,
  ui_deployed: true,
  clawd_exposure: "loopback_only",
  local_https_supported: true,
  local_https_prepared: true,
  local_https_enabled: true,
};

test("local mDNS settings normalize one safe local hostname label", () => {
  assert.equal(normalizeLocalMdnsHostname(" HOME-Agent.local "), "home-agent");
  assert.equal(normalizeLocalMdnsHostname("home_agent"), null);
  assert.equal(normalizeLocalMdnsHostname("two.labels.local"), null);
  assert.equal(normalizeLocalMdnsHostname(`a${"b".repeat(63)}`), null);
});

test("local mDNS settings keep HTTP independent and add HTTPS only when enabled", () => {
  assert.deepEqual(buildLocalMdnsAddresses(mdnsStatus, nginxStatus, null), {
    http: "http://home-agent.local/",
    https: "https://home-agent.local/",
  });
  assert.deepEqual(buildLocalMdnsAddresses(mdnsStatus, { ...nginxStatus, local_https_enabled: false }, null), {
      http: "http://home-agent.local/",
      https: null,
  });
});

test("local mDNS settings use the direct WEBD port when nginx is not active", () => {
  const webd: WebdExposureStatus = {
    supported: true,
    platform: "linux",
    enabled: true,
    running: true,
    listen: "0.0.0.0:8788",
    port: 8788,
    externally_accessible: true,
    nginx_compatible: true,
    restart_scheduled: false,
  };
  assert.deepEqual(buildLocalMdnsAddresses(mdnsStatus, null, webd), {
    http: "http://home-agent.local:8788/",
    https: null,
  });
});
