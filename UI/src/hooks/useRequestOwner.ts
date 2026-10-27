import { useCallback, useEffect, useMemo, useRef } from "react";

/** A fresh owner epoch prevents A -> B -> A from reviving old callbacks. */
export function useRequestOwner(scope: string, identity = "") {
  const owner = useMemo(() => ({ active: true }), [scope, identity]);
  const current = useRef(owner);
  current.current = owner;
  useEffect(() => {
    owner.active = true;
    return () => { owner.active = false; };
  }, [owner]);
  return useCallback(() => Boolean(scope.trim()) && current.current === owner && owner.active, [owner, scope]);
}
