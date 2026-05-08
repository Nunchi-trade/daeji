# 01 — Project Scaffold

Bootstraps the Kora Explorer at `apps/explorer/` inside the daeji monorepo.
After completing every step in order, the result is a clean Vite + React 19 + TypeScript project
that builds, type-checks, tests, and proxies RPC requests to a local or remote kora node.

Reference spec: [`../06-tech-stack.md`](../06-tech-stack.md)

---

## 1. Prerequisites Checklist

Run each command from the repository root (`/Users/will/dev/nunchi/daeji/`).

- [ ] **Node.js >= 20**
  ```bash
  node --version
  # must print v20.x or higher
  ```
- [ ] **pnpm >= 9** (preferred) or npm >= 10
  ```bash
  pnpm --version   # 9.x+
  ```
  If pnpm is not installed:
  ```bash
  corepack enable && corepack prepare pnpm@latest --activate
  ```
- [ ] **Rust monorepo builds cleanly**
  ```bash
  cargo build --all-targets
  ```
- [ ] **No existing `apps/explorer/` directory**
  ```bash
  test -d apps/explorer && echo "ERROR: apps/explorer already exists" || echo "OK"
  ```

---

## 2. Project Initialization

### 2.1 Create the directory

```bash
mkdir -p apps/explorer
```

### 2.2 Scaffold with Vite

```bash
cd apps/explorer
pnpm create vite@latest . -- --template react-ts
```

When prompted, select the current directory (`.`). If the directory already contains files
(it will not at this point), Vite will ask to overwrite — accept.

### 2.3 Remove Vite boilerplate

Delete the generated starter files that we will replace with our own:

```bash
rm -rf src/App.css src/index.css src/assets src/App.tsx src/main.tsx
```

### 2.4 Install dependencies

Replace the generated `package.json` entirely with the contents from section 3.1 below, then:

```bash
pnpm install
```

---

## 3. Configuration Files

Every file in this section must be created exactly as shown. No partial copies.

### 3.1 `apps/explorer/package.json`

```json
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
    "test": "vitest run",
    "test:watch": "vitest",
    "test:e2e": "playwright test"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "react-router": "^7.0.0",
    "@react-three/fiber": "^9.0.0",
    "@react-three/drei": "^10.0.0",
    "three": "^0.170.0",
    "regl": "^2.1.0",
    "zustand": "^5.0.0",
    "mitt": "^3.0.0",
    "viem": "^2.21.0",
    "framer-motion": "^11.0.0",
    "gsap": "^3.12.0",
    "tone": "^15.0.0"
  },
  "devDependencies": {
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@types/three": "^0.170.0",
    "@vitejs/plugin-react": "^4.0.0",
    "vite": "^6.0.0",
    "vite-plugin-glsl": "^1.3.0",
    "typescript": "~5.6.0",
    "eslint": "^9.0.0",
    "@eslint/js": "^9.0.0",
    "typescript-eslint": "^8.0.0",
    "eslint-plugin-react-hooks": "^5.0.0",
    "eslint-plugin-react-refresh": "^0.4.0",
    "globals": "^15.0.0",
    "prettier": "^3.0.0",
    "vitest": "^3.0.0",
    "@testing-library/react": "^16.0.0",
    "@testing-library/jest-dom": "^6.0.0",
    "jsdom": "^25.0.0",
    "playwright": "^1.49.0",
    "@playwright/test": "^1.49.0",
    "msw": "^2.0.0"
  }
}
```

### 3.2 `apps/explorer/tsconfig.json`

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "exactOptionalPropertyTypes": true,
    "noUncheckedIndexedAccess": true,
    "skipLibCheck": true,
    "paths": {
      "@/*": ["./src/*"]
    },
    "types": ["vite/client", "vitest/globals"],
    "lib": ["ES2022", "DOM", "DOM.Iterable"]
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

### 3.3 `apps/explorer/tsconfig.node.json`

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "types": ["node"]
  },
  "include": ["vite.config.ts"]
}
```

### 3.4 `apps/explorer/vite.config.ts`

```typescript
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
          'vendor-audio': ['tone'],
        },
      },
    },
    chunkSizeWarningLimit: 300,
  },
  server: {
    port: 5173,
    proxy: {
      '/rpc': {
        target: 'http://localhost:8545',
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/rpc/, ''),
      },
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    css: true,
  },
});
```

### 3.5 `apps/explorer/eslint.config.mjs`

```javascript
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';

