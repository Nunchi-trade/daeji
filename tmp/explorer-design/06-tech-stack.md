# Tech Stack & Project Setup

Concrete technology choices, dependency versions, build configuration, and project bootstrapping for the Kora Explorer.

---

## Core Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| react | ^19.0 | UI component framework |
| react-dom | ^19.0 | DOM rendering |
| react-router-dom | ^7.0 | URL-driven routing and navigation |
| vite | ^6.0 | Build tool, dev server, HMR |
| typescript | ^5.6 | Type safety, strict mode |
| three | ^0.170 | 3D rendering (Hash Terrain, Block Waterfall) |
| @react-three/fiber | ^9.0 | React bindings for Three.js |
| @react-three/drei | ^9.0 | Three.js helpers (OrthographicCamera, etc.) |
| regl | ^2.1 | Lightweight WebGL2 wrapper (Particle Constellation) |
| zustand | ^5.0 | State management (chain store + UI store) |
| viem | ^2.21 | Type-safe Ethereum RPC client (HTTP + WebSocket) |
| mitt | ^3.0 | Typed EventBus (200 bytes, zero deps) |
| framer-motion | ^11.0 | Panel transitions and React animations |
| gsap | ^3.12 | Scene-coordinated timeline animations |
| tone | ^15.0 | Web Audio synthesis for chain sonification |
| vite-plugin-glsl | ^1.3 | GLSL shader file imports |

### Dev Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| @vitejs/plugin-react | ^4.0 | React fast refresh for Vite |
| @types/three | ^0.170 | Three.js TypeScript definitions |
| @types/react | ^19.0 | React TypeScript definitions |
| eslint | ^9.0 | Linting |
| prettier | ^3.0 | Code formatting |
| vitest | ^2.0 | Unit testing |
| playwright | ^1.48 | E2E visual regression testing |

---

## Vite Configuration

```typescript
// apps/explorer/vite.config.ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import glsl from 'vite-plugin-glsl';
import path from 'node:path';

export default defineConfig({
  plugins: [
    react(),
    glsl(), // enables: import vertShader from './terrain.vert.glsl'
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
    },
  },
  build: {
    target: 'esnext',
    outDir: 'dist',
    rollupOptions: {
      output: {
        manualChunks: {
          // Core app (React, Zustand, router, panels)
          'vendor-react': ['react', 'react-dom', 'react-router-dom', 'zustand'],
          // Three.js ecosystem (loaded lazily with Terrain scene)
          'vendor-three': ['three', '@react-three/fiber', '@react-three/drei'],
          // Data layer
          'vendor-data': ['viem'],
          // Audio (loaded lazily, off by default)
          'vendor-audio': ['tone'],
        },
      },
    },
    chunkSizeWarningLimit: 300, // KB — warn if any chunk exceeds this
  },
  server: {
    port: 3000,
    proxy: {
      // Proxy RPC requests in dev to avoid CORS issues
      '/rpc': {
        target: 'http://65.109.61.210:8545',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/rpc/, ''),
      },
    },
  },
});
```

---

## TypeScript Configuration

```jsonc
// apps/explorer/tsconfig.json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "exactOptionalPropertyTypes": true,
    "noUncheckedIndexedAccess": true,
    "paths": {
      "@/*": ["./src/*"]
    },
    "types": ["vite/client"]
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

Strict mode is non-negotiable. `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess` catch the subtle bugs that RPC response handling is prone to.

---

## Package.json

```jsonc
// apps/explorer/package.json
{
  "name": "@kora/explorer",
  "private": true,
  "version": "0.0.1",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "lint": "eslint src/",
    "fmt": "prettier --write src/",
    "test": "vitest",
    "test:e2e": "playwright test"
  }
}
```

---

## Monorepo Integration

The explorer lives at `apps/explorer/` within the existing Rust monorepo. It does NOT interfere with Cargo workspace configuration.

### Directory Layout (within monorepo)

```
daeji/
├── Cargo.toml              # Rust workspace (unchanged)
├── crates/                  # Rust crates (unchanged)
├── apps/
│   └── explorer/            # ← NEW: React + Vite explorer
│       ├── package.json
│       ├── tsconfig.json
│       ├── vite.config.ts
│       ├── index.html
│       └── src/
│           └── ...
├── Justfile                 # Add explorer commands
└── ...
```

### Justfile Additions

```just
# Explorer commands
explorer-dev:
    cd apps/explorer && npm run dev

