import { defineConfig, type Plugin } from 'vitest/config';
import react from '@vitejs/plugin-react';
import glsl from 'vite-plugin-glsl';
import path from 'node:path';
import type { IncomingMessage, ServerResponse } from 'node:http';

const RPC_TARGET = 'https://kora-production-e104.up.railway.app';

function rpcProxy(): Plugin {
  return {
    name: 'rpc-proxy',
    configureServer(server) {
      server.middlewares.use(
        '/rpc' as string,
        (req: IncomingMessage, res: ServerResponse) => {
          if (req.method === 'OPTIONS') {
            res.writeHead(204, {
              'Access-Control-Allow-Origin': '*',
              'Access-Control-Allow-Methods': 'POST',
              'Access-Control-Allow-Headers': 'Content-Type',
            });
            res.end();
            return;
          }

          let body = '';
          req.on('data', (chunk: Buffer) => {
            body += chunk.toString();
          });
          req.on('end', () => {
            fetch(RPC_TARGET, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body,
            })
              .then((r) => r.text())
              .then((text) => {
                res.writeHead(200, {
                  'Content-Type': 'application/json',
                  'Access-Control-Allow-Origin': '*',
                });
                res.end(text);
              })
              .catch((err) => {
                console.error('[rpc-proxy]', err);
                res.writeHead(502, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: String(err) }));
              });
          });
        },
      );
    },
  };
}

export default defineConfig({
  plugins: [rpcProxy(), react(), glsl()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
    },
  },
  css: {
    modules: {
      localsConvention: 'camelCaseOnly',
    },
  },
  build: {
    target: 'esnext',
    outDir: 'dist',
    rollupOptions: {
      output: {
        manualChunks: {
          'vendor-react': ['react', 'react-dom', 'react-router', 'zustand'],
          'vendor-three': ['three', '@react-three/fiber', '@react-three/drei'],
          'vendor-regl': ['regl'],
          'vendor-data': ['viem'],
        },
      },
    },
    chunkSizeWarningLimit: 300,
  },
  server: { port: 5173 },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    css: true,
  },
});