export default tseslint.config(
  { ignores: ['dist'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': [
        'warn',
        { allowConstantExport: true },
      ],
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
    },
  },
);
```

### 3.6 `apps/explorer/.prettierrc`

```json
{
  "semi": true,
  "singleQuote": true,
  "trailingComma": "all",
  "printWidth": 100,
  "tabWidth": 2
}
```

---

## 4. Directory Structure

### 4.1 Create all directories

Run from `apps/explorer/`:

```bash
mkdir -p public/fonts
mkdir -p src/design
mkdir -p src/data
mkdir -p src/scenes/terrain
mkdir -p src/scenes/constellation
mkdir -p src/scenes/waterfall
mkdir -p src/scenes/consensus
mkdir -p src/scenes/aurora
mkdir -p src/scenes/organism
mkdir -p src/panels/DetailView
mkdir -p src/hooks
mkdir -p src/utils
mkdir -p e2e
```

### 4.2 Resulting tree (directories only)

```
apps/explorer/
├── e2e/
├── public/
│   └── fonts/
│       ├── Fraunces-Italic-Variable.woff2     ← added manually later
│       └── JetBrainsMono-Variable.woff2       ← added manually later
├── src/
│   ├── design/
│   │   ├── tokens.css
│   │   ├── reset.css
│   │   ├── typography.css
│   │   ├── atmosphere.css
│   │   └── glass.module.css
│   ├── data/
│   │   ├── rpc.ts
│   │   ├── poller.ts
│   │   ├── subscriber.ts
│   │   ├── bus.ts
│   │   ├── cache.ts
│   │   ├── store.ts
│   │   └── types.ts
│   ├── scenes/
│   │   ├── terrain/
│   │   ├── constellation/
│   │   ├── waterfall/
│   │   ├── consensus/
│   │   ├── aurora/
│   │   ├── organism/
│   │   └── SceneManager.tsx
│   ├── panels/
│   │   ├── GlassPanel.tsx
│   │   ├── StatusPanel.tsx
│   │   ├── BlockInfoPanel.tsx
│   │   ├── FeeWaveform.tsx
│   │   ├── SearchPanel.tsx
│   │   └── DetailView/
│   │       ├── BlockDetail.tsx
│   │       ├── TxDetail.tsx
│   │       ├── AddressDetail.tsx
│   │       └── HashArt.tsx
│   ├── hooks/
│   │   ├── useChainStore.ts
│   │   ├── useRPC.ts
│   │   ├── useScene.ts
│   │   └── useAmbientMode.ts
│   ├── utils/
│   │   ├── format.ts
│   │   ├── color.ts
│   │   └── math.ts
│   ├── main.tsx
│   ├── App.tsx
│   ├── routes.tsx
│   ├── test-setup.ts
│   └── vite-env.d.ts
├── index.html
├── package.json
├── tsconfig.json
├── tsconfig.node.json
├── vite.config.ts
├── eslint.config.mjs
├── .prettierrc
├── .env.example
└── .gitignore
```

### 4.3 Stub files

Each stub is the minimum code needed for `pnpm build` and `pnpm test` to pass
with zero TypeScript errors under strict mode.

#### `apps/explorer/index.html`

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
    <meta name="theme-color" content="#060608" />
    <title>Kora Explorer</title>

    <!-- Preload fonts for minimal FOUT -->
    <link
      rel="preload"
      href="/fonts/Fraunces-Italic-Variable.woff2"
      as="font"
      type="font/woff2"
      crossorigin
    />
    <link
      rel="preload"
      href="/fonts/JetBrainsMono-Variable.woff2"
      as="font"
      type="font/woff2"
      crossorigin
    />

    <style>
      /* Critical inline styles — prevent flash of unstyled content */
      html, body {
        margin: 0;
        padding: 0;
        background: #060608;
        color: #c8b8c0;
        font-family: 'JetBrains Mono', 'SF Mono', monospace;
        -webkit-font-smoothing: antialiased;
        -moz-osx-font-smoothing: grayscale;
        overflow: hidden;
        height: 100%;
      }
      #root {
        height: 100%;
      }
    </style>
  </head>
  <body class="ROSEDUST">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

#### `apps/explorer/src/vite-env.d.ts`

```typescript
/// <reference types="vite/client" />

