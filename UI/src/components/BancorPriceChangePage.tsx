import { ArrowLeft, Calculator, ShieldCheck } from "lucide-react";
import { useState } from "react";
import type { ReactNode } from "react";

import {
  calculateBancorPriceChange,
  type BancorPriceChangeProjection,
  type BancorPriceChangeSide,
} from "../lib/bancor-price-change";
import { resolveBancorMarketDirectionColor } from "../lib/bancor-market-colors";
import type { NniBancorMarketResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

export function BancorPriceChangePage({
  market,
  onBack,
  t,
}: {
  market: NniBancorMarketResponse | null;
  onBack: () => void;
  t: Translate;
}) {
  const [amounts, setAmounts] = useState<Record<BancorPriceChangeSide, string>>({
    buy: "1.0000",
    sell: "100.0000",
  });

  return (
    <div className="mx-auto grid w-full max-w-6xl gap-5 pb-10" data-bancor-view="price-change-calculator">
      <section className="theme-shadow-card p-5 sm:p-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-xl font-semibold text-sky-200 sm:text-2xl">
              <Calculator className="h-5 w-5" />
              <h1>{t("价格变化计算", "Price change calculator")}</h1>
            </div>
            <p className="mt-3 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "输入计划支付的数量，查看这笔交易可能怎样改变资金池。所有结果都在浏览器本地计算，不会签名、提交或成交。",
                "Enter the amount you plan to pay to see how it may change the pool. Everything is calculated locally in your browser; nothing is signed, submitted, or traded.",
              )}
            </p>
          </div>
          <button type="button" className="theme-secondary-btn" onClick={onBack}>
            <ArrowLeft className="h-4 w-4" />
            {t("返回市场", "Back to market")}
          </button>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-emerald-400/20 bg-emerald-400/[0.06] px-4 py-3 text-xs leading-5 text-white/60">
          <span className="inline-flex items-center gap-1.5 font-medium text-emerald-200">
            <ShieldCheck className="h-4 w-4" />
            {t("仅本地计算", "Local calculation only")}
          </span>
          <span>
            {market
              ? t(
                `基于市场版本 ${market.version}、当前手续费 ${(market.fee_bps / 100).toFixed(2)}%`,
                `Based on market version ${market.version} and the current ${(market.fee_bps / 100).toFixed(2)}% fee`,
              )
              : t("市场数据尚未加载，请返回市场刷新后再试。", "Market data is not loaded. Return to the market, refresh, and try again.")}
          </span>
        </div>
      </section>

      <section className="grid gap-5 lg:grid-cols-2">
        <PriceChangeCalculatorCard
          side="buy"
          amount={amounts.buy}
          market={market}
          onAmountChange={(amount) => setAmounts((current) => ({ ...current, buy: amount }))}
          t={t}
        />
        <PriceChangeCalculatorCard
          side="sell"
          amount={amounts.sell}
          market={market}
          onAmountChange={(amount) => setAmounts((current) => ({ ...current, sell: amount }))}
          t={t}
        />
      </section>

      <section className="theme-panel-soft p-4 text-xs leading-5 text-white/50 sm:p-5">
        <h2 className="font-semibold text-white/75">{t("如何理解结果", "How to read the result")}</h2>
        <p className="mt-2">
          {t(
            "手续费会先从支付资产中扣除，剩余数量才进入储备曲线。预计到账和成交后储备使用与服务端相同的最小单位整数取整规则；实际成交前仍需以新的签名报价为准。",
            "The fee is deducted from the payment asset before the remainder enters the reserve curve. Estimated output and post-trade reserves use the same smallest-unit integer rounding as the server; an actual trade still requires a fresh signed quote.",
          )}
        </p>
      </section>

      <BancorPriceChangeFormula t={t} />
    </div>
  );
}

