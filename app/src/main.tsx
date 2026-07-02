import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
// Font Awesome: import the stylesheet once and disable the runtime injector.
// Imported before index.css so Tailwind sizing utilities override FA's 1em
// default on icons.
import { config } from '@fortawesome/fontawesome-svg-core'
import '@fortawesome/fontawesome-svg-core/styles.css'
import './index.css'
import App from './App.tsx'
import { installExternalLinkHandler } from './platform/desktop.ts'
import { primeKatexFonts } from './components/markdown.ts'

config.autoAddCss = false

// KaTeX fonts load lazily and paint invisibly while in flight; WKWebView can
// leave that math blank forever (see primeKatexFonts). Preload + self-heal.
primeKatexFonts()

// Desktop build only: route external links to the system browser so they
// don't replace the app inside its own webview. No-op in a browser.
installExternalLinkHandler()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
