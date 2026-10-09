import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', chunkSizeWarningLimit: 400, rollupOptions: { input: { main: 'index.html', tray: 'tray.html' } } },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
});
