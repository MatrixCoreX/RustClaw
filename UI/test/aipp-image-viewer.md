# AiAPP Image Viewer Browser Regression

From `UI/`, with Playwright installed:

```sh
node test/aipp-image-viewer.browser.mjs
```

Optional environment variables:

- `PLAYWRIGHT_MODULE`: path to an already installed Playwright ESM entrypoint.
- `BROWSER_EXECUTABLE`: local Chromium/Chrome executable.
- `UI_BROWSER_ARTIFACTS`: screenshot output directory (defaults to a temporary directory).

The runner starts and closes its own loopback Vite server and browser. It uses
isolated fixtures and mocked authenticated endpoint responses; it does not log
in, read user media, change skills, or run collection tasks. Fixtures are not
included in the production build.

Coverage includes both generic AiAPP renderers, collected video covers, portrait
and landscape images, desktop/mobile, light/dark, Chinese/English, long filenames,
authenticated download endpoints and bytes, no download on image click, preview
and download retry, invalid image bytes, pending request cancellation, keyboard
focus/Escape, backdrop close, and unchanged non-image artifact previews.
