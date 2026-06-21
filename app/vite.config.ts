import { readFileSync } from 'node:fs'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// Tauri sets TAURI_ENV_* in the env of its before{Dev,Build}Command. We only
// pin the dev server (fixed port + no clobbering Tauri's console output) when
// invoked through Tauri, so the plain `npm run dev` / GitHub Pages build keep
// Vite's defaults.
const underTauri = !!process.env.TAURI_ENV_PLATFORM

// Single source of truth for the displayed app version: package.json, kept in
// step with the Rust crates and tauri.conf.json by scripts/bump-version.sh.
// Inlined at build time as the `__APP_VERSION__` global (see src/vite-env.d.ts)
// so both the desktop and GitHub Pages builds report the version they shipped.
const { version } = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
)

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify(version),
  },
  build: {
    // The engine wasm is ~500 KB; that's the product, not an accident.
    chunkSizeWarningLimit: 1024,
  },
  ...(underTauri
    ? {
        clearScreen: false,
        server: { port: 5173, strictPort: true },
      }
    : {}),
})
