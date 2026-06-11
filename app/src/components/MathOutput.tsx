// KaTeX rendering. The plain-text form is always carried as the fallback and
// hover title — it's the re-parseable form, which is the honest one.

import katex from 'katex'
import { useEffect, useRef } from 'react'

interface Props {
  latex: string
  fallback: string
}

export function MathOutput({ latex, fallback }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    try {
      katex.render(latex, ref.current!, {
        displayMode: true,
        throwOnError: true,
      })
    } catch {
      ref.current!.textContent = fallback
    }
  }, [latex, fallback])
  return <div ref={ref} title={fallback} className="overflow-x-auto py-0.5" />
}

export function MathInline({ latex, fallback }: Props) {
  const ref = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    try {
      katex.render(latex, ref.current!, {
        displayMode: false,
        throwOnError: true,
      })
    } catch {
      ref.current!.textContent = fallback
    }
  }, [latex, fallback])
  return <span ref={ref} title={fallback} />
}
