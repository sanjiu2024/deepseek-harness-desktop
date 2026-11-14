import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

// Tauri serves the dev server over a fixed port and fails the build when it
// moves, so the port is pinned and strict.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // The Rust side rebuilds itself; watching it would restart Vite in a loop.
      ignored: ['**/src-tauri/**', '**/../../crates/**', '**/../../kernel/**'],
    },
  },
  build: {
    target: 'chrome110',
    sourcemap: true,
  },
})
