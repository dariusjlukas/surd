// Thin platform shim. On the web build every function falls back to ordinary
// browser behavior; on the Tauri desktop build it routes through native
// dialogs/commands instead. `isTauri()` is the single switch — the Tauri
// packages are imported dynamically so they never enter the web bundle's
// critical path (the dynamic chunks are simply never fetched in a browser).

import { invoke, isTauri } from '@tauri-apps/api/core'

export { isTauri }

/** Trigger the classic blob/anchor download (web build). */
function browserDownload(suggestedName: string, blob: Blob) {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = suggestedName
  a.click()
  URL.revokeObjectURL(url)
}

/** Save text (notebook / data exports) to a user-chosen file. */
export async function saveTextFile(
  suggestedName: string,
  text: string,
  mime = 'application/json',
): Promise<void> {
  if (isTauri()) {
    await invoke('save_export', {
      suggestedName,
      data: text,
      base64: false,
    })
    return
  }
  browserDownload(suggestedName, new Blob([text], { type: mime }))
}

/** Save a `data:` URL (canvas/plot snapshots) to a user-chosen file. */
export async function saveDataUrl(
  suggestedName: string,
  dataUrl: string,
): Promise<void> {
  if (isTauri()) {
    // Strip the `data:<mime>;base64,` prefix; the command writes raw bytes.
    const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1)
    await invoke('save_export', {
      suggestedName,
      data: base64,
      base64: true,
    })
    return
  }
  const a = document.createElement('a')
  a.href = dataUrl
  a.download = suggestedName
  a.click()
}

/** Install a global click handler that opens external (http/https) links in
 * the system browser instead of navigating the app's webview. No-op on the
 * web build. Returns a teardown function. */
export function installExternalLinkHandler(): () => void {
  if (!isTauri()) return () => {}
  const onClick = (e: MouseEvent) => {
    const target = e.target as HTMLElement | null
    const anchor = target?.closest?.('a')
    if (!anchor) return
    const href = anchor.getAttribute('href') ?? ''
    if (!/^https?:\/\//i.test(href)) return
    e.preventDefault()
    void import('@tauri-apps/plugin-opener').then((m) => m.openUrl(href))
  }
  document.addEventListener('click', onClick)
  return () => document.removeEventListener('click', onClick)
}
