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
  assert.match(markup, /data-transfer-asset="USD"/);
  assert.doesNotMatch(markup, /role="tablist"/);
  assert.doesNotMatch(markup, /class="theme-panel /);
});
