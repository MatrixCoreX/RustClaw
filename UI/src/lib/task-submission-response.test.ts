import assert from "node:assert/strict";
import test from "node:test";
import { ApiHttpError, ApiResponseFormatError, readTaskSubmissionResponse } from "./api-response";
import { formatUiError } from "./ui-error";
for (const status of [401,403,413,429,502]) test(`HTTP ${status} keeps its status even with HTML or text`, async () => {
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

test("gateway failures are ambiguous and rejected non-JSON bodies are cancelled",async()=>{
  let cancelled=false;
  const body=new ReadableStream({cancel(){cancelled=true;}});
  try {await readTaskSubmissionResponse(new Response(body,{status:502}));assert.fail("expected failure");}
  catch(error){assert.ok(error instanceof ApiHttpError);assert.equal(error.definitiveRejection,false);}
  assert.equal(cancelled,true);
  assert.equal(new ApiHttpError(413,"body_too_large").definitiveRejection,true);
});
