// Drag handle between panes. Pointer-capture drag reports absolute widths
// (start width ± pointer delta, clamped by the caller's setter); double-click
// resets to the default. `invert` flips the delta for right-side panes,
// which grow as the pointer moves left.

import { useRef } from 'react'

interface Props {
  width: number
  defaultWidth: number
  invert?: boolean
  onResize(px: number): void
  label: string
}

export function PaneResizer({ width, defaultWidth, invert, onResize, label }: Props) {
  const dragRef = useRef<{ pointerId: number; startX: number; startW: number } | null>(null)

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      title={`drag to resize · double-click to reset`}
      onPointerDown={(e) => {
        if (e.button !== 0) return
        dragRef.current = { pointerId: e.pointerId, startX: e.clientX, startW: width }
        e.currentTarget.setPointerCapture(e.pointerId)
      }}
      onPointerMove={(e) => {
        const drag = dragRef.current
        if (!drag || drag.pointerId !== e.pointerId) return
        const dx = e.clientX - drag.startX
        onResize(drag.startW + (invert ? -dx : dx))
      }}
      onPointerUp={(e) => {
        if (dragRef.current?.pointerId === e.pointerId) dragRef.current = null
      }}
      onPointerCancel={(e) => {
        if (dragRef.current?.pointerId === e.pointerId) dragRef.current = null
      }}
      onDoubleClick={() => onResize(defaultWidth)}
      className="group relative z-10 -mx-0.75 w-1.5 shrink-0 cursor-col-resize touch-none select-none"
    >
      <div className="h-full w-full bg-transparent transition-colors duration-150 group-hover:bg-accent/40 group-active:bg-accent/60" />
    </div>
  )
}
