import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    // The engine wasm is ~500 KB; that's the product, not an accident.
    chunkSizeWarningLimit: 1024,
  },
})
