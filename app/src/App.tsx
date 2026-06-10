import { useEffect } from 'react'
import { InputBar } from './components/InputBar'
import { NotebookView } from './components/NotebookView'
import { StatusBar } from './components/StatusBar'
import { WorkspacePanel } from './components/WorkspacePanel'
import { useNotebook } from './state/store'

export default function App() {
  const boot = useNotebook((s) => s.boot)
  const showWorkspace = useNotebook((s) => s.showWorkspace)

  useEffect(() => {
    // StrictMode double-invokes effects in dev; restart() is idempotent (it
    // terminates any previous worker), so the second boot simply wins.
    void boot()
  }, [boot])

  return (
    <div className="flex h-screen flex-col bg-slate-950 text-slate-100">
      <StatusBar />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <NotebookView />
          <InputBar />
        </div>
        {showWorkspace && <WorkspacePanel />}
      </div>
    </div>
  )
}