function BancorPriceChangeFormula({ t }: { t: Translate }) {
  return (
    <section
      className="theme-shadow-card p-5 sm:p-6"
      aria-label={t("价格变化计算公式", "Price-change calculation formulas")}
      data-bancor-price-change-formula="true"
    >
      <div>
        <p className="text-xs font-medium uppercase tracking-wide text-sky-300/75">BANCOR</p>
        <h2 className="mt-1 text-lg font-semibold text-white">
          {t("价格变化计算公式", "Price-change calculation formulas")}
        </h2>
        <p className="mt-2 text-xs leading-5 text-white/50">
          {t(
            "先扣除手续费，再用当前储备计算到账量；手续费不进入资金池。所有数量都按 4 位小数的最小单位做整数运算。",
            "The fee is deducted first, then output is calculated from the current reserves; fees do not enter the pool. Every amount uses integer arithmetic at four-decimal smallest-unit precision.",
          )}
        </p>
      </div>

      <div className="mt-5 grid gap-4 lg:grid-cols-2">
        <FormulaPanel
          title={t("买入 POINT（支付 USD）", "Buy POINT (pay USD)")}
          formulas={[
            <span key="buy-output">
              <var>y</var><sub>POINT</sub> = ⌊
              <MathFraction
                numerator={<><var>x</var><sub>e</sub> × <var>R</var><sub>POINT</sub></>}
                denominator={<><var>R</var><sub>USD</sub> + <var>x</var><sub>e</sub></>}
              />⌋
            </span>,
            <span key="buy-usd-reserve">
              <var>R′</var><sub>USD</sub> = <var>R</var><sub>USD</sub> + <var>x</var><sub>e</sub>
            </span>,
            <span key="buy-point-reserve">
              <var>R′</var><sub>POINT</sub> = <var>R</var><sub>POINT</sub> − <var>y</var><sub>POINT</sub>
            </span>,
          ]}
        />
        <FormulaPanel
          title={t("卖出 POINT（收到 USD）", "Sell POINT (receive USD)")}
          formulas={[
            <span key="sell-output">
              <var>y</var><sub>USD</sub> = ⌊
              <MathFraction
                numerator={<><var>x</var><sub>e</sub> × <var>R</var><sub>USD</sub></>}
                denominator={<><var>R</var><sub>POINT</sub> + <var>x</var><sub>e</sub></>}
              />⌋
            </span>,
            <span key="sell-point-reserve">
              <var>R′</var><sub>POINT</sub> = <var>R</var><sub>POINT</sub> + <var>x</var><sub>e</sub>
            </span>,
            <span key="sell-usd-reserve">
              <var>R′</var><sub>USD</sub> = <var>R</var><sub>USD</sub> − <var>y</var><sub>USD</sub>
            </span>,
          ]}
        />
      </div>

      <div className="mt-4 grid gap-4 rounded-xl border border-sky-300/15 bg-sky-400/[0.045] p-4 lg:grid-cols-3" role="math">
        <FormulaSummary
          label={t("扣除手续费", "Deduct fee")}
          formula={<><var>F</var> = ⌈<var>x</var> × <var>f</var>⌉，<var>x</var><sub>e</sub> = <var>x</var> − <var>F</var></>}
        />
        <FormulaSummary
          label={t("边际价格", "Marginal price")}
          formula={
            <>
              <var>P</var> = <MathFraction numerator={<><var>R</var><sub>USD</sub></>} denominator={<><var>R</var><sub>POINT</sub></>} />，
              <var>P′</var> = <MathFraction numerator={<><var>R′</var><sub>USD</sub></>} denominator={<><var>R′</var><sub>POINT</sub></>} />
            </>
          }
        />
        <FormulaSummary
          label={t("价格变化", "Price change")}
          formula={
            <>
              <var>ΔP</var>% = (
              <MathFraction numerator={<var>P′</var>} denominator={<var>P</var>} /> − 1) × 100%
            </>
          }
        />
      </div>

      <dl className="mt-4 grid gap-x-5 gap-y-1.5 border-t border-white/8 pt-3 text-xs leading-5 text-white/45 sm:grid-cols-2 lg:grid-cols-3">
        <FormulaDefinition symbol="x" text={t("用户填写的支付数量", "payment amount entered by the user")} />
        <FormulaDefinition symbol="f" text={t("手续费率", "fee rate")} />
        <FormulaDefinition symbol="F" text={t("向上取整的手续费", "fee rounded up")} />
        <FormulaDefinition symbol="xₑ" text={t("扣除手续费后的有效投入", "effective input after fees")} />
        <FormulaDefinition symbol="R" text={t("成交前储备", "reserve before the trade")} />
        <FormulaDefinition symbol="R′" text={t("成交后储备", "reserve after the trade")} />
        <FormulaDefinition symbol="y" text={t("向下取整的预计到账", "estimated output rounded down")} />
        <FormulaDefinition symbol="P / P′" text={t("成交前 / 成交后池内边际价", "pool marginal price before / after")} />
        <FormulaDefinition symbol="⌈ ⌉ / ⌊ ⌋" text={t("向上取整 / 向下取整到最小单位", "round up / down to the smallest unit")} />
      </dl>
    </section>
  );
}

