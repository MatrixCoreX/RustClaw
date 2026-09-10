import assert from "node:assert/strict";
import test from "node:test";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { CandleChart, calculateBancorChartGeometry } from "../components/BancorPage";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

for (const language of ["zh", "en"]) {
  test(`BANCOR funding detail follows hover and touch selection, not the price plot (${language})`, async () => {
    const funding = {
      bucket_start_unix: 1800000000, bucket_end_unix: 1800000060,
      open: "1", high: "2", low: "1", close: "2",
      aic_volume_units: "0", usd_volume_units: "0", aic_volume: "0", usd_volume: "0",
      trade_count: 0, has_trades: false, liquidity_event_count: 1, liquidity_usd: "2.00000000",
    };
    let renderer: ReactTestRenderer | null = null;
    await act(async () => {
      renderer = create(<CandleChart candles={[funding, {
        ...funding, bucket_start_unix: 1800000060, bucket_end_unix: 1800000120,
        open: "2", low: "2", liquidity_event_count: 0, liquidity_usd: "0",
      }]} intervalSeconds={60} priceDecimalPlaces={8} maximized={false}
        onMaximizedChange={() => {}} formatUnixDateTime={String} t={(zh, en) => language === "zh" ? zh : en} />);
    });
    try {
      const root = renderer!.root;
      const surface = root.findByProps({ "data-bancor-tap-details": "enabled" });
      const detail = () => root.findAllByProps({ "data-bancor-liquidity-detail": "true" });
      const geometry = calculateBancorChartGeometry(900);
      const event = (ratio: number, pointerType = "mouse") => ({
        clientX: (18 + (geometry.plotRight - 18) * ratio) * 900 / geometry.width,
        clientY: 100, button: 0, pointerId: 1, pointerType, preventDefault() {},
        currentTarget: { getBoundingClientRect: () => ({ left: 0, width: 900 }),
          setPointerCapture() {}, hasPointerCapture: () => true, releasePointerCapture() {} },
      });
      assert.equal(detail().length, 0);
      assert.equal(root.findAllByProps({ "data-bancor-liquidity-marker": "true" }).length, 0);
      assert.equal(root.findByProps({ "data-bancor-candle-gap": "true" }).type, "line");
      await act(async () => surface.props.onPointerMove(event(0.25)));
      assert.equal(detail().length, 1);
      assert.equal(detail()[0].children[1], language === "zh" ? "注入" : "Funding");
      assert.equal(detail()[0].findByProps({ "aria-hidden": "true" }).children.join(""), "◆ ");
      await act(async () => surface.props.onPointerMove(event(0.75)));
      assert.equal(detail().length, 0);
      await act(async () => surface.props.onPointerDown(event(0.25, "touch")));
      await act(async () => surface.props.onPointerUp(event(0.25, "touch")));
      assert.equal(detail().length, 1);
      await act(async () => surface.props.onPointerLeave({ pointerType: "touch" }));
      assert.equal(detail().length, 1);
      await act(async () => surface.props.onPointerLeave({ pointerType: "mouse" }));
      assert.equal(detail().length, 0);
    } finally {
      await act(async () => renderer?.unmount());
    }
  });
}
