import { defineConfig } from 'rolldown';

export default defineConfig({
  input: { main: 'src/main.tsx' },
  output: {
    dir: 'dist',
    format: 'esm',
    entryFileNames: '[name].js',
    chunkFileNames: 'chunks/[name]-[hash].js',
  },
  platform: 'browser',
  resolve: {
    extensions: ['.tsx', '.ts', '.jsx', '.js'],
  },
});
