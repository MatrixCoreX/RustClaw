import { useEffect } from "react";

/** Native mobile back navigation follows the same actions as visible buttons. */
export function useClientBack(active: boolean, action: () => void) {
  useEffect(() => {
    if (!active) return;
    const back = (event: Event) => { event.preventDefault(); action(); };
    window.addEventListener("client-back", back);
    return () => window.removeEventListener("client-back", back);
  }, [active, action]);
}
