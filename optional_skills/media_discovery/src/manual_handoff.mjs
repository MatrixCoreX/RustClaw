// These are local browser controls, not skill result text or routing phrases.
const LABELS = {
  en: {
    title: "Manual verification", open: "Open verification page",
    help: "Complete sign-in or verification in the platform tab, then return here. Collection will not resume until you confirm.",
    proceed: "Verification complete, resume", pause: "Pause this platform",
    checking: "Checking the platform page. Complete any remaining verification there.",
  },
  zh: {
    title: "人工验证", open: "打开验证网页",
    help: "请在平台标签页完成登录或验证，再回到这里。点击继续之前，不会自动关闭窗口或恢复采集。",
    proceed: "验证完成，继续采集", pause: "暂停此平台",
    checking: "正在检查平台页面，请先完成平台页面上剩余的验证。",
  },
};

export async function createManualConfirmation(context, verificationPage, { platform, locale = "en" } = {}) {
  const labels = LABELS[String(locale).toLowerCase().startsWith("zh") ? "zh" : "en"];
  const page = await context.newPage();
  let action = "waiting";
  await page.exposeBinding("manualCollectionAction", async (source, requested) => {
    if (source.page !== page || source.frame !== page.mainFrame() || page.url() !== "about:blank") return;
    if (requested === "open") await verificationPage.bringToFront().catch(() => {});
    else if (["continue", "pause"].includes(requested)) action = requested;
  });
  await page.setContent(`<!doctype html><html><head><meta charset="utf-8">
    <meta name="viewport" content="width=device-width,initial-scale=1">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'">
    <style>
      :root{color-scheme:light dark;font:16px system-ui,sans-serif;background:#f5f7fa;color:#20272e}
      *{box-sizing:border-box}body{margin:0;padding:32px 20px}main{max-width:560px;margin:32px auto}
      h1{font-size:24px;margin:0 0 12px}p{line-height:1.6;overflow-wrap:anywhere}
      #platform{font-size:14px;color:#53606f}nav{display:flex;flex-wrap:wrap;gap:12px;margin-top:24px}
      button{font:inherit;min-height:44px;padding:10px 16px;border:1px solid #8393a3;border-radius:6px;
        background:#fff;color:#263747;cursor:pointer;white-space:normal}
      button[data-action="continue"]{background:#24644b;color:#fff;border-color:#24644b}
      button:focus-visible{outline:3px solid #409ae1;outline-offset:3px}button:disabled{opacity:.7;cursor:wait}
      @media(prefers-color-scheme:dark){:root{background:#202427;color:#edf0f3}#platform{color:#b8c3cc}
        button{background:#30383e;color:#edf0f3;border-color:#73818c}}
    </style></head><body><main><h1></h1><p id="platform"></p><p id="help" role="status"></p>
    <nav><button data-action="open"></button><button data-action="continue"></button>
    <button data-action="pause"></button></nav></main></body></html>`);
  await page.evaluate(({ labels, platform }) => {
    document.title = `${labels.title} - ${platform}`;
    document.querySelector("h1").textContent = labels.title;
    document.querySelector("#platform").textContent = platform;
    document.querySelector("#help").textContent = labels.help;
    for (const button of document.querySelectorAll("button")) {
      const action = button.dataset.action;
      button.textContent = labels[action === "continue" ? "proceed" : action];
      button.addEventListener("click", event => {
        if (!event.isTrusted) return;
        if (action === "continue") {
          button.disabled = true;
          document.querySelector("#help").textContent = labels.checking;
        }
        window.manualCollectionAction(action);
      });
    }
  }, { labels, platform });
  await page.bringToFront();
  return {
    page,
    readAction: async () => page.isClosed() || page.url() !== "about:blank" ? "pause" : action,
  };
}
