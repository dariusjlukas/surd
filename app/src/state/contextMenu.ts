// State for the app-wide custom context menu. One menu exists at a time;
// any component opens it via openContextMenu(event, entries). The component
// that renders it lives in components/ContextMenu.tsx.

import { create } from 'zustand'

export interface MenuItem {
  label: string
  onSelect: () => void
  danger?: boolean
  disabled?: boolean
}

export type MenuEntry = MenuItem | 'divider'

interface MenuState {
  pos: { x: number; y: number } | null
  entries: MenuEntry[]
  open(x: number, y: number, entries: MenuEntry[]): void
  close(): void
}

export const useContextMenu = create<MenuState>((set) => ({
  pos: null,
  entries: [],
  open: (x, y, entries) => set({ pos: { x, y }, entries }),
  close: () => set({ pos: null, entries: [] }),
}))

/** Open the menu at the pointer. Stops propagation so nested menu zones
 * (a cell inside the notebook background) don't both fire. */
export function openContextMenu(e: React.MouseEvent, entries: MenuEntry[]) {
  e.preventDefault()
  e.stopPropagation()
  useContextMenu.getState().open(e.clientX, e.clientY, entries)
}
