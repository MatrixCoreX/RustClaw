import assert from "node:assert/strict";
import test from "node:test";

import { gitConnectionErrorMessage } from "../components/GitRemoteSetupPanel";

const zh = (value: string) => value;
const en = (_zh: string, value: string) => value;

test("git remote setup maps machine errors to beginner-friendly recovery guidance", () => {
  assert.match(gitConnectionErrorMessage("git_connection_revision_conflict", zh), /刷新/);
  assert.match(gitConnectionErrorMessage("git_connection_allowlist_required", en), /owner\/organization/);
  assert.match(gitConnectionErrorMessage("unknown", en), /Refresh and try again/);
});

test("git remote setup never includes a credential value in error copy", () => {
  const secret = "synthetic-secret-never-render";
  for (const code of [
    "git_credential_write_failed",
    "git_credential_delete_failed",
    secret,
  ]) {
    assert.doesNotMatch(gitConnectionErrorMessage(code, en), new RegExp(secret));
  }
});
