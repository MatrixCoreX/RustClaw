import assert from "node:assert/strict";
import test from "node:test";

import {
  assetHistoryDisplayTotalPages,
  assetHistoryLocalTransactionOffset,
  assetHistoryRemotePage,
  assetHistoryRequestPath,
} from "./asset-transfer-history";

test("asset history reuses one remote batch for ten local pages", () => {
  assert.equal(assetHistoryRemotePage(1), 1);
  assert.equal(assetHistoryRemotePage(10), 1);
  assert.equal(assetHistoryRemotePage(11), 2);
  assert.equal(assetHistoryLocalTransactionOffset(1), 0);
  assert.equal(assetHistoryLocalTransactionOffset(10), 90);
  assert.equal(assetHistoryLocalTransactionOffset(11), 0);
  assert.equal(assetHistoryDisplayTotalPages(201), 21);
});

test("asset history request binds the remote batch and machine filters", () => {
  assert.equal(
    assetHistoryRequestPath("owner/+ key", "issuance", "incoming", 11),
    "/v1/nni/assets/transfers?owner_pubkey=owner%2F%2B+key&limit=100&page=2&source=issuance&direction=incoming",
  );
});