function FormulaPanel({ title, formulas }: { title: string; formulas: ReactNode[] }) {
  return (
    <article className="min-w-0 rounded-xl border border-white/8 bg-white/[0.025] p-4">
      <h3 className="text-sm font-semibold text-white/75">{title}</h3>
      <div className="mt-3 grid gap-3 overflow-x-auto font-serif text-base text-sky-100 sm:text-lg" role="math">
        {formulas.map((formula, index) => (
          <div key={index} className="min-w-max whitespace-nowrap py-0.5">{formula}</div>
        ))}
      </div>
    </article>
  );
}

function FormulaSummary({ label, formula }: { label: string; formula: ReactNode }) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] font-medium text-white/45">{label}</p>
      <div className="mt-2 overflow-x-auto whitespace-nowrap font-serif text-base text-sky-100">
        {formula}
      </div>
    </div>
  );
}

function MathFraction({ numerator, denominator }: { numerator: ReactNode; denominator: ReactNode }) {
  return (
    <span className="mx-1 inline-grid text-center align-middle leading-tight">
      <span className="border-b border-sky-100/55 px-1.5 pb-0.5">{numerator}</span>
      <span className="px-1.5 pt-0.5">{denominator}</span>
    </span>
  );
}

function FormulaDefinition({ symbol, text }: { symbol: string; text: string }) {
  return (
    <div>
      <dt className="inline font-mono text-white/70">{symbol}</dt>
      <dd className="inline"> — {text}</dd>
    </div>
  );
}

function PriceChangeCalculatorCard({
  side,
  amount,
  market,
  onAmountChange,
  t,
}: {
  side: BancorPriceChangeSide;
  amount: string;
  market: NniBancorMarketResponse | null;
  onAmountChange: (amount: string) => void;
  t: Translate;
}) {
  const inputAsset = side === "buy" ? "USD" : "POINT";
  const outputAsset = side === "buy" ? "POINT" : "USD";
  const result = amount.trim()
    ? calculateBancorPriceChange({ side, inputAmount: amount, market })
    : null;
  const projection = result?.ok ? result.projection : null;
  const error = result?.ok === false
    ? formatCalculatorError(result.error, t)
    : null;

  return (
    <article className="theme-shadow-card min-w-0 p-5 sm:p-6" data-bancor-calculator-side={side}>
      <div>
        <p className="text-xs font-medium uppercase tracking-wide text-sky-300/75">
          {side === "buy" ? t("买入 POINT", "Buy POINT") : t("卖出 POINT", "Sell POINT")}
        </p>
        <h2 className="mt-1 text-lg font-semibold text-white">
          {side === "buy"
            ? t("使用 USD 计算", "Calculate with USD")
            : t("使用 POINT 计算", "Calculate with POINT")}
        </h2>
      </div>

      <label className="mt-5 block text-sm text-white/65">
        <span>{t("计划支付", "Planned payment")}</span>
        <div className="theme-input mt-2 flex min-w-0 items-center rounded-xl px-3 py-2.5">
          <input
            className="min-w-0 flex-1 bg-transparent font-mono text-base text-white outline-none"
            inputMode="decimal"
            autoComplete="off"
            aria-label={t(`计划支付 ${inputAsset}`, `Planned ${inputAsset} payment`)}
            value={amount}
            onChange={(event) => onAmountChange(event.target.value)}
            placeholder="0.0000"
          />
          <span className="ml-2 shrink-0 text-sm font-semibold text-white/55">{inputAsset}</span>
        </div>
      </label>

      {error ? (
        <p className="mt-3 rounded-xl border border-amber-400/25 bg-amber-400/[0.07] px-3 py-2 text-sm text-amber-100" role="alert">
          {error}
        </p>
      ) : null}

      {projection ? <ProjectionDetails projection={projection} t={t} /> : (
        <div className="mt-5 rounded-xl border border-dashed border-white/10 px-4 py-6 text-center text-sm text-white/45">
          {amount.trim()
            ? t("暂时无法计算，请确认市场数据和输入数量。", "Unable to calculate yet. Check the market data and amount.")
            : t("输入数量后立即显示结果。", "Enter an amount to see the result immediately.")}
        </div>
      )}

      <p className="mt-4 text-xs leading-5 text-white/40">
        {t(
          `这是 ${inputAsset} 换取 ${outputAsset} 的本地估算，不会使用设备私钥。`,
          `This is a local ${inputAsset}-to-${outputAsset} estimate and never uses the device private key.`,
        )}
      </p>
    </article>
  );
}

