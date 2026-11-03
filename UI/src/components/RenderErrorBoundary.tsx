import { Component, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  t: (zh: string, en: string) => string;
}

/** Keep the surrounding navigation usable and never display raw exception data. */
export class RenderErrorBoundary extends Component<Props, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    const { t } = this.props;
    return (
      <section role="alert" className="rounded-xl border border-white/15 bg-white/5 p-6">
        <h2 className="text-base font-semibold text-white/90">{t("这个页面暂时无法显示", "This page could not be displayed")}</h2>
        <p className="mt-2 text-sm text-white/65">
          {t("你可以重试或切换其他页面；已有任务和记录不会因此删除。", "Retry or open another page. Existing tasks and records are not deleted.")}
        </p>
        <button type="button" className="theme-accent-btn mt-4 px-4 py-2 text-sm"
          onClick={() => this.setState({ failed: false })}>
          {t("重新打开", "Try again")}
        </button>
      </section>
    );
  }
}
