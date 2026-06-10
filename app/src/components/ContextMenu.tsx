// Renderer for the app-wide custom context menu (state/contextMenu.ts owns
// the state). The host renders a full-screen catcher (click-away, second
// right-click) plus the menu itself, clamped to the viewport after measuring.

import { useLayoutEffect, useEffect, useRef, useState } from 'react'
import { useContextMenu } from '../state/contextMenu'

export function ContextMenuHost() {
  const pos = useContextMenu((s) => s.pos)
  const entries = useContextMenu((s) => s.entries)
  const close = useContextMenu((s) => s.close)

  const menuRef = useRef<HTMLDivElement>(null)
  const [style, setStyle] = useState<React.CSSProperties>({})

  // Clamp to the viewport once the menu has a size.
  useLayoutEffect(() => {
    if (!pos || !menuRef.current) return
    const { offsetWidth: w, offsetHeight: h } = menuRef.current
    setStyle({
      left: Math.min(pos.x, window.innerWidth - w - 8),
      top: Math.min(pos.y, window.innerHeight - h - 8),
    })
  }, [pos, entries])

  useEffect(() => {
    if (!pos) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
    }
  }, [pos, close])

  if (!pos) return null

  return (
    <div
      className="fixed inset-0 z-50"
      onPointerDown={(e) => {
        // close on any press outside the menu (the menu stops propagation)
        if (e.target === e.currentTarget) close()
      }}
      onContextMenu={(e) => {
        e.preventDefault()
        close()
      }}
      onWheel={close}
    >
      <div
        ref={menuRef}
        style={style}
        onPointerDown={(e) => e.stopPropagation()}
        className="absolute min-w-44 rounded-lg border border-edge-strong bg-raised py-1 shadow-xl shadow-black/30"
      >
        {entries.map((entry, i) =>
          entry === 'divider' ? (
            <div key={i} className="mx-2 my-1 border-t border-edge" />
          ) : (
            <button
              key={i}
              disabled={entry.disabled}
              onClick={() => {
                close()
                entry.onSelect()
              }}
              className={`block w-full px-3 py-1.5 text-left text-sm disabled:cursor-default disabled:opacity-40 ${
                entry.danger
                  ? 'text-danger hover:bg-danger/10'
                  : 'text-ink hover:bg-hover'
              }`}
            >
              {entry.label}
            </button>
          ),
        )}
      </div>
    </div>
  )
}
