import { useEffect, useState } from "react";

/**
 * Debounced copy of a changing value.
 *
 * Used by the mod browser: typing must not fire one registry request per
 * keystroke, and React Query keys on the debounced value so the cache holds one
 * entry per settled query rather than one per character.
 */
export function useDebouncedValue<T>(value: T, delayMs = 350): T {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);

  return debounced;
}