// GLSL shader imports via vite-plugin-glsl
declare module '*.glsl' {
  const value: string;
  export default value;
}
declare module '*.vert.glsl' {
  const value: string;
  export default value;
}
declare module '*.frag.glsl' {
  const value: string;
  export default value;
}
```

#### `apps/explorer/src/main.tsx`

```tsx
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';

// Design system CSS — order matters
import './design/reset.css';
import './design/tokens.css';
import './design/typography.css';
import './design/atmosphere.css';

const root = document.getElementById('root');
if (!root) throw new Error('Root element not found');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

#### `apps/explorer/src/App.tsx`

```tsx
import { BrowserRouter } from 'react-router';
import { AppRoutes } from './routes';

export function App() {
  return (
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  );
}
```

#### `apps/explorer/src/routes.tsx`

```tsx
import { Routes, Route } from 'react-router';

function Home() {
  return (
    <div style={{ padding: 24 }}>
      <h1 style={{ fontFamily: 'var(--rd-font-display)', fontStyle: 'italic' }}>
        Kora Explorer
      </h1>
      <p style={{ fontFamily: 'var(--rd-font-mono)', color: 'var(--rd-text-dim)' }}>
        Scaffold loaded.
      </p>
    </div>
  );
}

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<Home />} />
    </Routes>
  );
}
```

#### `apps/explorer/src/test-setup.ts`

```typescript
import '@testing-library/jest-dom/vitest';
```

#### `apps/explorer/src/design/reset.css`

```css
*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

html, body, #root {
  height: 100%;
  width: 100%;
}

body {
  line-height: 1.5;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

img, picture, video, canvas, svg {
  display: block;
  max-width: 100%;
}

input, button, textarea, select {
  font: inherit;
}
```

#### `apps/explorer/src/design/tokens.css`

```css
:root {
  /* -- Void (backgrounds) -- */
  --rd-void:            #060608;
  --rd-void-light:      #0c0c10;
  --rd-void-surface:    #12111a;

  /* -- Rose (primary accent - blocks, heartbeat) -- */
  --rd-rose-deep:       #3a2030;
  --rd-rose-dim:        #6a4058;
  --rd-rose:            #aa7088;
  --rd-rose-bright:     #cc90a8;
  --rd-rose-glow:       #dca5bd;

  /* -- Bone (value, transactions, warmth) -- */
  --rd-bone-dim:        #8a7860;
  --rd-bone:            #b8a880;
  --rd-bone-bright:     #d8c8a0;

  /* -- Dream (pending, unresolved, liminal) -- */
  --rd-dream-dim:       #5a5a78;
  --rd-dream:           #7a7a98;
  --rd-dream-bright:    #9494b4;

  /* -- Semantic -- */
  --rd-success:         #7a8a78;
  --rd-warning:         #c89a68;
  --rd-danger:          #cc5555;

  /* -- Text -- */
  --rd-text:            #c8b8c0;
  --rd-text-dim:        #6a5a68;
  --rd-text-ghost:      #3a303a;
  --rd-text-bright:     #e8d8e0;

  /* -- Borders -- */
  --rd-border:          rgba(255, 255, 255, 0.07);
  --rd-border-strong:   rgba(255, 255, 255, 0.14);
  --rd-border-rose:     rgba(170, 112, 136, 0.3);

  /* -- Glass surfaces -- */
  --rd-glass-bg:        rgba(8, 8, 12, 0.45);
  --rd-glass-bg-hover:  rgba(170, 112, 136, 0.08);
  --rd-glass-highlight: rgba(255, 255, 255, 0.06);
  --rd-glass-blur:      12px;
  --rd-glass-saturate:  180%;

  /* -- LED indicator -- */
  --rd-led-size:        5px;
  --rd-led-connected:   var(--rd-success);
  --rd-led-warning:     var(--rd-warning);
  --rd-led-error:       var(--rd-danger);
  --rd-led-streaming:   var(--rd-rose-glow);

  /* -- Spacing (8px base grid) -- */
  --rd-space-xs:        4px;
  --rd-space-sm:        8px;
  --rd-space-md:        16px;
  --rd-space-lg:        24px;
  --rd-space-xl:        32px;
  --rd-space-2xl:       48px;

  /* -- Typography -- */
  --rd-font-mono:       'JetBrains Mono', 'SF Mono', monospace;
  --rd-font-display:    'Fraunces', Georgia, serif;

  --rd-text-xs:         10px;
  --rd-text-sm:         11px;
  --rd-text-base:       14px;
  --rd-text-lg:         18px;
  --rd-text-xl:         24px;
  --rd-text-hero:       38px;

  --rd-tracking-tight:  0.02em;
  --rd-tracking-normal: 0.06em;
  --rd-tracking-wide:   0.12em;
  --rd-tracking-label:  0.28em;
  --rd-tracking-section: 0.32em;

  /* -- Transitions -- */
  --rd-ease-out:        cubic-bezier(0.16, 1, 0.3, 1);
  --rd-ease-in-out:     cubic-bezier(0.65, 0, 0.35, 1);
  --rd-duration-fast:   80ms;
  --rd-duration-normal: 200ms;
  --rd-duration-slow:   400ms;

  /* -- Z-index layers -- */
  --rd-z-scene:         1;
  --rd-z-panels:        100;
  --rd-z-overlay:       500;
  --rd-z-search:        600;
  --rd-z-detail:        700;
  --rd-z-grain:         9997;
  --rd-z-vignette:      9998;
  --rd-z-scanlines:     9999;

  /* -- Scene-specific -- */
  --rd-activity:        0;
  --rd-ambient-opacity: 1;
}
```

