// KaTeX rendering. The plain-text form is always carried as the fallback and
// hover title — it's the re-parseable form, which is the honest one.

import katex from 'katex'
import { useEffect, useMemo, useRef } from 'react'
import { splitMathText } from '../plot/mathtext'

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

/** Matplotlib-style mathtext — plain text with `$...$` math segments — for
 * plot titles and axis labels. Math renders through KaTeX (falling back to
 * its literal text, like MathInline); the rest is ordinary text. */
export function MathText({ text }: { text: string }) {
  const segments = useMemo(() => splitMathText(text), [text])
  return (
    <span title={text}>
      {segments.map((seg, i) =>
        seg.math ? (
          <MathInline key={i} latex={seg.value} fallback={`$${seg.value}$`} />
        ) : (
          <span key={i}>{seg.value}</span>
        ),
      )}
    </span>
  )
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
