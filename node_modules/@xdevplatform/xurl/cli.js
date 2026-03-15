#!/usr/bin/env node

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const binary = path.join(__dirname, "binary", process.platform === "win32" ? "xurl.exe" : "xurl");

if (!fs.existsSync(binary)) {
  console.error("xurl binary not found. Try reinstalling: npm install -g @xdevplatform/xurl");
  process.exit(1);
}

try {
  const result = execFileSync(binary, process.argv.slice(2), { stdio: "inherit" });
} catch (e) {
  process.exit(e.status || 1);
}
