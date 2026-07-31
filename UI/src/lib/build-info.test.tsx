import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { UiBuildBadge } from "../components/UiBuildBadge";

test("renders the build version as a directly comparable UI marker", () => {
  const markup = renderToStaticMarkup(
    <UiBuildBadge t={(zh) => zh} />,
  );

  assert.match(markup, /data-ui-build-version="test-build"/);
  assert.match(markup, />UI test-build</);
  assert.match(markup, /两个页面显示相同版本时/);
});