#### `apps/explorer/src/design/typography.css`

```css
@font-face {
  font-family: 'Fraunces';
  src: url('/fonts/Fraunces-Italic-Variable.woff2') format('woff2');
  font-style: italic;
  font-display: swap;
  font-weight: 100 900;
}

@font-face {
  font-family: 'JetBrains Mono';
  src: url('/fonts/JetBrainsMono-Variable.woff2') format('woff2');
  font-style: normal;
  font-display: swap;
  font-weight: 100 800;
}

body {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-base);
  color: var(--rd-text);
  background: var(--rd-void);
}
```

#### `apps/explorer/src/design/atmosphere.css`

```css
/* -- Grain -- */
.grain {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-grain);
  opacity: 0.035;
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' height='300'%3E%3Cfilter id='g'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.85' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23g)'/%3E%3C/svg%3E");
}

/* -- Vignette -- */
body::before {
  content: '';
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-vignette);
  background: radial-gradient(ellipse at 50% 30%, transparent 50%, rgba(6, 6, 8, 0.72) 100%);
}

/* -- Scanlines -- */
body::after {
  content: '';
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-scanlines);
  background: repeating-linear-gradient(
    to bottom,
    transparent 0,
    transparent 2px,
    rgba(0, 0, 0, 0.45) 2px,
    rgba(0, 0, 0, 0.45) 3px
  );
  opacity: 0.06;
  mix-blend-mode: multiply;
}

/* -- Rose Wash -- */
.roseWash {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: calc(var(--rd-z-panels) - 1);
  background: radial-gradient(
    ellipse at 50% 50%,
    rgba(170, 112, 136, var(--rd-activity)) 0%,
    transparent 60%
  );
  transition: opacity 2s ease-out;
}
```

#### `apps/explorer/src/design/glass.module.css`

```css
.panel {
  background: var(--rd-glass-bg);
  backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  -webkit-backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  border: 1px solid var(--rd-border);
  box-shadow: inset 0 1px 0 var(--rd-glass-highlight);
  border-radius: 0;
  position: relative;
  transition:
    transform var(--rd-duration-fast) var(--rd-ease-out),
    background var(--rd-duration-normal) var(--rd-ease-out);
}

.panel:hover {
  transform: translateY(-2px);
  background: var(--rd-glass-bg-hover);
  transition-duration: var(--rd-duration-fast), var(--rd-duration-fast);
}

.panel:not(:hover) {
  transition-duration: 120ms, var(--rd-duration-normal);
}

.panelActive {
  border-left: 2px solid var(--rd-rose-dim);
}

.label {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  color: var(--rd-text-dim);
}

.led {
  width: var(--rd-led-size);
  height: var(--rd-led-size);
  border-radius: 50%;
  background: var(--rd-led-connected);
  box-shadow: 0 0 6px var(--rd-led-connected);
  animation: pulse 2.4s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 0.7; }
  50% { opacity: 1; }
}
```

