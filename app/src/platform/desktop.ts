// Thin platform shim. On the web build every function falls back to ordinary
// browser behavior; on the Tauri desktop build it routes through native
// dialogs/commands instead. `isTauri()` is the single switch — the Tauri
// packages are imported dynamically so they never enter the web bundle's
// critical path (the dynamic chunks are simply never fetched in a browser).

import { invoke, isTauri } from '@tauri-apps/api/core'

export { isTauri }

// The hosted MkDocs site. Used by the web build, and as the desktop build's
// fallback when a build shipped without the bundled offline copy.
const HOSTED_DOCS_URL = 'https://dariusjlukas.github.io/surd/docs/'

/** Open the documentation. The desktop build prefers the offline copy bundled
 * into the app (app/dist/docs, built by scripts/build-offline-docs.mjs) and
 * shows it in its own window so the notebook stays put; it falls back to the
 * hosted site when those assets aren't present — e.g. `tauri dev`, or a build
 * made without MkDocs. The web build opens the hosted site in a new tab. */
export async function openDocs(): Promise<void> {
  if (!isTauri()) {
    window.open(HOSTED_DOCS_URL, '_blank', 'noopener,noreferrer')
    return
  }
  // Probe the bundled docs (a same-origin asset under Tauri's protocol). When
  // they're absent the request fails or 404s and we fall back to the hosted
  // site via the system browser.
  const bundled = await fetch('/docs/index.html')
    .then((r) => r.ok)
    .catch(() => false)
  if (!bundled) {
    await import('@tauri-apps/plugin-opener').then((m) =>
      m.openUrl(HOSTED_DOCS_URL),
    )
    return
  }
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow')
  const existing = await WebviewWindow.getByLabel('docs')
  if (existing) {
    await existing.setFocus()
    return
  }
  new WebviewWindow('docs', {
    url: '/docs/index.html',
    title: 'surd — documentation',
    width: 1000,
    height: 760,
  })
}

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

/** Save raw bytes (given as base64) to a user-chosen file — the binary-export
 * path (.f32/.f64/.cf32/.cf64). The desktop build hands the base64 to the Rust
 * save command (which writes the decoded bytes); the web build decodes it to a
 * Blob and triggers the usual anchor download. */
export async function saveBinaryFile(
  suggestedName: string,
  base64: string,
): Promise<void> {
  if (isTauri()) {
    await invoke('save_export', {
      suggestedName,
      data: base64,
      base64: true,
    })
    return
  }
  const bin = atob(base64)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  browserDownload(
    suggestedName,
    new Blob([bytes], { type: 'application/octet-stream' }),
  )
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

/** Open the native print dialog for the app's own content — the path to "Save
 * as PDF". The desktop webview (WKWebView on macOS) treats JS `window.print()`
 * as a no-op, so the Tauri build routes through a Rust command that runs the
 * platform print operation; the web build uses the browser's `window.print()`.
 * Both render the live page under its `@media print` stylesheet. */
export async function printDocument(): Promise<void> {
  if (isTauri()) {
    await invoke('print_webview')
    return
  }
  window.print()
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
