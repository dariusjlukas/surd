import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// Tauri sets TAURI_ENV_* in the env of its before{Dev,Build}Command. We only
// pin the dev server (fixed port + no clobbering Tauri's console output) when
// invoked through Tauri, so the plain `npm run dev` / GitHub Pages build keep
// Vite's defaults.
const underTauri = !!process.env.TAURI_ENV_PLATFORM

export default defineConfig({
  plugins: [react(), tailwindcss()],
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
