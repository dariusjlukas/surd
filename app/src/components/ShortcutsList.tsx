// A reference card for the app's keyboard and pointer interactions, rendered
// as a section in Settings. The model is keyboard-first (the input bar's line
// discipline, history recall, run-from-here) but those otherwise only surface
// as faint inline hints and tooltips; this gives them one place to look.

type Shortcut = { keys: string[]; desc: string }
type Group = { title: string; items: Shortcut[] }

// `keys` entries render as separate <kbd> chips; a single entry may itself
// read "↑ / ↓" when two keys do the same job.
const GROUPS: Group[] = [
  {
    title: 'Input bar',
    items: [
      {
        keys: ['Enter'],
        desc: 'Evaluate (inserts a newline if the line isn’t finished yet)',
      },
      { keys: ['Shift', 'Enter'], desc: 'New line' },
      {
        keys: ['↑ / ↓'],
        desc: 'Recall previous / next input (when the bar is empty)',
      },
      { keys: ['Esc'], desc: 'Clear the input' },
    ],
  },
  {
    title: 'Editing a cell',
    items: [
      { keys: ['Click'], desc: 'Edit a math cell · double-click a text cell' },
      { keys: ['Enter'], desc: 'Run this cell, then everything below it' },
      { keys: ['⌘ / Ctrl', 'Enter'], desc: 'Run, even mid-line' },
      { keys: ['Shift', 'Enter'], desc: 'New line (text cells: render)' },
      {
        keys: ['Esc'],
        desc: 'Revert and close (first Esc dismisses autocomplete)',
      },
    ],
  },
  {
    title: 'Plots',
    items: [
      { keys: ['Drag'], desc: 'Pan' },
      { keys: ['Wheel'], desc: 'Zoom x about the cursor' },
      { keys: ['Shift', 'Wheel'], desc: 'Zoom y about the cursor' },
      { keys: ['Double-click'], desc: 'Reset the view' },
    ],
  },
]

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border border-edge-strong bg-surface px-1.5 py-0.5 font-mono text-[11px] leading-none text-muted">
      {children}
    </kbd>
  )
}

export function ShortcutsList() {
  return (
    <div className="space-y-4">
      {GROUPS.map((g) => (
        <div key={g.title}>
          <div className="mb-1.5 text-sm font-medium text-ink">{g.title}</div>
          <ul className="space-y-1">
            {g.items.map((s, i) => (
              <li
                key={i}
                className="flex items-baseline justify-between gap-4 text-sm"
              >
                <span className="text-muted">{s.desc}</span>
                <span className="flex shrink-0 items-center gap-1">
                  {s.keys.map((k, j) => (
                    <Kbd key={j}>{k}</Kbd>
                  ))}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  )
}
