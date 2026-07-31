import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.env.APP_PRODUCT_IDENTITY_CONFIG
  ? path.resolve(process.env.APP_PRODUCT_IDENTITY_CONFIG)
  : path.resolve(here, "../../configs/product_identity.toml");
const raw = fs.readFileSync(configPath, "utf8");
const displayName = raw.match(/^\s*display_name\s*=\s*"([^"]+)"/m)?.[1]?.trim();
if (!displayName) throw new Error(`Missing display_name in ${configPath}`);

globalThis.__APP_DISPLAY_NAME__ = displayName;
globalThis.__APP_UI_VERSION__ = "test-build";
