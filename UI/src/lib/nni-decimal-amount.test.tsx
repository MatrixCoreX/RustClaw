import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniDecimalAmount } from "../components/NniDecimalAmount";

test("NNI decimal amounts keep all digits while shrinking only the fraction", () => {
  const html = renderToStaticMarkup(
    <NniDecimalAmount value="+12345.67890123 AIC" />,
  );
  assert.match(html, /data-nni-decimal-amount="\+12345\.67890123 AIC"/);
  assert.match(html, />\+12345<\/span><span class="nni-decimal-fraction">\.67890123<\/span> AIC/);
});

test("NNI decimal amounts leave non-decimal labels unchanged", () => {
  assert.equal(renderToStaticMarkup(<NniDecimalAmount value="—" />), "<span>—</span>");
});

test("NNI price and fee values can retain a normal-size fraction", () => {
  const html = renderToStaticMarkup(
    <NniDecimalAmount value="0.00010000 USD" shrinkFraction={false} />,
  );
  assert.match(html, /data-nni-decimal-fraction-size="normal"/);
  assert.match(html, />0\.00010000 USD<\/span>/);
  assert.doesNotMatch(html, /class="nni-decimal-fraction"/);
});
