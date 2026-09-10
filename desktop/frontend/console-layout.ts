import { useLayoutEffect, useRef } from 'react';

/** Account for desktop chrome without changing the shared browser layout. */
export function useDesktopConsoleLayout() {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const shell = ref.current;
    const bar = document.querySelector<HTMLElement>('.desktop-device-bar');
    const header = shell?.querySelector<HTMLElement>('.theme-header');
    if (!shell || !bar || !header) return;
    const measure = () => {
      shell.style.setProperty('--desktop-device-bar-height', `${bar.getBoundingClientRect().height}px`);
      shell.style.setProperty('--desktop-console-header-height', `${header.getBoundingClientRect().height}px`);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(bar);
    observer.observe(header);
    return () => observer.disconnect();
  }, []);
  return ref;
}
