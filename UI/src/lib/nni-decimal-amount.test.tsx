import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniDecimalAmount } from "../components/NniDecimalAmount";

test("NNI decimal amounts keep all digits while shrinking only the fraction", () => {
  const html = renderToStaticMarkup(
    <NniDecimalAmount value="+12345.67890123 POINT" />,
  );
  assert.match(html, /data-nni-decimal-amount="\+12345\.67890123 POINT"/);
  assert.match(html, />\+12345<\/span><span class="nni-decimal-fraction">\.67890123<\/span> POINT/);
});

test("NNI decimal amounts leave non-decimal labels unchanged", () => {
  assert.equal(renderToStaticMarkup(<NniDecimalAmount value="—" />), "<span>—</span>");
});
