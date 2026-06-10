// Dedicated settings view, swapped in for the notebook area (App.tsx).
// Everything here is live state — there is no save button; settings persist
// to localStorage as they change (state/settings.ts).

import { faArrowLeft } from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome'
import { ACCENT_THEMES, useSettings, type ThemeMode } from '../state/settings'
import { useNotebook } from '../state/store'

const MODES: { id: ThemeMode; label: string }[] = [
  { id: 'light', label: 'Light' },
  { id: 'dark', label: 'Dark' },
  { id: 'system', label: 'System' },
]

export function SettingsPage() {
  const toggleSettings = useNotebook((s) => s.toggleSettings)
  const notebookCount = useNotebook((s) => s.notebooks.length)
  const settings = useSettings()

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-2xl px-6 py-8">
        <div className="mb-6 flex items-center gap-3">
          <button
            onClick={toggleSettings}
            title="back to notebook"
            className="rounded-md border border-edge px-2 py-1 text-sm text-muted hover:border-edge-strong hover:text-ink"
          >
            <FontAwesomeIcon icon={faArrowLeft} className="mr-1.5 h-3 w-3" />
            back
          </button>
          <h2 className="text-lg font-semibold text-ink">Settings</h2>
        </div>

        <Section title="Appearance">
          <Field
            label="Mode"
            hint="System follows your OS preference."
          >
            <div className="flex overflow-hidden rounded-md border border-edge">
              {MODES.map((m) => (
                <button
                  key={m.id}
                  onClick={() => settings.setMode(m.id)}
                  className={`px-3 py-1 text-sm transition-colors ${
                    settings.mode === m.id
                      ? 'bg-accent/15 font-medium text-accent'
                      : 'text-muted hover:bg-hover hover:text-ink'
                  }`}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </Field>
          <Field label="Theme" hint="Accent color for prompts, highlights, and plots.">
            <div className="flex flex-wrap gap-2">
              {ACCENT_THEMES.map((t) => {
                const active = settings.accent === t.id
                const swatch = settings.resolvedMode === 'dark' ? t.swatch[0] : t.swatch[1]
                return (
                  <button
                    key={t.id}
                    onClick={() => settings.setAccent(t.id)}
                    className={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-sm transition-colors ${
                      active
                        ? 'border-accent/60 bg-accent/10 text-ink'
                        : 'border-edge text-muted hover:border-edge-strong hover:text-ink'
                    }`}
                  >
                    <span
                      className="h-3 w-3 rounded-full"
                      style={{ backgroundColor: swatch }}
                    />
                    {t.label}
                  </button>
                )
              })}
            </div>
          </Field>
        </Section>

        <Section title="Notebook">
          <ToggleField
            label="Confirm before deleting"
            hint="Ask before deleting a notebook or clearing its cells."
            value={settings.confirmDelete}
            onChange={settings.setConfirmDelete}
          />
          <ToggleField
            label="Follow new output"
            hint="Scroll to the newest cell when one is added."
            value={settings.autoScroll}
            onChange={settings.setAutoScroll}
          />
        </Section>

        <Section title="About">
          <p className="text-sm text-muted">
            <span className="font-mono font-semibold text-accent">exact</span> — a
            correct-by-default mathematical scratchpad. <code className="text-ink">1/3</code>{' '}
            stays a third, <code className="text-ink">sqrt(2)</code> stays a radical;
            floats only when you ask with <code className="text-ink">N(x)</code>.
          </p>
          <p className="mt-2 text-xs text-faint">
            {notebookCount} notebook{notebookCount === 1 ? '' : 's'} stored locally in
            your browser (IndexedDB). Nothing leaves your machine — export a notebook
            from the sidebar to back it up.
          </p>
        </Section>
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-6 rounded-lg border border-edge bg-surface/40 p-4">
      <h3 className="mb-3 text-xs font-medium uppercase tracking-wide text-faint">
        {title}
      </h3>
      <div className="space-y-4">{children}</div>
    </section>
  )
}

function Field({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div>
      <div className="mb-1.5 text-sm font-medium text-ink">{label}</div>
      {children}
      {hint && <p className="mt-1.5 text-xs text-faint">{hint}</p>}
    </div>
  )
}

function ToggleField({
  label,
  hint,
  value,
  onChange,
}: {
  label: string
  hint: string
  value: boolean
  onChange(v: boolean): void
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <div className="text-sm font-medium text-ink">{label}</div>
        <p className="mt-0.5 text-xs text-faint">{hint}</p>
      </div>
      <button
        role="switch"
        aria-checked={value}
        onClick={() => onChange(!value)}
        className={`relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors ${
          value ? 'bg-accent' : 'bg-edge-strong'
        }`}
      >
        <span
          className={`absolute top-0.5 left-0 h-4 w-4 rounded-full bg-white shadow transition-transform ${
            value ? 'translate-x-4.5' : 'translate-x-0.5'
          }`}
        />
      </button>
    </div>
  )
}
