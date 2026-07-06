// UI settings, separate from notebook data: appearance (mode + accent
// theme), pane widths, and behavior toggles. Persisted to localStorage —
// settings are tiny and synchronous hydration means no theme flash; the
// inline script in index.html reads the same key to set data attributes
// before first paint.
//
// Theme application is DOM-level (data-mode / data-theme on <html>), done by
// a module-level store subscription rather than a React effect so it runs
// synchronously with every change — components that sample CSS variables
// (the plot painter) always see the attributes already applied.

import { create } from 'zustand'
import { persist } from 'zustand/middleware'

export type ThemeMode = 'light' | 'dark' | 'system'

/** How 3D surfaces are drawn: opaque shaded, semi-transparent, or wireframe. */
export type SurfaceRender = 'solid' | 'glass' | 'wire'

export interface AccentTheme {
  id: string
  label: string
  /** [dark-mode accent, light-mode accent] — for swatch previews. */
  swatch: [string, string]
}

export const ACCENT_THEMES: AccentTheme[] = [
  { id: 'sky', label: 'Sky', swatch: ['#38bdf8', '#0284c7'] },
  { id: 'violet', label: 'Violet', swatch: ['#a78bfa', '#7c3aed'] },
  { id: 'emerald', label: 'Emerald', swatch: ['#34d399', '#059669'] },
  { id: 'amber', label: 'Amber', swatch: ['#fbbf24', '#d97706'] },
  { id: 'rose', label: 'Rose', swatch: ['#fb7185', '#e11d48'] },
]

export const SIDEBAR_WIDTH = { min: 180, max: 420, default: 240 }
export const WORKSPACE_WIDTH = { min: 220, max: 520, default: 288 }

interface SettingsState {
  mode: ThemeMode
  accent: string
  /** The mode actually in effect ('system' resolved) — lets components that
   * sample CSS variables react to OS theme flips. Not persisted. */
  resolvedMode: 'light' | 'dark'
  sidebarWidth: number
  workspaceWidth: number
  confirmDelete: boolean
  autoScroll: boolean
  /** Show the faint "≈ 0.333333" certified decimal preview next to exact
   * results whose magnitude isn't readable off the canonical form. */
  showApprox: boolean
  /** Draw style for 3D surface plots — a display preference shared by every
   * surface, like the theme accent. */
  surfaceRender: SurfaceRender

  setMode(mode: ThemeMode): void
  setAccent(accent: string): void
  setSidebarWidth(px: number): void
  setWorkspaceWidth(px: number): void
  setConfirmDelete(v: boolean): void
  setAutoScroll(v: boolean): void
  setShowApprox(v: boolean): void
  setSurfaceRender(v: SurfaceRender): void
}

const clamp = (v: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, Math.round(v)))

export const useSettings = create<SettingsState>()(
  persist(
    (set) => ({
      mode: 'system',
      accent: 'sky',
      resolvedMode: 'dark',
      sidebarWidth: SIDEBAR_WIDTH.default,
      workspaceWidth: WORKSPACE_WIDTH.default,
      confirmDelete: true,
      autoScroll: true,
      showApprox: true,
      surfaceRender: 'solid',

      setMode: (mode) => set({ mode }),
      setAccent: (accent) => set({ accent }),
      setSidebarWidth: (px) =>
        set({ sidebarWidth: clamp(px, SIDEBAR_WIDTH.min, SIDEBAR_WIDTH.max) }),
      setWorkspaceWidth: (px) =>
        set({
          workspaceWidth: clamp(px, WORKSPACE_WIDTH.min, WORKSPACE_WIDTH.max),
        }),
      setConfirmDelete: (confirmDelete) => set({ confirmDelete }),
      setAutoScroll: (autoScroll) => set({ autoScroll }),
      setShowApprox: (showApprox) => set({ showApprox }),
      setSurfaceRender: (surfaceRender) => set({ surfaceRender }),
    }),
    {
      name: 'exact.settings.v1',
      partialize: (s) => ({
        mode: s.mode,
        accent: s.accent,
        sidebarWidth: s.sidebarWidth,
        workspaceWidth: s.workspaceWidth,
        confirmDelete: s.confirmDelete,
        autoScroll: s.autoScroll,
        showApprox: s.showApprox,
        surfaceRender: s.surfaceRender,
      }),
    },
  ),
)

const media = window.matchMedia('(prefers-color-scheme: dark)')

function applyTheme() {
  const s = useSettings.getState()
  const resolved =
    s.mode === 'system' ? (media.matches ? 'dark' : 'light') : s.mode
  const root = document.documentElement
  root.dataset.mode = resolved
  root.dataset.theme = s.accent
  if (s.resolvedMode !== resolved)
    useSettings.setState({ resolvedMode: resolved })
}

useSettings.subscribe(applyTheme)
media.addEventListener('change', applyTheme)
applyTheme()