#### `apps/explorer/src/App.test.tsx`

A minimal test to prove the vitest + React Testing Library pipeline works.

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { App } from './App';

describe('App', () => {
  it('renders the scaffold heading', () => {
    render(<App />);
    expect(screen.getByText('Kora Explorer')).toBeInTheDocument();
  });
});
```

#### `apps/explorer/src/utils/format.ts`

```typescript
/** Truncate a hex address: 0x1234...abcd */
export function truncateAddress(addr: string, chars = 4): string {
  if (addr.length <= chars * 2 + 2) return addr;
  return `${addr.slice(0, chars + 2)}...${addr.slice(-chars)}`;
}

/** Format a bigint wei value to ETH string with up to `decimals` places */
export function formatEth(wei: bigint, decimals = 4): string {
  const eth = Number(wei) / 1e18;
  return eth.toFixed(decimals);
}
```

#### `apps/explorer/src/utils/format.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { truncateAddress, formatEth } from './format';

describe('truncateAddress', () => {
  it('truncates a 42-char address', () => {
    expect(truncateAddress('0x1234567890abcdef1234567890abcdef12345678')).toBe(
      '0x1234...5678',
    );
  });

  it('returns short strings unchanged', () => {
    expect(truncateAddress('0x1234')).toBe('0x1234');
  });
});

describe('formatEth', () => {
  it('formats 1 ETH', () => {
    expect(formatEth(1_000_000_000_000_000_000n)).toBe('1.0000');
  });

  it('formats 0 ETH', () => {
    expect(formatEth(0n)).toBe('0.0000');
  });
});
```

#### `apps/explorer/.gitignore`

```
node_modules/
dist/
*.local
.env
.env.local
```

---

## 5. Monorepo Integration

### 5.1 Root `.gitignore` additions

Add the following lines to `/Users/will/dev/nunchi/daeji/.gitignore` if not already present:

```gitignore
# Explorer (JS/Vite)
apps/explorer/node_modules/
apps/explorer/dist/
```

### 5.2 Justfile additions

Append the following to `/Users/will/dev/nunchi/daeji/Justfile`:

```just
# ── Explorer (apps/explorer) ──

# Start explorer dev server
explorer-dev:
    cd apps/explorer && pnpm run dev

# Production build the explorer
explorer-build:
    cd apps/explorer && pnpm run build

# Preview the production build
explorer-preview:
    cd apps/explorer && pnpm run preview

# Run explorer unit tests
explorer-test:
    cd apps/explorer && pnpm run test

# Lint explorer source
explorer-lint:
    cd apps/explorer && pnpm run lint
```

### 5.3 Cargo workspace

**No changes to `Cargo.toml`.** The explorer is a JavaScript project. Cargo does not
know about it and does not need to. The `apps/` directory is not listed in
`workspace.members` and never will be.

---

## 6. Environment Variables

### 6.1 `apps/explorer/.env.example`

```bash
# Kora RPC endpoint (HTTP)
# In development, the Vite dev server proxies /rpc to this URL.
# In production, nginx proxies /rpc to the validator.
VITE_RPC_URL=http://localhost:8545

# Kora WebSocket endpoint (Phase 2: subscriptions)
VITE_WS_URL=ws://localhost:8546

# Chain ID — used for display and viem chain definition.
# Leave blank to auto-detect from the node via eth_chainId.
VITE_CHAIN_ID=1337
```

### 6.2 How env vars are consumed

All variables prefixed with `VITE_` are available at build time via `import.meta.env`.
Non-prefixed variables are server-side only (not exposed to the browser bundle).

```typescript
// Example usage in src/data/rpc.ts:
const rpcUrl = import.meta.env.VITE_RPC_URL ?? 'http://localhost:8545';
const wsUrl  = import.meta.env.VITE_WS_URL ?? 'ws://localhost:8546';
```

### 6.3 Local development with `.env`

Copy the example file and edit as needed:

```bash
cp .env.example .env
```

The `.env` file is gitignored. Each developer can point to their own kora node
(local devnet, remote testnet, etc.).

To point at the remote kora node from the design docs:

```bash
VITE_RPC_URL=http://65.109.61.210:8545
VITE_WS_URL=ws://65.109.61.210:8546
```

---

## 7. Complete Bootstrap Script

For convenience, every step from sections 2-6 as a single script.
Run from the repository root.

```bash
#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
EXPLORER_DIR="$REPO_ROOT/apps/explorer"

