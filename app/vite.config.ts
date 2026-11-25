import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';

// Tauri expects a fixed dev port and its own messages left on screen.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: ['es2022', 'safari16'],
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test/setup.ts'],
  },
});
