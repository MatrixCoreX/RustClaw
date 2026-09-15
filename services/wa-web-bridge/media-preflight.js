const fs = require("fs");

function resolveOutboundLimit(value, fallback) {
  const limit = value ?? fallback;
  if (!Number.isSafeInteger(limit) || limit < 0) {
    throw new Error("channel_media_limit_invalid");
  }
  return limit;
}

class MediaPreflightError extends Error {
  constructor(reason, actualBytes, maxBytes) {
    super(`channel_media_${reason}`);
    this.error_code = `channel_media_${reason}`;
    this.message_key = `channel.media.preflight.${reason}`;
    this.actual_bytes = actualBytes;
    this.max_bytes = maxBytes > 0 ? maxBytes : null;
  }
}

function validateOutboundFile(filePath, _mediaLabel, maxBytes) {
  const limit = resolveOutboundLimit(maxBytes, 0);
  let stat;
  try {
    stat = fs.statSync(filePath);
  } catch {
    throw new MediaPreflightError("unreadable", null, limit);
  }
  if (!stat.isFile()) {
    throw new MediaPreflightError("not_regular_file", null, limit);
  }
  if (stat.size === 0) {
    throw new MediaPreflightError("empty", 0, limit);
  }
  if (limit > 0 && stat.size > limit) {
    throw new MediaPreflightError("too_large", stat.size, limit);
  }
  return stat.size;
}

module.exports = { MediaPreflightError, resolveOutboundLimit, validateOutboundFile };
