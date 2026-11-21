import assert from "node:assert/strict";
import test from "node:test";
import { ApiHttpError, ApiResponseFormatError, readTaskSubmissionResponse } from "./api-response";
import { formatUiError } from "./ui-error";
for (const status of [401,403,413,429,502]) test(`HTTP ${status} is a known rejection even with HTML or text`, async () => {
  for (const body of ["Payload too large","<html>proxy error</html>"]) {
    await assert.rejects(readTaskSubmissionResponse(new Response(body,{status})), error => error instanceof ApiHttpError && error.status === status);
  }
});
test("successful malformed response remains an ambiguous result for receipt recovery",async()=>{
  await assert.rejects(readTaskSubmissionResponse(new Response("<html>proxy</html>",{status:200})),ApiResponseFormatError);
});
test("oversize upload gives a concrete bilingual limit",()=>{
  assert.match(formatUiError(new ApiHttpError(413,"body_too_large"),(_,en)=>en,"", ""),/20 MiB.*60 MiB/);
});