# ── Guard ──
if [ -d "$EXPLORER_DIR" ]; then
  echo "ERROR: $EXPLORER_DIR already exists. Aborting."
  exit 1
fi

# ── Create project ──
mkdir -p "$EXPLORER_DIR"
cd "$EXPLORER_DIR"

# ── package.json (created by writing the file; pnpm create vite is not used
# because we need exact control over every dependency version) ──
# Copy the package.json from section 3.1 into apps/explorer/package.json.

# ── Directory structure ──
mkdir -p public/fonts
mkdir -p src/design
mkdir -p src/data
mkdir -p src/scenes/terrain
mkdir -p src/scenes/constellation
mkdir -p src/scenes/waterfall
mkdir -p src/scenes/consensus
mkdir -p src/scenes/aurora
mkdir -p src/scenes/organism
mkdir -p src/panels/DetailView
mkdir -p src/hooks
mkdir -p src/utils
mkdir -p e2e

# ── Install ──
pnpm install

# ── Verify ──
echo ""
echo "=== Verifying scaffold ==="
pnpm run build && echo "BUILD OK" || echo "BUILD FAILED"
pnpm run test  && echo "TEST OK"  || echo "TEST FAILED"
```

> **Note:** The script above assumes all config files (tsconfig, vite.config, eslint,
> stub source files, etc.) have already been written to disk. In practice you will
> either write them by hand following sections 3-4, or generate them with a setup
> tool that reads this document.

---

## 8. Verification Checklist

Run every check from `apps/explorer/`.

- [ ] **Install succeeds**
  ```bash
  pnpm install
  # exit code 0, node_modules/ created, no peer dep errors
  ```

- [ ] **Dev server starts with HMR**
  ```bash
  pnpm dev
  # Terminal shows:  VITE v6.x.x  ready in Xms
  #                  Local:   http://localhost:5173/
  # Open in browser: page shows "Kora Explorer" heading
  # Edit src/routes.tsx, save — browser updates without full reload
  ```

- [ ] **Production build produces dist/**
  ```bash
  pnpm build
  ls dist/index.html           # exists
  ls dist/assets/*.js          # JS chunks exist
  ```

- [ ] **Unit tests pass**
  ```bash
  pnpm test
  # All tests pass (App.test.tsx, format.test.ts)
  ```

- [ ] **TypeScript strict mode has zero errors**
  ```bash
  npx tsc --noEmit
  # No output (clean)
  ```

- [ ] **RPC proxy works** (requires a running kora node on localhost:8545)
  ```bash
  # With pnpm dev running in another terminal:
  curl -s -X POST http://localhost:5173/rpc \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
  # Should return: {"jsonrpc":"2.0","id":1,"result":"0x..."}
  ```

- [ ] **Manual chunks split correctly**
  ```bash
  pnpm build 2>&1 | grep -E 'vendor-react|vendor-three|vendor-regl|vendor-data|vendor-audio'
  # Each chunk listed separately in the build output
  # Alternatively, check dist/assets/ for files containing these names
  ```

- [ ] **ESLint passes**
  ```bash
  pnpm lint
  # No errors
  ```

- [ ] **Justfile targets work from repo root**
  ```bash
  cd /Users/will/dev/nunchi/daeji
  just explorer-build
  just explorer-test
  ```

---

## 9. What This Document Does Not Cover

The following are out of scope for the scaffold and will be addressed in subsequent
implementation documents:

- Font files (`Fraunces-Italic-Variable.woff2`, `JetBrainsMono-Variable.woff2`) --
  these must be downloaded separately and placed in `public/fonts/`.
- The data layer (`src/data/*`) -- covered in `02-data-layer.md`.
- Scene implementations (`src/scenes/*`) -- covered in `03-scenes.md` onward.
- Panel components (`src/panels/*`) -- covered in `04-panels.md`.
- React hooks (`src/hooks/*`) -- covered alongside the components that use them.
- Playwright E2E tests -- covered in `05-testing.md`.
- Docker integration / nginx config -- covered in `06-deployment.md`.
- MSW mock handlers for tests -- covered in `05-testing.md`.
