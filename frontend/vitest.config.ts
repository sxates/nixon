import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import path from 'node:path';

// specs/0023 L3 — the frontend test layer. jsdom environment + the `@/*` -> ./src
// path alias (mirrors tsconfig) so hook/service/lib tests resolve imports the same
// way the app does. Tauri IPC is mocked per-test via vi.mock('@tauri-apps/api/*').
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./vitest.setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    // The Tauri/Next app code is exercised through unit seams, not the dev server.
    css: false,
  },
});