explorer-build:
    cd apps/explorer && npm run build

explorer-preview:
    cd apps/explorer && npm run preview
```

### .gitignore Additions

```
apps/explorer/node_modules/
apps/explorer/dist/
```

---

## Environment Variables

```bash
# apps/explorer/.env (local development)
VITE_RPC_HTTP=http://65.109.61.210:8545
VITE_RPC_WS=ws://65.109.61.210:8546

# apps/explorer/.env.local (overrides, gitignored)
# VITE_RPC_HTTP=http://localhost:8545
# VITE_RPC_WS=ws://localhost:8546
```

All env vars prefixed with `VITE_` are exposed to client code via `import.meta.env`. Non-prefixed variables are server-side only (build time).

---

## Font Assets

Two variable fonts, self-hosted (no Google Fonts dependency):

| Font | File | Size | Usage |
|------|------|------|-------|
| Fraunces Italic Variable | `public/fonts/Fraunces-Italic-Variable.woff2` | ~55KB | Block numbers, hero metrics |
| JetBrains Mono Variable | `public/fonts/JetBrainsMono-Variable.woff2` | ~45KB | All data, labels, code, addresses |

Downloaded from Google Fonts / JetBrains and served from `/fonts/`. No external requests at runtime.

---

## Browser Support

| Browser | Minimum Version | Required Feature |
|---------|----------------|------------------|
| Chrome | 113+ | WebGL2, CSS `backdrop-filter` |
| Firefox | 115+ | WebGL2, CSS `backdrop-filter` |
| Safari | 16.4+ | WebGL2, CSS `-webkit-backdrop-filter` |
| Edge | 113+ | (Chromium-based, same as Chrome) |

### Required Web APIs

- **WebGL2** — all scenes (terrain, constellation, waterfall)
- **CSS backdrop-filter** — glass panel blur effect
- **Web Audio API** — optional sonification (degrades gracefully)
- **WebSocket** — Phase 2 subscriptions (falls back to HTTP polling)
- **ResizeObserver** — responsive canvas sizing
- **requestAnimationFrame** — all animation loops
- **BigInt** — all chain numeric values (block numbers, balances, gas)

### Fallback Strategy

If WebGL2 is not available (rare in 2025+):
- Skip all 3D scenes
- Default to MOSAIC mode (Canvas2D only)
- Show warning banner: "WebGL2 required for full experience"

---

## Development Workflow

### First-Time Setup

```bash
cd apps/explorer
npm install
npm run dev
# → http://localhost:3000
```

### Hot Module Replacement

Vite HMR works for:
- React components (fast refresh, preserves state)
- CSS modules (instant style updates)
- GLSL shaders (full page reload — no shader HMR)
- Zustand stores (state preserved across edits)

### Testing Strategy

| Layer | Tool | What It Tests |
|-------|------|--------------|
| Unit | Vitest | Hash algorithms, data transforms, cache logic, color derivation |
| Component | Vitest + React Testing Library | Panel rendering, search behavior, state updates |
| Visual | Playwright | Screenshot comparison of scenes at known block states |
| Integration | Vitest + MSW | RPC response handling, error paths, reconnection |

Visual regression tests use deterministic block hashes to produce consistent screenshots. Each scene has a reference image generated from a known set of blocks.

---

## Deployment

### Static Build

```bash
npm run build
# → apps/explorer/dist/
# Total: ~600KB gzipped (excluding fonts)
```

The built explorer is a static SPA. Serve from any CDN or static file server. All routing is client-side (requires SPA fallback: serve `index.html` for all routes).

### Docker Integration

Add to the existing Docker devnet as an optional service:

```yaml
# docker/docker-compose.yml (addition)
explorer:
  image: nginx:alpine
  volumes:
    - ../apps/explorer/dist:/usr/share/nginx/html:ro
    - ./nginx-explorer.conf:/etc/nginx/conf.d/default.conf:ro
  ports:
    - "3000:80"
  depends_on:
    - validator-0
```

Nginx config proxies `/rpc` to the validator's JSON-RPC port, avoiding CORS issues in production.

### Production RPC Proxy

```nginx
# docker/nginx-explorer.conf
server {
    listen 80;
    root /usr/share/nginx/html;
    index index.html;

    # SPA fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # RPC proxy (avoids exposing node IP to browsers)
    location /rpc {
        proxy_pass http://validator-0:8545;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        # WebSocket support for subscriptions
    }

    # Static asset caching
    location /assets/ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}
```