function ProjectionDetails({ projection, t }: { projection: BancorPriceChangeProjection; t: Translate }) {
  const change = Number(projection.marginalPriceChangePercent.replace("%", ""));
  const changeColor = change > 0
    ? resolveBancorMarketDirectionColor("up", t)
    : change < 0
      ? resolveBancorMarketDirectionColor("down", t)
      : "var(--theme-chart-neutral)";
  return (
    <div className="mt-5 grid gap-3" data-bancor-price-change-result="ready">
      <ResultRow
        label={t("手续费", "Fee")}
        value={`${projection.feeAmount} ${projection.inputAsset}`}
      />
      <ResultRow
        label={t("有效投入", "Effective input")}
        value={`${projection.effectiveInputAmount} ${projection.inputAsset}`}
      />
      <ResultRow
        emphasized
        label={t("预计到账", "Estimated output")}
        value={`${projection.outputAmount} ${projection.outputAsset}`}
      />
      <div className="grid gap-2 rounded-xl border border-white/8 bg-white/[0.025] p-3 sm:grid-cols-2">
        <ResultCell label={t("成交后 POINT 储备", "POINT reserve after")} value={`${projection.pointReserveAfter} POINT`} />
        <ResultCell label={t("成交后 USD 储备", "USD reserve after")} value={`${projection.usdReserveAfter} USD`} />
      </div>
      <div className="rounded-xl border border-sky-300/15 bg-sky-400/[0.05] p-3">
        <p className="text-xs text-white/45">{t("池内边际价变化", "Pool marginal-price change")}</p>
        <div className="mt-2 flex flex-wrap items-baseline gap-2 font-mono text-sm">
          <span className="break-all text-white/65">{projection.currentMarginalPrice}</span>
          <span className="text-white/35">→</span>
          <span className="break-all font-semibold text-white">{projection.marginalPriceAfter} USD / POINT</span>
        </div>
        <p className="mt-2 text-lg font-semibold" style={{ color: changeColor }}>
          {projection.marginalPriceChangePercent}
        </p>
      </div>
    </div>
  );
}

function ResultRow({
  label,
  value,
  emphasized = false,
}: {
  label: string;
  value: string;
  emphasized?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-start justify-between gap-4 border-b border-white/8 pb-3 text-sm">
      <span className="shrink-0 text-white/45">{label}</span>
      <span className={`min-w-0 break-all text-right font-mono ${emphasized ? "font-semibold text-sky-200" : "text-white/75"}`}>{value}</span>
    </div>
  );
}

function ResultCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] text-white/40">{label}</p>
      <p className="mt-1 break-all font-mono text-xs font-semibold text-white/70">{value}</p>
    </div>
  );
}

function formatCalculatorError(
  error: "amount_invalid" | "amount_too_small" | "market_capacity_exceeded" | "market_invalid",
  t: Translate,
): string {
  if (error === "market_invalid") {
    return t("市场数据尚未准备好，请返回刷新市场。", "Market data is not ready. Return and refresh the market.");
  }
  if (error === "amount_too_small") {
    return t("数量太小，扣除手续费后无法得到最小单位的预计到账。", "The amount is too small to produce a minimum-unit output after fees.");
  }
  if (error === "market_capacity_exceeded") {
    return t("计算后的储备超出市场可保存范围，请减少数量。", "The resulting reserve exceeds the market storage limit. Reduce the amount.");
  }
  return t("请输入大于 0、最多 4 位小数的数量。", "Enter an amount above 0 with no more than 4 decimal places.");
}
