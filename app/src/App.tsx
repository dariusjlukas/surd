import { useEffect } from 'react'
import { ContextMenuHost } from './components/ContextMenu'
import { InputBar } from './components/InputBar'
import { NotebookView } from './components/NotebookView'
import { PaneResizer } from './components/PaneResizer'
import { SettingsPage } from './components/SettingsPage'
import { Sidebar } from './components/Sidebar'
import { StatusBar } from './components/StatusBar'
import { WorkspacePanel } from './components/WorkspacePanel'
import { SIDEBAR_WIDTH, WORKSPACE_WIDTH, useSettings } from './state/settings'
import { useNotebook } from './state/store'

export default function App() {
  const boot = useNotebook((s) => s.boot)
  const showWorkspace = useNotebook((s) => s.showWorkspace)
  const showSidebar = useNotebook((s) => s.showSidebar)
  const showSettings = useNotebook((s) => s.showSettings)
  const sidebarWidth = useSettings((s) => s.sidebarWidth)
  const workspaceWidth = useSettings((s) => s.workspaceWidth)
  const setSidebarWidth = useSettings((s) => s.setSidebarWidth)
  const setWorkspaceWidth = useSettings((s) => s.setWorkspaceWidth)

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
          {showSidebar && (
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
          {showWorkspace && (
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
        </div>
      )}
      <ContextMenuHost />
    </div>
  )
}
