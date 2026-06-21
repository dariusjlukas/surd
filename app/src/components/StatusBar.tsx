import {
  faBars,
  faCircleQuestion,
  faGear,
  faTableList,
} from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import { openDocs } from '../platform/desktop'
import {
  useActiveNotebook,
  useNotebook,
  type EngineStatus,
} from '../state/store'
import { useIsNarrow } from '../state/useMediaQuery'

const LABEL: Record<EngineStatus, string> = {
  booting: 'loading engine…',
  restoring: 'restoring workspace…',
  ready: 'ready',
  busy: 'evaluating…',
  failed: 'engine failed to load — reload the page',
}

const DOT: Record<EngineStatus, string> = {
  booting: 'bg-warn',
  restoring: 'bg-warn',
  ready: 'bg-ok',
  busy: 'bg-accent animate-pulse',
  failed: 'bg-danger',
}

function IconButton({
  onClick,
  title,
  active,
  children,
}: {
  onClick: () => void
  title: string
  active?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`rounded-md p-1.5 transition-colors ${
        active
          ? 'bg-accent/15 text-accent'
          : 'text-muted hover:bg-hover hover:text-ink'
      }`}
    >
      {children}
    </button>
  )
}

export function StatusBar() {
  const engineStatus = useNotebook((s) => s.engineStatus)
  const cancel = useNotebook((s) => s.cancel)
  const showWorkspace = useNotebook((s) => s.showWorkspace)
  const toggleWorkspace = useNotebook((s) => s.toggleWorkspace)
  const showSidebar = useNotebook((s) => s.showSidebar)
  const toggleSidebar = useNotebook((s) => s.toggleSidebar)
  const showSettings = useNotebook((s) => s.showSettings)
  const toggleSettings = useNotebook((s) => s.toggleSettings)
  const mobileDrawer = useNotebook((s) => s.mobileDrawer)
  const toggleMobileDrawer = useNotebook((s) => s.toggleMobileDrawer)
  const active = useActiveNotebook()
  const isNarrow = useIsNarrow()

  // On a phone the panels are overlay drawers (one at a time), driven by a
  // separate session state; on desktop they're the persisted pinned columns.
  const sidebarOpen = isNarrow ? mobileDrawer === 'sidebar' : showSidebar
  const workspaceOpen = isNarrow ? mobileDrawer === 'workspace' : showWorkspace
  const onToggleSidebar = isNarrow
    ? () => toggleMobileDrawer('sidebar')
    : toggleSidebar
  const onToggleWorkspace = isNarrow
    ? () => toggleMobileDrawer('workspace')
    : toggleWorkspace

  return (
    <header className="flex items-center gap-2 border-b border-edge bg-surface/50 px-2 py-1.5 sm:px-3">
      <IconButton
        onClick={onToggleSidebar}
        title={sidebarOpen ? 'hide notebooks' : 'show notebooks'}
        active={sidebarOpen}
      >
        <FontAwesomeIcon icon={faBars} className="h-4 w-4" />
      </IconButton>
      <h1 className="font-mono text-base font-semibold text-accent">surd</h1>
      {!showSettings && (
        <>
          <span className="hidden text-faint sm:inline">/</span>
          <span className="hidden min-w-0 truncate text-sm text-muted sm:inline">
            {active.name}
          </span>
        </>
      )}
      <span className="ml-auto flex items-center gap-1.5 text-xs text-faint">
        <span className={`h-1.5 w-1.5 rounded-full ${DOT[engineStatus]}`} />
        {LABEL[engineStatus]}
      </span>
      {engineStatus === 'busy' && (
        <button
          onClick={cancel}
          className="rounded-md bg-danger px-2.5 py-0.5 text-xs font-medium text-on-accent hover:opacity-85"
        >
          cancel
        </button>
      )}
      <IconButton
        onClick={onToggleWorkspace}
        title={
          workspaceOpen
            ? 'hide workspace variables'
            : 'show workspace variables'
        }
        active={workspaceOpen}
      >
        <FontAwesomeIcon icon={faTableList} className="h-4 w-4" />
      </IconButton>
      <IconButton onClick={() => void openDocs()} title="documentation">
        <FontAwesomeIcon icon={faCircleQuestion} className="h-4 w-4" />
      </IconButton>
      <IconButton
        onClick={toggleSettings}
        title={showSettings ? 'back to notebook' : 'settings'}
        active={showSettings}
      >
        <FontAwesomeIcon icon={faGear} className="h-4 w-4" />
      </IconButton>
    </header>
  )
}
