export class PageRequestLifecycle {
  private sequence = 0;
  private foreground: number | null = null;

  begin(silent: boolean): number | null {
    if (silent && this.foreground !== null) return null;
    const id = ++this.sequence;
    if (!silent) this.foreground = id;
    return id;
  }

  isCurrent(id: number): boolean {
    return id === this.sequence;
  }

  finish(id: number): boolean {
    if (id !== this.foreground) return false;
    this.foreground = null;
    return true;
  }

  invalidate(): void {
    this.sequence += 1;
    this.foreground = null;
  }
}
