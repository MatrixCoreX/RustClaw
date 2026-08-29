import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { AssetTransferDialog } from "../components/AssetTransferDialog";

test("asset transfer dialog uses an opaque themed panel", () => {
  const markup = renderToStaticMarkup(
    <AssetTransferDialog
      open
      asset="USD"
      t={(zh) => zh}
      sourcePublicKey="asset-owner-public-key"
      aicBalance="1.00000000"
      usdBalance="1.00000000"
      signingDeviceReady
      loading={false}
      remoteError={null}
      onClose={() => undefined}
      onSubmit={async () => null}
    />,
  );

  assert.match(markup, /role="dialog"/);
  assert.match(markup, /theme-dialog-panel/);
  assert.match(markup, />USD 转账</);
  assert.match(markup, /data-transfer-asset="USD"/);
  assert.match(markup, /data-transfer-asset-symbol="USD"/);
  assert.match(markup, /aria-label="本次转账资产：USD"/);
  assert.doesNotMatch(markup, /向合规的资产公钥转出 AIC 或 USD/);
  assert.match(markup, /aria-label="收款账户公钥"[^>]*class="theme-input font-mono text-xs"|class="theme-input font-mono text-xs"[^>]*aria-label="收款账户公钥"/);
  assert.match(markup, /aria-label="Memo"[^>]*class="theme-input text-sm"|class="theme-input text-sm"[^>]*aria-label="Memo"/);
  assert.match(markup, /aria-pressed="true"[^>]*>[^<]*<svg[\s\S]*?硬件设备代签/);
  assert.doesNotMatch(markup, /<textarea/);
  assert.doesNotMatch(markup, /role="tablist"/);
  assert.doesNotMatch(markup, /class="theme-panel /);
});
