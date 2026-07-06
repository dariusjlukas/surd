import { useEffect } from 'react'
import { ContextMenuHost } from './components/ContextMenu'
import { InputBar } from './components/InputBar'
import { UndoToast } from './components/UndoToast'
import { NotebookView } from './components/NotebookView'
import { PaneResizer } from './components/PaneResizer'
import { SettingsPage } from './components/SettingsPage'
import { Sidebar } from './components/Sidebar'
import { StatusBar } from './components/StatusBar'
import { WorkspacePanel } from './components/WorkspacePanel'
import { SIDEBAR_WIDTH, WORKSPACE_WIDTH, useSettings } from './state/settings'
import { useNotebook } from './state/store'
import { useIsNarrow } from './state/useMediaQuery'

export default function App() {
  const boot = useNotebook((s) => s.boot)
  const showWorkspace = useNotebook((s) => s.showWorkspace)
  const showSidebar = useNotebook((s) => s.showSidebar)
  const showSettings = useNotebook((s) => s.showSettings)
  const mobileDrawer = useNotebook((s) => s.mobileDrawer)
  const closeMobileDrawer = useNotebook((s) => s.closeMobileDrawer)
  const sidebarWidth = useSettings((s) => s.sidebarWidth)
  const workspaceWidth = useSettings((s) => s.workspaceWidth)
  const setSidebarWidth = useSettings((s) => s.setSidebarWidth)
  const setWorkspaceWidth = useSettings((s) => s.setWorkspaceWidth)
  const isNarrow = useIsNarrow()

  useEffect(() => {
    // Persistence is IndexedDB now, so rehydration is async — the engine must
    // boot from the *hydrated* notebook, not the empty default. StrictMode
    // double-invokes effects in dev; restart() is idempotent (it terminates
    // any previous worker), so a second boot simply wins.
    if (useNotebook.persist.hasHydrated()) {
      void boot()
      return
    }
    return useNotebook.persist.onFinishHydration(() => void boot())
  }, [boot])

  return (
    <div className="flex h-screen flex-col bg-app text-ink">
      <StatusBar />
      {showSettings ? (
        <SettingsPage />
      ) : (
        <div className="flex min-h-0 flex-1">
          {/* Desktop: pinned, resizable side columns. */}
          {!isNarrow && showSidebar && (
            <>
              <Sidebar width={sidebarWidth} />
              <PaneResizer
                label="resize notebook list"
                width={sidebarWidth}
                defaultWidth={SIDEBAR_WIDTH.default}
                onResize={setSidebarWidth}
              />
            </>
          )}
          <div className="flex min-w-0 flex-1 flex-col">
            <NotebookView />
            <InputBar />
          </div>
          {!isNarrow && showWorkspace && (
            <>
              <PaneResizer
                label="resize workspace panel"
                width={workspaceWidth}
                defaultWidth={WORKSPACE_WIDTH.default}
                invert
                onResize={setWorkspaceWidth}
              />
              <WorkspacePanel width={workspaceWidth} />
            </>
          )}

          {/* Phone: the same panels as fixed overlay drawers over a full-width
              notebook, one at a time, dismissed by tapping the backdrop. */}
          {isNarrow && mobileDrawer && (
            <div
              className="fixed inset-0 z-30 bg-black/40"
              onClick={closeMobileDrawer}
              aria-hidden="true"
            />
          )}
          {isNarrow && mobileDrawer === 'sidebar' && (
            <Sidebar width={sidebarWidth} mobile />
          )}
          {isNarrow && mobileDrawer === 'workspace' && (
            <WorkspacePanel width={workspaceWidth} mobile />
          )}
        </div>
      )}
      <UndoToast />
      <ContextMenuHost />
    </div>
  )
}
