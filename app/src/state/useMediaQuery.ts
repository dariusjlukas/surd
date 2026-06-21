// A tiny matchMedia hook. Used to switch the three-pane desktop layout to a
// single column with overlay drawers on phone-width screens (see App).

import { useEffect, useState } from 'react'

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches)
  useEffect(() => {
    const m = window.matchMedia(query)
    const sync = () => setMatches(m.matches)
    sync() // the query may have changed between render and effect
    m.addEventListener('change', sync)
    return () => m.removeEventListener('change', sync)
  }, [query])
  return matches
}

/** True below Tailwind's `md` breakpoint (768px) — i.e. phone-ish widths. */
export function useIsNarrow(): boolean {
  return useMediaQuery('(max-width: 767px)')
}
