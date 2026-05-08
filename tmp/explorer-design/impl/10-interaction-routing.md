# 10 -- Interaction & Routing

URL routing, keyboard navigation, ambient mode, scene transitions, accessibility, touch support, and performance tier detection for the Kora Explorer.

Reference specs: [`../05-interaction.md`](../05-interaction.md), [`../07-performance.md`](../07-performance.md)
Depends on: `04-state-stores`, `05-scene-terrain`, `06-scene-constellation`, `07-scene-waterfall`, `08-scene-consensus`, `09-panels-search`

---

## 10.1 URL Routing (React Router 7)

### 10.1.1 Route Definitions

File: `apps/explorer/src/routes.tsx`

```tsx
import { Routes, Route, Navigate, Outlet } from 'react-router';
import { Suspense, lazy } from 'react';
import { Explorer } from './Explorer';
import { SceneLoading } from './scenes/SceneLoading';

// Lazy-loaded scenes (code-split per chunk strategy in vite.config.ts)
const TerrainScene = lazy(() => import('./scenes/terrain/TerrainScene'));
const ConstellationCanvas = lazy(() => import('./scenes/constellation/ConstellationCanvas'));
const WaterfallScene = lazy(() => import('./scenes/waterfall/WaterfallScene'));
const ConsensusRing = lazy(() => import('./scenes/consensus/ConsensusRing'));

// Lazy-loaded detail views (in vendor-detail chunk)
const BlockDetail = lazy(() => import('./panels/DetailView/BlockDetail'));
const TxDetail = lazy(() => import('./panels/DetailView/TxDetail'));
const AddressDetail = lazy(() => import('./panels/DetailView/AddressDetail'));

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<Explorer />}>
        {/* Default redirect to terrain scene */}
        <Route index element={<Navigate to="/terrain" replace />} />

        {/* Scene routes */}
        <Route
          path="terrain"
          element={
            <Suspense fallback={<SceneLoading scene="terrain" />}>
              <TerrainScene />
            </Suspense>
          }
        />
        <Route
          path="constellation"
          element={
            <Suspense fallback={<SceneLoading scene="constellation" />}>
              <ConstellationCanvas />
            </Suspense>
          }
        />
        <Route
          path="waterfall"
          element={
            <Suspense fallback={<SceneLoading scene="waterfall" />}>
              <WaterfallScene />
            </Suspense>
          }
        />
        <Route
          path="consensus"
          element={
            <Suspense fallback={<SceneLoading scene="consensus" />}>
              <ConsensusRing />
            </Suspense>
          }
        />

        {/* Entity detail routes -- rendered as overlays on the current scene */}
        <Route
          path="block/:numberOrHash"
          element={
            <Suspense fallback={<SceneLoading scene="detail" />}>
              <BlockDetail />
            </Suspense>
          }
        />
        <Route
          path="tx/:hash"
          element={
            <Suspense fallback={<SceneLoading scene="detail" />}>
              <TxDetail />
            </Suspense>
          }
        />
        <Route
          path="address/:address"
          element={
            <Suspense fallback={<SceneLoading scene="detail" />}>
              <AddressDetail />
            </Suspense>
          }
        />
      </Route>
    </Routes>
  );
}
```

### 10.1.2 Explorer Layout Component

File: `apps/explorer/src/Explorer.tsx`

The `Explorer` component is the root layout. It renders the active scene in the background, panels as overlays, atmospheric layers, and the `<Outlet />` for detail views. The scene persists across detail view navigation -- opening `/block/123` does not unmount the current scene.

```tsx
import { Outlet, useLocation, useNavigate } from 'react-router';
import { Suspense, useMemo } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { StatusBar } from './panels/StatusBar';
import { SceneContainer } from './scenes/SceneContainer';
import { SearchOverlay } from './panels/SearchOverlay';
import { useUIStore } from './stores/ui';
import { useAmbientMode } from './hooks/useAmbientMode';
import { useKeyboardShortcuts } from './hooks/useKeyboardShortcuts';
import { usePerformanceTier } from './hooks/usePerformanceTier';
import { SkipToContent } from './a11y/SkipToContent';

export function Explorer() {
  const location = useLocation();
  useAmbientMode();
  useKeyboardShortcuts();
  usePerformanceTier();

  const ambientMode = useUIStore((s) => s.ambientMode);
  const ambientOpacity = useUIStore((s) => s.ambientOpacity);

  // Determine if the current route is a detail view overlay
  const isDetailRoute = useMemo(() => {
    const path = location.pathname;
    return (
      path.startsWith('/block/') ||
      path.startsWith('/tx/') ||
      path.startsWith('/address/')
    );
  }, [location.pathname]);

  // Determine the active scene from the URL or from stored state
  const activeScene = useUIStore((s) => s.activeScene);

  return (
    <div className="explorer" data-ambient={ambientMode}>
      <SkipToContent />

      {/* Atmospheric layers */}
      <div className="grain" aria-hidden="true" />
      <div className="roseWash" aria-hidden="true" />

      {/* Scene layer -- always running behind panels */}
      <div
        className="sceneLayer"
        style={{ opacity: isDetailRoute ? 0.3 : 1 }}
        aria-hidden="true"
      >
        <SceneContainer activeScene={activeScene} />
      </div>

      {/* Panel layer -- fades with ambient mode */}
      <div
        className="panelLayer"
        style={{ opacity: ambientOpacity }}
        role="main"
        id="main-content"
      >
        <StatusBar />
        {/* Outlet renders detail views when on /block/:id, /tx/:hash, /address/:addr */}
        <AnimatePresence mode="wait">
          {isDetailRoute && (
            <motion.div
              key={location.pathname}
              className="detailOverlay"
              initial={{ opacity: 0, x: '100%' }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: '100%' }}
              transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
              role="dialog"
              aria-label="Entity detail"
              aria-modal="true"
            >
              <Outlet />
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Search overlay -- above everything except atmosphere */}
      <SearchOverlay />
    </div>
  );
}
```

### 10.1.3 Scene-Preserving Navigation

Detail views are overlays, not scene replacements. When the user navigates to `/block/123`, the current scene keeps running in the background at reduced opacity (30%). This means:

- The `SceneContainer` renders based on `useUIStore.activeScene`, not the URL.
- Scene routes (`/terrain`, `/constellation`, etc.) update `activeScene` in the store.
- Detail routes (`/block/:id`, `/tx/:hash`, `/address/:addr`) do not change `activeScene`.
- The `Outlet` renders the detail panel on top of the dimmed scene.

```tsx
// apps/explorer/src/scenes/SceneContainer.tsx
import { Suspense, lazy, memo } from 'react';
import { ErrorBoundary } from './ErrorBoundary';
import { SceneLoading } from './SceneLoading';
import { SceneError } from './SceneError';
import type { SceneName } from '../stores/ui';

const scenes: Record<SceneName, React.LazyExoticComponent<React.ComponentType>> = {
  terrain: lazy(() => import('./terrain/TerrainScene')),
  constellation: lazy(() => import('./constellation/ConstellationCanvas')),
  waterfall: lazy(() => import('./waterfall/WaterfallScene')),
  consensus: lazy(() => import('./consensus/ConsensusRing')),
};

interface SceneContainerProps {
  activeScene: SceneName;
}

export const SceneContainer = memo(function SceneContainer({
  activeScene,
}: SceneContainerProps) {
  const SceneComponent = scenes[activeScene];

  return (
    <ErrorBoundary fallback={<SceneError scene={activeScene} />}>
      <Suspense fallback={<SceneLoading scene={activeScene} />}>
        <SceneComponent />
      </Suspense>
    </ErrorBoundary>
  );
});
```

### 10.1.4 URL Sync Hook

Bidirectional sync between the URL and the UI store. Scene routes update the store; store changes (from keyboard shortcuts) update the URL.

File: `apps/explorer/src/hooks/useRouteSync.ts`

```tsx
import { useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useUIStore } from '../stores/ui';
import type { SceneName } from '../stores/ui';

const SCENE_PATHS: Record<string, SceneName> = {
  '/terrain': 'terrain',
  '/constellation': 'constellation',
  '/waterfall': 'waterfall',
  '/consensus': 'consensus',
};

const SCENE_TO_PATH: Record<SceneName, string> = {
  terrain: '/terrain',
  constellation: '/constellation',
  waterfall: '/waterfall',
  consensus: '/consensus',
};

export function useRouteSync() {
  const location = useLocation();
  const navigate = useNavigate();
  const activeScene = useUIStore((s) => s.activeScene);
  const setActiveScene = useUIStore((s) => s.setActiveScene);

  // URL -> Store: when user navigates via browser bar or back/forward
  useEffect(() => {
    const sceneName = SCENE_PATHS[location.pathname];
    if (sceneName && sceneName !== activeScene) {
      setActiveScene(sceneName);
    }
  }, [location.pathname, activeScene, setActiveScene]);

  // Store -> URL: when scene changes via keyboard shortcut or programmatic switch
  useEffect(() => {
    const expectedPath = SCENE_TO_PATH[activeScene];
    const currentPath = location.pathname;

    // Only navigate if we are on a scene route (not a detail route)
    const isOnSceneRoute = Object.keys(SCENE_PATHS).includes(currentPath) ||
      currentPath === '/';
    if (isOnSceneRoute && currentPath !== expectedPath) {
      navigate(expectedPath, { replace: false });
    }
  }, [activeScene, location.pathname, navigate]);
}
```

### 10.1.5 Deep Linking

A deep link to `/block/123` must work on a fresh page load. The system:

1. Parses the route params via `useParams()`.
2. If the block is not in the LRU cache, fetches it from the RPC.
3. Shows a skeleton panel during the fetch.
4. Renders the detail panel once data arrives.
5. The background scene defaults to the last-used scene (stored in `localStorage`) or `terrain`.

```tsx
// apps/explorer/src/panels/DetailView/BlockDetail.tsx (deep link handling)
import { useParams, useNavigate } from 'react-router';
import { useEffect, useState } from 'react';
import { useChainStore } from '../../stores/chain';
import { httpClient } from '../../rpc/client';
import type { ChainBlock } from '../../rpc/types';

export default function BlockDetail() {
  const { numberOrHash } = useParams<{ numberOrHash: string }>();
  const navigate = useNavigate();
  const [block, setBlock] = useState<ChainBlock | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const cachedBlocks = useChainStore((s) => s.blocks);

  useEffect(() => {
    if (!numberOrHash) return;

    // Attempt cache lookup first
    const isHash = numberOrHash.startsWith('0x');
    const cached = isHash
      ? findBlockByHash(cachedBlocks, numberOrHash)
      : cachedBlocks.get(BigInt(numberOrHash));

    if (cached) {
      setBlock(cached);
      setLoading(false);
      return;
    }

    // RPC fallback
    setLoading(true);
    setError(null);

    const fetchBlock = async () => {
      try {
        const result = isHash
          ? await httpClient.getBlock({ blockHash: numberOrHash as `0x${string}` })
          : await httpClient.getBlock({ blockNumber: BigInt(numberOrHash) });

        if (!result) {
          setError(`Block ${numberOrHash} not found`);
          return;
        }

        setBlock(result as unknown as ChainBlock);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch block');
      } finally {
        setLoading(false);
      }
    };

    fetchBlock();
  }, [numberOrHash, cachedBlocks]);

  const handleClose = () => {
    navigate(-1);
  };

  const handlePreviousBlock = () => {
    if (!block) return;
    const prev = block.number - 1n;
    if (prev >= 0n) navigate(`/block/${prev}`);
  };

  const handleNextBlock = () => {
    if (!block) return;
    navigate(`/block/${block.number + 1n}`);
  };

  if (loading) return <BlockDetailSkeleton />;
  if (error) return <BlockDetailError message={error} onClose={handleClose} />;
  if (!block) return null;

  return (
    <BlockDetailPanel
      block={block}
      onClose={handleClose}
      onPrevious={handlePreviousBlock}
      onNext={handleNextBlock}
    />
  );
}
```

### 10.1.6 App Entry Point Update

File: `apps/explorer/src/App.tsx`

```tsx
import { BrowserRouter } from 'react-router';
import { AppRoutes } from './routes';
import { useRouteSync } from './hooks/useRouteSync';

function RouteSync() {
  useRouteSync();
  return null;
}

export function App() {
  return (
    <BrowserRouter>
      <RouteSync />
      <AppRoutes />
    </BrowserRouter>
  );
}
```

### 10.1.7 Verification Checklist

- [ ] `pnpm dev` -- navigate to `/` redirects to `/terrain`
- [ ] `/terrain`, `/constellation`, `/waterfall`, `/consensus` each render their scene
- [ ] `/block/1` opens the block detail panel over the current scene
- [ ] `/block/0x6d1f...` opens the same block by hash
- [ ] `/tx/0x1a2b...` opens transaction detail
- [ ] `/address/0xa3f2...` opens address detail
- [ ] Browser back button from `/block/1` returns to the previous scene route
- [ ] Browser forward button re-opens the detail panel
- [ ] Pasting `/block/1` in a new tab loads the block on a fresh page (deep link)
- [ ] Scene continues running (animating) behind the detail overlay at 30% opacity
- [ ] Closing the detail panel via Escape navigates back

---

## 10.2 Navigation State Machine

### 10.2.1 State Definition

File: `apps/explorer/src/stores/ui.ts` (additions)

```typescript
export type SceneName = 'terrain' | 'constellation' | 'waterfall' | 'consensus';

export type NavigationPhase =
  | 'idle'          // No entity selected, scene running normally
  | 'navigating'    // URL changed, determining data source
  | 'loading'       // RPC call in progress, showing skeleton
  | 'displaying';   // Entity data loaded, detail panel open

interface UIState {
  // Scene
  activeScene: SceneName;
  previousScene: SceneName | null;

  // Navigation
  navigationPhase: NavigationPhase;
  selectedEntity: EntityTarget | null;

  // Ambient
  ambientMode: boolean;
  ambientOpacity: number;        // 1.0 (investigative) or 0.2 (ambient)

  // Search
  searchOpen: boolean;
  searchQuery: string;

  // Pause
  paused: boolean;

  // Performance
  performanceTier: PerformanceTier;

  // Transition
  sceneTransition: SceneTransition | null;

  // Actions
  setActiveScene: (scene: SceneName) => void;
  setNavigationPhase: (phase: NavigationPhase) => void;
  selectEntity: (target: EntityTarget) => void;
  clearEntity: () => void;
  setAmbient: (active: boolean) => void;
  toggleSearch: () => void;
  togglePause: () => void;
  setPerformanceTier: (tier: PerformanceTier) => void;
}

export type EntityTarget =
  | { type: 'block'; id: string }     // number or hash string
  | { type: 'tx'; hash: string }
  | { type: 'address'; address: string };

export type PerformanceTier = 'full' | 'standard' | 'light' | 'mobile';

interface SceneTransition {
  from: SceneName;
  to: SceneName;
  progress: number;   // 0..1
  startedAt: number;
}
```

### 10.2.2 State Machine Implementation

```typescript
// apps/explorer/src/stores/ui.ts (store implementation)
import { create } from 'zustand';

export const useUIStore = create<UIState>((set, get) => ({
  activeScene: (localStorage.getItem('kora-scene') as SceneName) ?? 'terrain',
  previousScene: null,
  navigationPhase: 'idle',
  selectedEntity: null,
  ambientMode: false,
  ambientOpacity: 1,
  searchOpen: false,
  searchQuery: '',
  paused: false,
  performanceTier: 'full',
  sceneTransition: null,

  setActiveScene: (scene) => {
    const prev = get().activeScene;
    if (prev === scene) return;
    localStorage.setItem('kora-scene', scene);
    set({
      activeScene: scene,
      previousScene: prev,
      sceneTransition: {
        from: prev,
        to: scene,
        progress: 0,
        startedAt: performance.now(),
      },
    });
  },

  setNavigationPhase: (phase) => set({ navigationPhase: phase }),

  selectEntity: (target) => set({
    selectedEntity: target,
    navigationPhase: 'navigating',
    ambientMode: false,
    ambientOpacity: 1,
  }),

  clearEntity: () => set({
    selectedEntity: null,
    navigationPhase: 'idle',
  }),

  setAmbient: (active) => set({
    ambientMode: active,
    ambientOpacity: active ? 0.2 : 1,
  }),

  toggleSearch: () => {
    const current = get().searchOpen;
    set({
      searchOpen: !current,
      ambientMode: false,
      ambientOpacity: 1,
    });
  },

  togglePause: () => set((s) => ({ paused: !s.paused })),

  setPerformanceTier: (tier) => set({ performanceTier: tier }),
}));
```

### 10.2.3 Navigation Phase Transitions

```
                    ┌─────────────────────────┐
                    │         IDLE             │
                    │                          │
                    │  Scene running normally  │
                    │  No entity selected      │
                    │  No panel open           │
                    └────────────┬─────────────┘
                                 │
                    click entity / URL change / search select
                                 │
                    ┌────────────▼─────────────┐
                    │       NAVIGATING          │
                    │                          │
                    │  URL updated             │
                    │  Check LRU cache         │
                    │  If hit: skip to         │
                    │    DISPLAYING            │
                    │  If miss: go to LOADING  │
                    └────┬───────────┬─────────┘
                         │           │
                    cache hit    cache miss
                         │           │
                         │    ┌──────▼──────────┐
                         │    │    LOADING       │
                         │    │                  │
                         │    │  RPC call active │
                         │    │  Skeleton panel  │
                         │    │  visible         │
                         │    └──────┬───────────┘
                         │           │
                         │      data received / error
                         │           │
                    ┌────▼───────────▼─────────┐
                    │      DISPLAYING           │
                    │                          │
                    │  Detail panel open       │
                    │  Scene dimmed to 30%     │
                    │  Data rendered           │
                    └────────────┬─────────────┘
                                 │
                    close panel / Escape / navigate back
                                 │
                    ┌────────────▼─────────────┐
                    │         IDLE             │
                    └──────────────────────────┘
```

The hook that drives the state machine reads `useParams()` and transitions through phases:

```tsx
// apps/explorer/src/hooks/useNavigationPhase.ts
import { useEffect } from 'react';
import { useLocation } from 'react-router';
import { useUIStore } from '../stores/ui';

export function useNavigationPhase() {
  const location = useLocation();
  const setPhase = useUIStore((s) => s.setNavigationPhase);
  const selectEntity = useUIStore((s) => s.selectEntity);
  const clearEntity = useUIStore((s) => s.clearEntity);

  useEffect(() => {
    const path = location.pathname;

    if (path.startsWith('/block/')) {
      const id = path.slice('/block/'.length);
      selectEntity({ type: 'block', id });
    } else if (path.startsWith('/tx/')) {
      const hash = path.slice('/tx/'.length);
      selectEntity({ type: 'tx', hash });
    } else if (path.startsWith('/address/')) {
      const address = path.slice('/address/'.length);
      selectEntity({ type: 'address', address });
    } else {
      clearEntity();
    }
  }, [location.pathname, selectEntity, clearEntity]);
}
```

### 10.2.4 Verification Checklist

- [ ] Navigation phase starts as `idle` on page load
- [ ] Clicking an entity transitions: `idle` -> `navigating` -> `loading` (if cache miss) -> `displaying`
- [ ] Cached entities skip `loading` and go directly to `displaying`
- [ ] Pressing Escape or clicking close returns to `idle`
- [ ] The skeleton panel shows during the `loading` phase
- [ ] `navigationPhase` can be read from devtools via `useUIStore.getState().navigationPhase`

---

## 10.3 Keyboard Navigation

### 10.3.1 Shortcut Map

| Key | Action | Context |
|-----|--------|---------|
| `1` | Switch to Terrain scene | Any (not in input) |
| `2` | Switch to Constellation scene | Any (not in input) |
| `3` | Switch to Waterfall scene | Any (not in input) |
| `4` | Switch to Consensus scene | Any (not in input) |
| `Cmd/Ctrl+K` | Open search overlay | Any |
| `/` | Open search overlay | Any (not in input) |
| `Escape` | Close panel / search / exit ambient | Contextual priority |
| `ArrowLeft` | Previous block | BlockDetail open |
| `ArrowRight` | Next block | BlockDetail open |
| `Space` | Toggle pause (terrain scroll, waterfall) | Any (not in input) |
| `?` | Show keyboard shortcuts overlay | Any (not in input) |
| `F` | Toggle fullscreen | Any (not in input) |

### 10.3.2 Hook Implementation

File: `apps/explorer/src/hooks/useKeyboardShortcuts.ts`

```typescript
import { useEffect, useCallback } from 'react';
import { useNavigate, useLocation } from 'react-router';
import { useUIStore } from '../stores/ui';
import type { SceneName } from '../stores/ui';

const SCENE_KEYS: Record<string, SceneName> = {
  '1': 'terrain',
  '2': 'constellation',
  '3': 'waterfall',
  '4': 'consensus',
};

/** Returns true if the event target is an input-like element. */
function isInputFocused(event: KeyboardEvent): boolean {
  const target = event.target as HTMLElement;
  if (!target) return false;
  const tagName = target.tagName.toLowerCase();
  return (
    tagName === 'input' ||
    tagName === 'textarea' ||
    tagName === 'select' ||
    target.isContentEditable
  );
}

export function useKeyboardShortcuts() {
  const navigate = useNavigate();
  const location = useLocation();

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const { key, metaKey, ctrlKey } = event;
      const state = useUIStore.getState();

      // --- Cmd/Ctrl+K: always open search, even when input focused ---
      if ((metaKey || ctrlKey) && key === 'k') {
        event.preventDefault();
        state.toggleSearch();
        return;
      }

      // --- Skip all other shortcuts when input is focused ---
      if (isInputFocused(event)) return;

      // --- Escape: contextual close (priority order) ---
      if (key === 'Escape') {
        event.preventDefault();

        // 1. Close search if open
        if (state.searchOpen) {
          useUIStore.setState({ searchOpen: false, searchQuery: '' });
          return;
        }

        // 2. Close detail panel if open
        if (state.selectedEntity) {
          navigate(-1);
          return;
        }

        // 3. Exit ambient mode if active
        if (state.ambientMode) {
          state.setAmbient(false);
          return;
        }

        return;
      }

      // --- Scene switching: 1-4 ---
      if (key in SCENE_KEYS) {
        event.preventDefault();
        const scene = SCENE_KEYS[key]!;
        state.setActiveScene(scene);
        return;
      }

      // --- Search: / ---
      if (key === '/') {
        event.preventDefault();
        state.toggleSearch();
        return;
      }

      // --- Block navigation: arrow keys (only when BlockDetail open) ---
      if (key === 'ArrowLeft' || key === 'ArrowRight') {
        const path = location.pathname;
        if (!path.startsWith('/block/')) return;

        event.preventDefault();
        const current = path.slice('/block/'.length);
        // Only navigate by number, not by hash
        const num = parseBigIntSafe(current);
        if (num === null) return;

        const next = key === 'ArrowLeft' ? num - 1n : num + 1n;
        if (next >= 0n) {
          navigate(`/block/${next}`, { replace: true });
        }
        return;
      }

      // --- Space: toggle pause ---
      if (key === ' ') {
        event.preventDefault();
        state.togglePause();
        return;
      }

      // --- ?: show keyboard shortcuts overlay ---
      if (key === '?') {
        event.preventDefault();
        useUIStore.setState({ shortcutsOverlayOpen: true });
        return;
      }

      // --- F: toggle fullscreen ---
      if (key === 'f' || key === 'F') {
        event.preventDefault();
        toggleFullscreen();
        return;
      }
    },
    [navigate, location.pathname],
  );

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [handleKeyDown]);
}

function parseBigIntSafe(value: string): bigint | null {
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

function toggleFullscreen() {
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  } else {
    document.documentElement.requestFullscreen().catch(() => {});
  }
}
```

### 10.3.3 Keyboard Hints in Tooltips

Scene navigation buttons and interactive elements show keyboard hints. The hint is a small badge next to the button label.

```tsx
// apps/explorer/src/components/KeyHint.tsx
import styles from './KeyHint.module.css';

interface KeyHintProps {
  keys: string;   // e.g. "1", "Cmd+K", "Esc"
}

export function KeyHint({ keys }: KeyHintProps) {
  return (
    <kbd className={styles.hint} aria-hidden="true">
      {keys}
    </kbd>
  );
}
```

```css
/* apps/explorer/src/components/KeyHint.module.css */
.hint {
  display: inline-block;
  font-family: var(--rd-font-mono);
  font-size: 9px;
  letter-spacing: var(--rd-tracking-wide);
  text-transform: uppercase;
  color: var(--rd-text-dim);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid var(--rd-border);
  padding: 1px 4px;
  margin-left: var(--rd-space-xs);
  vertical-align: middle;
  line-height: 1.4;
}
```

### 10.3.4 Keyboard Shortcuts Overlay

```tsx
// apps/explorer/src/panels/ShortcutsOverlay.tsx
import { useUIStore } from '../stores/ui';
import { GlassPanel } from './GlassPanel';
import styles from './ShortcutsOverlay.module.css';

const SHORTCUTS = [
  { keys: '1 / 2 / 3 / 4', action: 'Switch scene' },
  { keys: 'Cmd+K  or  /', action: 'Open search' },
  { keys: 'Esc', action: 'Close panel / search' },
  { keys: '\u2190 / \u2192', action: 'Previous / next block' },
  { keys: 'Space', action: 'Toggle pause' },
  { keys: '?', action: 'This overlay' },
  { keys: 'F', action: 'Toggle fullscreen' },
] as const;

export function ShortcutsOverlay() {
  const open = useUIStore((s) => s.shortcutsOverlayOpen);
  const close = () => useUIStore.setState({ shortcutsOverlayOpen: false });

  if (!open) return null;

  return (
    <div
      className={styles.backdrop}
      onClick={close}
      onKeyDown={(e) => e.key === 'Escape' && close()}
      role="dialog"
      aria-label="Keyboard shortcuts"
      aria-modal="true"
    >
      <GlassPanel className={styles.panel} onClick={(e) => e.stopPropagation()}>
        <h2 className={styles.heading}>KEYBOARD SHORTCUTS</h2>
        <table className={styles.table}>
          <tbody>
            {SHORTCUTS.map((s) => (
              <tr key={s.keys}>
                <td className={styles.keys}>
                  <kbd>{s.keys}</kbd>
                </td>
                <td className={styles.action}>{s.action}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </GlassPanel>
    </div>
  );
}
```

### 10.3.5 Verification Checklist

- [ ] Pressing `1` navigates to `/terrain` and switches scene
- [ ] Pressing `2` navigates to `/constellation`
- [ ] Pressing `3` navigates to `/waterfall`
- [ ] Pressing `4` navigates to `/consensus`
- [ ] `Cmd+K` (Mac) or `Ctrl+K` (Win/Linux) opens search overlay
- [ ] `/` opens search overlay when no input is focused
- [ ] Typing in search input does not trigger `1-4` scene switches
- [ ] `Escape` closes search first, then detail panel, then exits ambient
- [ ] `ArrowLeft` / `ArrowRight` navigate blocks when BlockDetail is open
- [ ] `ArrowLeft` does nothing when block number is 0
- [ ] `Space` toggles pause state
- [ ] `?` opens the keyboard shortcuts overlay
- [ ] `F` toggles fullscreen
- [ ] No shortcuts fire when typing in a `<input>` or `<textarea>`

---

## 10.4 Ambient Mode

### 10.4.1 Behavior Spec

The explorer has two interaction postures:

- **Investigative** (default on any input): all panels visible, cursor visible, full interactivity.
- **Ambient** (after 30s idle): panels fade to 20% opacity, cursor hidden, scene takes full visual priority.

Transitions:
- To ambient: 30s idle timer fires, panels fade over 2s via CSS transition.
- To investigative: any `mousemove`, `mousedown`, `keydown`, `touchstart`, or `scroll` event immediately restores panels over 200ms.

### 10.4.2 Hook Implementation

File: `apps/explorer/src/hooks/useAmbientMode.ts`

```typescript
import { useEffect, useRef } from 'react';
import { useUIStore } from '../stores/ui';

const IDLE_TIMEOUT = 30_000;   // 30 seconds to enter ambient
const INTERACTION_EVENTS = [
  'mousemove',
  'mousedown',
  'keydown',
  'touchstart',
  'scroll',
  'wheel',
] as const;

export function useAmbientMode() {
  const setAmbient = useUIStore((s) => s.setAmbient);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => {
    const resetTimer = () => {
      // Any interaction exits ambient mode
      const state = useUIStore.getState();
      if (state.ambientMode) {
        setAmbient(false);
      }

      // Clear and restart the idle timer
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        // Only enter ambient if no detail panel or search is open
        const current = useUIStore.getState();
        if (!current.selectedEntity && !current.searchOpen) {
          setAmbient(true);
        }
      }, IDLE_TIMEOUT);
    };

    // Register all interaction listeners (passive for performance)
    for (const event of INTERACTION_EVENTS) {
      window.addEventListener(event, resetTimer, { passive: true });
    }

    // Start the initial timer
    resetTimer();

    return () => {
      for (const event of INTERACTION_EVENTS) {
        window.removeEventListener(event, resetTimer);
      }
      clearTimeout(timerRef.current);
    };
  }, [setAmbient]);
}
```

### 10.4.3 CSS Implementation

The ambient fade is handled entirely by CSS transitions, driven by a `data-ambient` attribute on the root `.explorer` element and the `--rd-ambient-opacity` custom property.

```css
/* apps/explorer/src/Explorer.module.css */

.explorer {
  position: fixed;
  inset: 0;
  overflow: hidden;
  background: var(--rd-void);
}

/* Panel layer opacity driven by ambient state */
.panelLayer {
  position: fixed;
  inset: 0;
  z-index: var(--rd-z-panels);
  pointer-events: none;
  /* Slow fade into ambient (2s), fast fade back to investigative (200ms) */
  transition: opacity 200ms var(--rd-ease-out);
}

.explorer[data-ambient="true"] .panelLayer {
  transition: opacity 2000ms ease-out;
}

/* Allow pointer events on actual panel children */
.panelLayer > * {
  pointer-events: auto;
}

/* Hide cursor in ambient mode */
.explorer[data-ambient="true"] {
  cursor: none;
}

/* Scene layer */
.sceneLayer {
  position: fixed;
  inset: 0;
  z-index: var(--rd-z-scene);
  transition: opacity 300ms var(--rd-ease-out);
}

/* Detail overlay */
.detailOverlay {
  position: fixed;
  top: 0;
  right: 0;
  bottom: 0;
  width: clamp(400px, 50vw, 720px);
  z-index: var(--rd-z-detail);
  overflow-y: auto;
  background: var(--rd-glass-bg);
  backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  -webkit-backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  border-left: 1px solid var(--rd-border);
}
```

### 10.4.4 Space Bar Manual Toggle

Space bar toggles pause mode, which also prevents the ambient timer from activating:

```typescript
// In useAmbientMode -- pause inhibits ambient
timerRef.current = setTimeout(() => {
  const current = useUIStore.getState();
  if (!current.selectedEntity && !current.searchOpen && !current.paused) {
    setAmbient(true);
  }
}, IDLE_TIMEOUT);
```

### 10.4.5 Verification Checklist

- [ ] After 30 seconds of no input, panels fade to 20% opacity over 2 seconds
- [ ] Any mouse movement immediately restores panels to 100% over 200ms
- [ ] Any keypress restores panels
- [ ] Touch events restore panels
- [ ] Cursor hides in ambient mode, reappears on interaction
- [ ] Ambient mode does not activate while a detail panel is open
- [ ] Ambient mode does not activate while search is open
- [ ] Ambient mode does not activate while paused (space bar)
- [ ] Scene keeps running and receiving data during ambient mode

---

## 10.5 Accessibility

### 10.5.1 Skip-to-Content Link

File: `apps/explorer/src/a11y/SkipToContent.tsx`

```tsx
import styles from './SkipToContent.module.css';

export function SkipToContent() {
  return (
    <a href="#main-content" className={styles.skip}>
      Skip to main content
    </a>
  );
}
```

```css
/* apps/explorer/src/a11y/SkipToContent.module.css */
.skip {
  position: fixed;
  top: -100%;
  left: var(--rd-space-md);
  z-index: 10000;
  padding: var(--rd-space-sm) var(--rd-space-md);
  background: var(--rd-void-surface);
  color: var(--rd-text-bright);
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-sm);
  text-decoration: none;
  border: 1px solid var(--rd-border-strong);
}

.skip:focus {
  top: var(--rd-space-md);
  outline: 2px solid var(--rd-rose);
  outline-offset: 2px;
}
```

### 10.5.2 Focus Management

Focus is trapped inside open panels using a focus trap. When a detail panel opens, focus moves to the panel's close button. When it closes, focus returns to the element that triggered the open.

File: `apps/explorer/src/a11y/useFocusTrap.ts`

```typescript
import { useEffect, useRef } from 'react';

/**
 * Traps tab focus within a container element.
 * Returns a ref to attach to the container.
 */
export function useFocusTrap<T extends HTMLElement>(active: boolean) {
  const containerRef = useRef<T>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!active || !containerRef.current) return;

    // Save the element that had focus before the trap activated
    previousFocusRef.current = document.activeElement as HTMLElement;

    const container = containerRef.current;
    const focusableSelector =
      'a[href], button:not([disabled]), input:not([disabled]), ' +
      'textarea:not([disabled]), select:not([disabled]), ' +
      '[tabindex]:not([tabindex="-1"])';

    // Focus the first focusable element in the container
    const firstFocusable = container.querySelector<HTMLElement>(focusableSelector);
    firstFocusable?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return;

      const focusableElements = container.querySelectorAll<HTMLElement>(focusableSelector);
      if (focusableElements.length === 0) return;

      const first = focusableElements[0]!;
      const last = focusableElements[focusableElements.length - 1]!;

      if (event.shiftKey) {
        // Shift+Tab: if focus is on first element, wrap to last
        if (document.activeElement === first) {
          event.preventDefault();
          last.focus();
        }
      } else {
        // Tab: if focus is on last element, wrap to first
        if (document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };

    container.addEventListener('keydown', handleKeyDown);

    return () => {
      container.removeEventListener('keydown', handleKeyDown);
      // Restore focus to the element that had it before
      previousFocusRef.current?.focus();
    };
  }, [active]);

  return containerRef;
}
```

### 10.5.3 ARIA Roles

| Element | Role | ARIA Attributes |
|---------|------|-----------------|
| Scene navigation tabs | `role="tablist"` | Each tab: `role="tab"`, `aria-selected`, `aria-controls` |
| Scene container | `role="tabpanel"` | `aria-labelledby` (linked to tab) |
| Detail panel | `role="dialog"` | `aria-label`, `aria-modal="true"` |
| Search overlay | `role="combobox"` | `aria-expanded`, `aria-controls` pointing to result list |
| Search results | `role="listbox"` | `aria-activedescendant` for highlighted result |
| Each search result | `role="option"` | `aria-selected` |
| Status LED | (decorative) | `aria-live="polite"` on the status text, not the dot |
| Glass panels | `role="region"` | `aria-label` describing content |
| Canvas scenes | `<canvas>` | `aria-hidden="true"` (visual-only, data in panels) |
| Block hash values | `<code>` | `aria-label` with full untruncated value |

Scene navigation with ARIA:

```tsx
// apps/explorer/src/panels/SceneNav.tsx
import { useUIStore } from '../stores/ui';
import { KeyHint } from '../components/KeyHint';
import type { SceneName } from '../stores/ui';

const SCENES: { id: SceneName; label: string; key: string }[] = [
  { id: 'terrain', label: 'Terrain', key: '1' },
  { id: 'constellation', label: 'Constellation', key: '2' },
  { id: 'waterfall', label: 'Waterfall', key: '3' },
  { id: 'consensus', label: 'Consensus', key: '4' },
];

export function SceneNav() {
  const activeScene = useUIStore((s) => s.activeScene);
  const setActiveScene = useUIStore((s) => s.setActiveScene);

  return (
    <nav role="tablist" aria-label="Scene navigation">
      {SCENES.map((scene) => (
        <button
          key={scene.id}
          role="tab"
          id={`tab-${scene.id}`}
          aria-selected={activeScene === scene.id}
          aria-controls={`panel-${scene.id}`}
          onClick={() => setActiveScene(scene.id)}
          tabIndex={activeScene === scene.id ? 0 : -1}
        >
          {scene.label}
          <KeyHint keys={scene.key} />
        </button>
      ))}
    </nav>
  );
}
```

### 10.5.4 Focus Ring Styling

File: `apps/explorer/src/design/a11y.css`

```css
/* Focus ring: visible only on keyboard navigation (not mouse click) */
:focus-visible {
  outline: 2px solid var(--rd-rose);
  outline-offset: 2px;
}

/* Remove default outline for mouse focus */
:focus:not(:focus-visible) {
  outline: none;
}

/*
 * Ensure focus ring has sufficient contrast.
 * --rd-rose (#aa7088) on --rd-void (#060608) = 4.8:1 (AA).
 * If the element is on a lighter surface, the ring still meets 3:1 minimum
 * for UI components.
 */
```

Import in `main.tsx` after the other design CSS:

```tsx
import './design/a11y.css';
```

### 10.5.5 Reduced Motion

File: `apps/explorer/src/design/reduced-motion.css`

```css
@media (prefers-reduced-motion: reduce) {
  /*
   * Disable all atmospheric animations.
   * The grain, vignette, scanlines are static -- only the rose wash
   * transitions, which are now instant.
   */
  .grain,
  body::before,
  body::after {
    animation: none !important;
  }

  /*
   * All transitions become instant.
   * Data still updates; only the animation is removed.
   */
  *,
  *::before,
  *::after {
    transition-duration: 0ms !important;
    animation-duration: 0ms !important;
    animation-iteration-count: 1 !important;
  }

  /*
   * Scene-specific reductions.
   * These classes are applied conditionally by each scene component
   * when it detects reduced motion via the useReducedMotion hook.
   */

  /* Terrain: no camera tracking scroll, show static snapshot */
  [data-scene="terrain"] canvas {
    animation: none !important;
  }

  /* Constellation: no particle animation, show static connections */
  [data-scene="constellation"] canvas {
    animation: none !important;
  }

  /* Waterfall: blocks appear instantly, no falling animation */
  [data-scene="waterfall"] .blockCard {
    transform: none !important;
    transition: none !important;
  }

  /* Consensus: ring updates instantly, no rotation or pulse */
  [data-scene="consensus"] canvas {
    animation: none !important;
  }

  /* Panel hover: no translateY movement */
  .panel:hover {
    transform: none !important;
  }

  /* LED pulse: static, no animation */
  .led {
    animation: none !important;
    opacity: 1 !important;
  }

  /* Scene transitions: instant crossfade */
  .sceneLayer {
    transition-duration: 0ms !important;
  }

  /* Detail panel: instant slide (no animation) */
  .detailOverlay {
    transition-duration: 0ms !important;
  }
}
```

The `useReducedMotion` hook for scene components:

```typescript
// apps/explorer/src/hooks/useReducedMotion.ts
import { useState, useEffect } from 'react';

export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => {
    if (typeof window === 'undefined') return false;
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  });

  useEffect(() => {
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const handler = (event: MediaQueryListEvent) => setReduced(event.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  return reduced;
}
```

Scene components use this to disable their animation loops:

```typescript
// Example usage in TerrainScene.tsx
const reducedMotion = useReducedMotion();

// In the animation loop:
if (reducedMotion) {
  // Render a single static frame showing the latest 30 tiles
  // No camera tracking, no tile scroll
  renderStaticSnapshot();
  return;
}
// Normal animation continues...
```

### 10.5.6 High Contrast Mode

ROSEDUST is always dark. But `prefers-contrast: more` increases the contrast of dim elements:

```css
/* apps/explorer/src/design/high-contrast.css */
@media (prefers-contrast: more) {
  :root {
    /* Boost dim text to meet AAA on void */
    --rd-text-dim:        #8a7a88;  /* was #6a5a68 (3.2:1) -> now ~5.0:1 */
    --rd-text-ghost:      #5a4a58;  /* was #3a303a (fails) -> now ~3.2:1 (AA large) */

    /* Stronger borders */
    --rd-border:          rgba(255, 255, 255, 0.14);  /* was 0.07 */
    --rd-border-strong:   rgba(255, 255, 255, 0.25);  /* was 0.14 */

    /* Glass panel slightly more opaque for readability */
    --rd-glass-bg:        rgba(8, 8, 12, 0.65);       /* was 0.45 */
  }
}
```

### 10.5.7 Screen Reader Labels

All interactive and informational elements carry descriptive labels:

```tsx
// Example: StatusBar with screen reader support
<div role="region" aria-label="Chain status">
  <span
    className={styles.led}
    role="img"
    aria-label={`Connection status: ${status}`}
  />
  <span aria-live="polite" className="sr-only">
    {status === 'connected'
      ? `Connected to Kora chain ${chainId}`
      : status === 'reconnecting'
        ? 'Reconnecting to Kora node...'
        : 'Disconnected from Kora node'}
  </span>
  <span>Block {blockNumber.toLocaleString()}</span>
</div>
```

Visually-hidden utility class:

```css
/* apps/explorer/src/design/a11y.css */
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border-width: 0;
}
```

### 10.5.8 WCAG Contrast Ratios

The ROSEDUST palette is designed to meet these ratios on `--rd-void` (#060608):

| Token | Hex | Ratio vs Void | WCAG Level |
|-------|-----|---------------|------------|
| `--rd-text` | #c8b8c0 | 9.4:1 | AAA |
| `--rd-text-dim` | #6a5a68 | 3.2:1 | AA Large |
| `--rd-bone-bright` | #d8c8a0 | 8.1:1 (on glass bg) | AAA |
| `--rd-rose` | #aa7088 | 4.8:1 | AA |
| `--rd-rose-bright` | #cc90a8 | 7.0:1 | AAA |
| `--rd-warning` | #c89a68 | 6.5:1 | AAA |
| `--rd-danger` | #cc5555 | 4.6:1 | AA |
| `--rd-success` | #7a8a78 | 4.2:1 | AA |
| `--rd-text-ghost` | #3a303a | 1.8:1 | Fails (decorative only) |

`--rd-text-ghost` intentionally fails contrast. Text using `text-ghost` must always have a visible alternative nearby or be purely decorative.

### 10.5.9 Verification Checklist

- [ ] Tab key traverses all interactive elements in logical order
- [ ] Focus ring (2px rose outline) visible on every focusable element when tabbing
- [ ] Focus ring not visible on mouse click
- [ ] Skip-to-content link appears on first Tab press, jumps to `#main-content`
- [ ] Scene nav tabs have `role="tablist"` / `role="tab"` / `aria-selected`
- [ ] Detail panels have `role="dialog"` and `aria-modal="true"`
- [ ] Focus trapped inside open detail panel (Tab does not escape)
- [ ] Focus returns to trigger element when panel closes
- [ ] Search results have `role="listbox"` / `role="option"` / `aria-activedescendant`
- [ ] Status LED text has `aria-live="polite"` (announces connection changes)
- [ ] Canvas elements have `aria-hidden="true"`
- [ ] `prefers-reduced-motion: reduce` disables all animations (verify with DevTools emulation)
- [ ] `prefers-contrast: more` increases dim text and border contrast
- [ ] Block hashes in `<code>` tags have `aria-label` with full untruncated value
- [ ] VoiceOver (Mac) or NVDA (Windows) can read all panel content

---

## 10.6 Touch Support (Mobile/Tablet)

### 10.6.1 Touch Targets

All interactive elements meet the 44x44px minimum touch target size:

```css
/* apps/explorer/src/design/touch.css */
@media (pointer: coarse) {
  /* Ensure minimum touch target size */
  button,
  a,
  [role="tab"],
  [role="option"],
  .touchTarget {
    min-width: 44px;
    min-height: 44px;
  }

  /* Increase spacing between adjacent targets */
  [role="tablist"] [role="tab"] + [role="tab"] {
    margin-left: var(--rd-space-sm);
  }

  /* Detail panels go full-screen on mobile */
  .detailOverlay {
    width: 100vw !important;
    left: 0;
    right: 0;
  }

  /* Mosaic tiles minimum size */
  .mosaicTile {
    min-width: 44px;
    min-height: 44px;
  }
}
```

### 10.6.2 Swipe Gesture Hook

File: `apps/explorer/src/hooks/useSwipeNavigation.ts`

```typescript
import { useEffect, useRef } from 'react';
import { useUIStore } from '../stores/ui';
import type { SceneName } from '../stores/ui';

const SCENE_ORDER: SceneName[] = ['terrain', 'constellation', 'waterfall', 'consensus'];
const SWIPE_THRESHOLD = 80;   // minimum px distance
const SWIPE_MAX_TIME = 300;   // max ms for a swipe gesture

interface TouchState {
  startX: number;
  startY: number;
  startTime: number;
}

export function useSwipeNavigation() {
  const touchRef = useRef<TouchState | null>(null);

  useEffect(() => {
    const handleTouchStart = (event: TouchEvent) => {
      const touch = event.touches[0];
      if (!touch) return;
      touchRef.current = {
        startX: touch.clientX,
        startY: touch.clientY,
        startTime: Date.now(),
      };
    };

    const handleTouchEnd = (event: TouchEvent) => {
      const start = touchRef.current;
      if (!start) return;
      touchRef.current = null;

      const touch = event.changedTouches[0];
      if (!touch) return;

      const elapsed = Date.now() - start.startTime;
      if (elapsed > SWIPE_MAX_TIME) return;

      const dx = touch.clientX - start.startX;
      const dy = touch.clientY - start.startY;

      // Must be predominantly horizontal
      if (Math.abs(dx) < SWIPE_THRESHOLD || Math.abs(dy) > Math.abs(dx) * 0.5) {
        return;
      }

      const state = useUIStore.getState();

      // Swipe to navigate between scenes
      const currentIndex = SCENE_ORDER.indexOf(state.activeScene);
      if (currentIndex === -1) return;

      if (dx < 0 && currentIndex < SCENE_ORDER.length - 1) {
        // Swipe left -> next scene
        state.setActiveScene(SCENE_ORDER[currentIndex + 1]!);
      } else if (dx > 0 && currentIndex > 0) {
        // Swipe right -> previous scene
        state.setActiveScene(SCENE_ORDER[currentIndex - 1]!);
      }
    };

    window.addEventListener('touchstart', handleTouchStart, { passive: true });
    window.addEventListener('touchend', handleTouchEnd, { passive: true });

    return () => {
      window.removeEventListener('touchstart', handleTouchStart);
      window.removeEventListener('touchend', handleTouchEnd);
    };
  }, []);
}
```

### 10.6.3 Pinch-to-Zoom

Terrain and constellation scenes support pinch-to-zoom. The gesture is handled inside each scene's canvas by listening to touch events and computing the distance between two touch points.

```typescript
// apps/explorer/src/hooks/usePinchZoom.ts
import { useEffect, useRef, useCallback } from 'react';

interface PinchState {
  initialDistance: number;
  initialZoom: number;
}

export function usePinchZoom(
  elementRef: React.RefObject<HTMLElement | null>,
  onZoom: (scale: number) => void,
  getCurrentZoom: () => number,
) {
  const pinchRef = useRef<PinchState | null>(null);

  const getDistance = useCallback((touches: TouchList): number => {
    if (touches.length < 2) return 0;
    const t0 = touches[0]!;
    const t1 = touches[1]!;
    return Math.hypot(t1.clientX - t0.clientX, t1.clientY - t0.clientY);
  }, []);

  useEffect(() => {
    const el = elementRef.current;
    if (!el) return;

    const handleTouchStart = (event: TouchEvent) => {
      if (event.touches.length !== 2) return;
      pinchRef.current = {
        initialDistance: getDistance(event.touches),
        initialZoom: getCurrentZoom(),
      };
    };

    const handleTouchMove = (event: TouchEvent) => {
      if (!pinchRef.current || event.touches.length !== 2) return;
      event.preventDefault();

      const currentDistance = getDistance(event.touches);
      const scale = currentDistance / pinchRef.current.initialDistance;
      onZoom(pinchRef.current.initialZoom * scale);
    };

    const handleTouchEnd = () => {
      pinchRef.current = null;
    };

    el.addEventListener('touchstart', handleTouchStart, { passive: true });
    el.addEventListener('touchmove', handleTouchMove, { passive: false });
    el.addEventListener('touchend', handleTouchEnd, { passive: true });

    return () => {
      el.removeEventListener('touchstart', handleTouchStart);
      el.removeEventListener('touchmove', handleTouchMove);
      el.removeEventListener('touchend', handleTouchEnd);
    };
  }, [elementRef, onZoom, getCurrentZoom, getDistance]);
}
```

### 10.6.4 Long Press for Context Menu

```typescript
// apps/explorer/src/hooks/useLongPress.ts
import { useRef, useCallback } from 'react';

const LONG_PRESS_DURATION = 500; // ms

export function useLongPress(
  onLongPress: (event: React.TouchEvent) => void,
  onClick?: (event: React.TouchEvent) => void,
) {
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  const isLongPressRef = useRef(false);

  const start = useCallback(
    (event: React.TouchEvent) => {
      isLongPressRef.current = false;
      timerRef.current = setTimeout(() => {
        isLongPressRef.current = true;
        onLongPress(event);
      }, LONG_PRESS_DURATION);
    },
    [onLongPress],
  );

  const end = useCallback(
    (event: React.TouchEvent) => {
      clearTimeout(timerRef.current);
      if (!isLongPressRef.current && onClick) {
        onClick(event);
      }
    },
    [onClick],
  );

  const cancel = useCallback(() => {
    clearTimeout(timerRef.current);
    isLongPressRef.current = false;
  }, []);

  return {
    onTouchStart: start,
    onTouchEnd: end,
    onTouchMove: cancel,
  };
}
```

### 10.6.5 Pull-Down Refresh on Waterfall

The waterfall scene supports pull-down to manually trigger a block refetch:

```typescript
// apps/explorer/src/hooks/usePullToRefresh.ts
import { useEffect, useRef } from 'react';

const PULL_THRESHOLD = 100; // px of pull distance to trigger refresh

export function usePullToRefresh(
  containerRef: React.RefObject<HTMLElement | null>,
  onRefresh: () => Promise<void>,
) {
  const startYRef = useRef(0);
  const pullingRef = useRef(false);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const handleTouchStart = (event: TouchEvent) => {
      // Only activate pull-to-refresh when scrolled to top
      if (el.scrollTop > 0) return;
      const touch = event.touches[0];
      if (!touch) return;
      startYRef.current = touch.clientY;
      pullingRef.current = true;
    };

    const handleTouchMove = (event: TouchEvent) => {
      if (!pullingRef.current) return;
      const touch = event.touches[0];
      if (!touch) return;
      const dy = touch.clientY - startYRef.current;

      if (dy > 0) {
        // Pulling down -- show visual indicator
        el.style.transform = `translateY(${Math.min(dy * 0.3, 60)}px)`;
      }
    };

    const handleTouchEnd = async (event: TouchEvent) => {
      if (!pullingRef.current) return;
      pullingRef.current = false;

      const touch = event.changedTouches[0];
      if (!touch) return;
      const dy = touch.clientY - startYRef.current;

      el.style.transition = 'transform 200ms ease-out';
      el.style.transform = 'translateY(0)';

      if (dy > PULL_THRESHOLD) {
        await onRefresh();
      }

      setTimeout(() => {
        el.style.transition = '';
      }, 200);
    };

    el.addEventListener('touchstart', handleTouchStart, { passive: true });
    el.addEventListener('touchmove', handleTouchMove, { passive: true });
    el.addEventListener('touchend', handleTouchEnd, { passive: true });

    return () => {
      el.removeEventListener('touchstart', handleTouchStart);
      el.removeEventListener('touchmove', handleTouchMove);
      el.removeEventListener('touchend', handleTouchEnd);
    };
  }, [containerRef, onRefresh]);
}
```

### 10.6.6 Mobile Behavior

On mobile (`pointer: coarse` or `window.innerWidth < 760`):

- No 3D scenes. Defaults to mosaic mode with hash-art tiles.
- Detail panels are full-screen (100vw, not the 50vw clamp).
- Swipe down closes a detail panel (detected via touch gesture).
- Long press shows a tooltip (equivalent to hover on desktop).
- Touch targets are 44x44px minimum.

### 10.6.7 Verification Checklist

- [ ] All buttons and tabs are at least 44x44px on touch devices
- [ ] Swipe left/right switches between scenes
- [ ] Pinch-to-zoom works on terrain and constellation canvases
- [ ] Long press on a block tile shows the tooltip (same as hover)
- [ ] Pull-down on waterfall triggers a data refresh
- [ ] Detail panels are full-screen on mobile
- [ ] Swipe down on a detail panel closes it

---

## 10.7 Performance Tier Auto-Detection

### 10.7.1 Detection Algorithm

File: `apps/explorer/src/hooks/usePerformanceTier.ts`

```typescript
import { useEffect } from 'react';
import { useUIStore } from '../stores/ui';
import type { PerformanceTier } from '../stores/ui';

export function usePerformanceTier() {
  const setTier = useUIStore((s) => s.setPerformanceTier);

  useEffect(() => {
    // Check for manual override first
    const override = localStorage.getItem('kora-perf-tier') as PerformanceTier | null;
    if (override && ['full', 'standard', 'light', 'mobile'].includes(override)) {
      setTier(override);
      return;
    }

    const detected = detectPerformanceTier();
    setTier(detected);
  }, [setTier]);
}

export function detectPerformanceTier(): PerformanceTier {
  // --- 1. Mobile detection ---
  if (/Mobi|Android/i.test(navigator.userAgent)) return 'mobile';
  if (window.innerWidth < 760) return 'mobile';

  // --- 2. WebGL2 availability ---
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2');
  if (!gl) return 'light';

  // --- 3. Device memory (Chrome/Edge only) ---
  const memory = (navigator as { deviceMemory?: number }).deviceMemory;
  if (memory !== undefined) {
    if (memory < 4) return 'light';
  }

  // --- 4. CPU core count ---
  const cores = navigator.hardwareConcurrency ?? 4;
  if (cores < 4) return 'light';

  // --- 5. Network quality (Chrome/Edge only) ---
  const connection = (navigator as { connection?: { effectiveType?: string } }).connection;
  if (connection?.effectiveType) {
    if (connection.effectiveType === '2g') return 'mobile';
    if (connection.effectiveType === '3g') return 'light';
  }

  // --- 6. GPU renderer heuristic ---
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  if (debugInfo) {
    const renderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) as string;

    // Integrated GPUs that are known to be slower
    const isLowEndIntegrated =
      /Intel.*(HD|UHD)\s*(Graphics)?\s*(4[0-9]{2}|5[0-5][0-9])/i.test(renderer) ||
      /Mali-[GT][0-9]{2}/i.test(renderer) ||
      /Adreno\s*(3|4[0-2])/i.test(renderer);

    if (isLowEndIntegrated) return 'standard';

    // All other integrated GPUs (Intel UHD 6xx+, Apple M-series, etc.) are fast enough
    const isIntegrated =
      /Intel|Mali|Adreno/i.test(renderer) && !/Arc/i.test(renderer);
    if (isIntegrated && memory !== undefined && memory < 8) return 'standard';
  }

  // Clean up
  gl.getExtension('WEBGL_lose_context')?.loseContext();

  // --- 7. Default: full tier ---
  return 'full';
}
```

### 10.7.2 Tier Configuration

File: `apps/explorer/src/config/tiers.ts`

```typescript
import type { PerformanceTier } from '../stores/ui';

export interface TierConfig {
  terrainRingSize: number;
  maxParticles: number;
  blockCacheSize: number;
  atmosphereLayers: boolean;
  audioAvailable: boolean;
  scenesEnabled: string[];
  gridResolution: number;         // terrain heightmap grid N x N
  particleTrails: boolean;
  hashArtSize: number;            // mosaic tile canvas size
  enableFog: boolean;
  enableBloom: boolean;
  maxVisibleWaterfallBlocks: number;
}

export const TIER_CONFIGS: Record<PerformanceTier, TierConfig> = {
  full: {
    terrainRingSize: 200,
    maxParticles: 16384,
    blockCacheSize: 500,
    atmosphereLayers: true,
    audioAvailable: true,
    scenesEnabled: ['terrain', 'constellation', 'waterfall', 'consensus'],
    gridResolution: 16,
    particleTrails: true,
    hashArtSize: 128,
    enableFog: true,
    enableBloom: true,
    maxVisibleWaterfallBlocks: 50,
  },
  standard: {
    terrainRingSize: 100,
    maxParticles: 8192,
    blockCacheSize: 300,
    atmosphereLayers: true,
    audioAvailable: true,
    scenesEnabled: ['terrain', 'waterfall'],
    gridResolution: 12,
    particleTrails: false,
    hashArtSize: 64,
    enableFog: true,
    enableBloom: false,
    maxVisibleWaterfallBlocks: 30,
  },
  light: {
    terrainRingSize: 0,
    maxParticles: 0,
    blockCacheSize: 200,
    atmosphereLayers: false,
    audioAvailable: false,
    scenesEnabled: [],
    gridResolution: 8,
    particleTrails: false,
    hashArtSize: 64,
    enableFog: false,
    enableBloom: false,
    maxVisibleWaterfallBlocks: 20,
  },
  mobile: {
    terrainRingSize: 0,
    maxParticles: 0,
    blockCacheSize: 100,
    atmosphereLayers: false,
    audioAvailable: false,
    scenesEnabled: [],
    gridResolution: 8,
    particleTrails: false,
    hashArtSize: 48,
    enableFog: false,
    enableBloom: false,
    maxVisibleWaterfallBlocks: 15,
  },
};

/** Get current tier config from store */
export function useTierConfig(): TierConfig {
  const tier = useUIStore((s) => s.performanceTier);
  return TIER_CONFIGS[tier];
}
```

### 10.7.3 Manual Override UI

```tsx
// apps/explorer/src/panels/SettingsPanel.tsx (performance tier section)
import { useUIStore } from '../stores/ui';
import type { PerformanceTier } from '../stores/ui';

const TIER_LABELS: Record<PerformanceTier, string> = {
  full: 'Full (dGPU, 8GB+ RAM)',
  standard: 'Standard (iGPU, 4GB+ RAM)',
  light: 'Light (no 3D)',
  mobile: 'Mobile',
};

export function PerformanceTierToggle() {
  const currentTier = useUIStore((s) => s.performanceTier);
  const setTier = useUIStore((s) => s.setPerformanceTier);

  const handleChange = (tier: PerformanceTier) => {
    setTier(tier);
    localStorage.setItem('kora-perf-tier', tier);
  };

  return (
    <fieldset>
      <legend className="label">PERFORMANCE TIER</legend>
      {(Object.keys(TIER_LABELS) as PerformanceTier[]).map((tier) => (
        <label key={tier}>
          <input
            type="radio"
            name="perf-tier"
            value={tier}
            checked={currentTier === tier}
            onChange={() => handleChange(tier)}
          />
          {TIER_LABELS[tier]}
        </label>
      ))}
    </fieldset>
  );
}
```

### 10.7.4 Verification Checklist

- [ ] On a desktop with dGPU and 8GB+ RAM, tier detects as `full`
- [ ] On a laptop with Intel UHD integrated GPU and 4GB RAM, tier detects as `standard`
- [ ] On a device without WebGL2, tier detects as `light`
- [ ] On mobile (Android/iOS), tier detects as `mobile`
- [ ] Setting `localStorage.setItem('kora-perf-tier', 'light')` overrides detection
- [ ] Changing the tier via the settings UI updates the store and persists to localStorage
- [ ] Scene components read the tier config and adjust particle count, grid resolution, etc.
- [ ] Atmospheric layers (grain, vignette, scanlines) are hidden on `light` and `mobile` tiers

---

## 10.8 Scene Transition

### 10.8.1 Transition Timing

From the design spec (`05-interaction.md`, section "Mode Switch"):

1. Current scene fades out: opacity 1 -> 0, 300ms
2. Intentional gap: 100ms (the void is visible -- a deliberate beat)
3. New scene fades in: opacity 0 -> 1, translateY 12px -> 0, 300ms expo ease

Total: 700ms. Deliberate, not instant. Not sluggish.

### 10.8.2 SceneTransitionManager

File: `apps/explorer/src/scenes/SceneTransitionManager.tsx`

```tsx
import { useState, useEffect, useRef } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { useUIStore } from '../stores/ui';
import { SceneContainer } from './SceneContainer';
import { useReducedMotion } from '../hooks/useReducedMotion';
import type { SceneName } from '../stores/ui';

const FADE_OUT_MS = 300;
const GAP_MS = 100;
const FADE_IN_MS = 300;

export function SceneTransitionManager() {
  const activeScene = useUIStore((s) => s.activeScene);
  const reducedMotion = useReducedMotion();
  const [displayedScene, setDisplayedScene] = useState(activeScene);
  const [phase, setPhase] = useState<'visible' | 'fading-out' | 'gap' | 'fading-in'>('visible');
  const cancelRef = useRef(false);

  useEffect(() => {
    if (activeScene === displayedScene) return;

    // Cancel any in-progress transition
    cancelRef.current = true;
    cancelRef.current = false;

    if (reducedMotion) {
      // Instant transition for reduced motion
      setDisplayedScene(activeScene);
      setPhase('visible');
      return;
    }

    // Phase 1: fade out current scene
    setPhase('fading-out');

    const fadeOutTimer = setTimeout(() => {
      if (cancelRef.current) return;

      // Phase 2: gap (void visible)
      setPhase('gap');

      const gapTimer = setTimeout(() => {
        if (cancelRef.current) return;

        // Phase 3: swap scene and fade in
        setDisplayedScene(activeScene);
        setPhase('fading-in');

        const fadeInTimer = setTimeout(() => {
          if (cancelRef.current) return;
          setPhase('visible');
        }, FADE_IN_MS);

        return () => clearTimeout(fadeInTimer);
      }, GAP_MS);

      return () => clearTimeout(gapTimer);
    }, FADE_OUT_MS);

    return () => {
      cancelRef.current = true;
      clearTimeout(fadeOutTimer);
    };
  }, [activeScene, displayedScene, reducedMotion]);

  const opacity =
    phase === 'visible' ? 1 :
    phase === 'fading-out' ? 0 :
    phase === 'gap' ? 0 :
    0; // fading-in starts at 0, CSS transition handles the rest

  const translateY = phase === 'fading-in' ? 12 : 0;

  return (
    <div
      className="sceneTransition"
      style={{
        opacity: phase === 'fading-in' || phase === 'visible' ? 1 : 0,
        transform: `translateY(${translateY}px)`,
        transition: reducedMotion
          ? 'none'
          : `opacity ${FADE_IN_MS}ms cubic-bezier(0.16, 1, 0.3, 1), ` +
            `transform ${FADE_IN_MS}ms cubic-bezier(0.16, 1, 0.3, 1)`,
      }}
    >
      <SceneContainer activeScene={displayedScene} />
    </div>
  );
}
```

### 10.8.3 Rapid Switch Cancellation

When the user rapidly presses `1`, `2`, `3` in quick succession, in-progress transitions must be cancelled. The `cancelRef` pattern above handles this: setting `cancelRef.current = true` prevents stale timeout callbacks from executing. The scene immediately jumps to the latest target.

### 10.8.4 Shared State Preservation

During a transition, scene data keeps flowing. The Zustand chain store and the EventBus continue operating. Both the outgoing and incoming scenes remain subscribed to the bus. The outgoing scene's subscriptions are only cleaned up when it fully unmounts (which only happens on tier downgrade, not on transitions).

### 10.8.5 Detail Panel Transition

Detail panels use Framer Motion for slide-in/slide-out, as specified in the design doc:

1. Clicked element pulses once (rose-glow flash, 200ms)
2. Background scene dims to 30% opacity (300ms)
3. Detail panel slides in from right (translateX 100% -> 0, 400ms expo ease)

Closing reverses:
1. Panel slides out (translateX 0 -> 100%, 300ms)
2. Background scene restores to 100% (300ms, overlapping)

```tsx
// Detail panel animation variants (used in Explorer.tsx AnimatePresence)
const detailVariants = {
  initial: { opacity: 0, x: '100%' },
  animate: { opacity: 1, x: 0 },
  exit: { opacity: 0, x: '100%' },
};

const detailTransition = {
  duration: 0.4,
  ease: [0.16, 1, 0.3, 1], // expo ease-out (--rd-ease-out)
};
```

### 10.8.6 Verification Checklist

- [ ] Switching from terrain to constellation shows: 300ms fade-out, 100ms void, 300ms fade-in
- [ ] The incoming scene slides up 12px during fade-in (expo ease)
- [ ] Rapid switching (1-2-3-4 fast) does not leave stale scenes visible
- [ ] Data continues flowing to scenes during transition (no missed blocks)
- [ ] Detail panel slides in from the right over 400ms
- [ ] Detail panel close slides out to the right over 300ms
- [ ] Scene dims to 30% when detail panel opens, restores to 100% on close
- [ ] Reduced motion: all transitions are instant (no animation)

---

## 10.9 Complete File Manifest

Files created or modified by this implementation document:

| File | Action | Section |
|------|--------|---------|
| `apps/explorer/src/routes.tsx` | Replace | 10.1.1 |
| `apps/explorer/src/Explorer.tsx` | Create | 10.1.2 |
| `apps/explorer/src/Explorer.module.css` | Create | 10.4.3 |
| `apps/explorer/src/scenes/SceneContainer.tsx` | Create | 10.1.3 |
| `apps/explorer/src/hooks/useRouteSync.ts` | Create | 10.1.4 |
| `apps/explorer/src/App.tsx` | Modify | 10.1.6 |
| `apps/explorer/src/stores/ui.ts` | Modify | 10.2.1, 10.2.2 |
| `apps/explorer/src/hooks/useNavigationPhase.ts` | Create | 10.2.3 |
| `apps/explorer/src/hooks/useKeyboardShortcuts.ts` | Create | 10.3.2 |
| `apps/explorer/src/components/KeyHint.tsx` | Create | 10.3.3 |
| `apps/explorer/src/components/KeyHint.module.css` | Create | 10.3.3 |
| `apps/explorer/src/panels/ShortcutsOverlay.tsx` | Create | 10.3.4 |
| `apps/explorer/src/hooks/useAmbientMode.ts` | Create | 10.4.2 |
| `apps/explorer/src/a11y/SkipToContent.tsx` | Create | 10.5.1 |
| `apps/explorer/src/a11y/SkipToContent.module.css` | Create | 10.5.1 |
| `apps/explorer/src/a11y/useFocusTrap.ts` | Create | 10.5.2 |
| `apps/explorer/src/design/a11y.css` | Create | 10.5.4 |
| `apps/explorer/src/design/reduced-motion.css` | Create | 10.5.5 |
| `apps/explorer/src/hooks/useReducedMotion.ts` | Create | 10.5.5 |
| `apps/explorer/src/design/high-contrast.css` | Create | 10.5.6 |
| `apps/explorer/src/panels/SceneNav.tsx` | Create | 10.5.3 |
| `apps/explorer/src/design/touch.css` | Create | 10.6.1 |
| `apps/explorer/src/hooks/useSwipeNavigation.ts` | Create | 10.6.2 |
| `apps/explorer/src/hooks/usePinchZoom.ts` | Create | 10.6.3 |
| `apps/explorer/src/hooks/useLongPress.ts` | Create | 10.6.4 |
| `apps/explorer/src/hooks/usePullToRefresh.ts` | Create | 10.6.5 |
| `apps/explorer/src/hooks/usePerformanceTier.ts` | Create | 10.7.1 |
| `apps/explorer/src/config/tiers.ts` | Create | 10.7.2 |
| `apps/explorer/src/panels/SettingsPanel.tsx` | Create | 10.7.3 |
| `apps/explorer/src/scenes/SceneTransitionManager.tsx` | Create | 10.8.2 |

### CSS Import Order in `main.tsx`

```tsx
// Design system CSS -- order matters
import './design/reset.css';
import './design/tokens.css';
import './design/typography.css';
import './design/atmosphere.css';
import './design/a11y.css';
import './design/reduced-motion.css';
import './design/high-contrast.css';
import './design/touch.css';
```

---

## 10.10 Integration Verification

End-to-end checks that exercise the full interaction layer working together:

- [ ] **Route resolution:** every route in `routes.tsx` renders the correct component
- [ ] **Deep link:** paste `/block/1` in a new browser tab, see block detail load from RPC
- [ ] **Browser navigation:** back/forward buttons correctly navigate between scenes and entity views
- [ ] **Scene preservation:** opening `/block/1` keeps the terrain scene running at 30% opacity behind the panel
- [ ] **Keyboard shortcuts:** `1`-`4` switch scenes, `Cmd+K` opens search, `Escape` closes contextually
- [ ] **Block stepping:** `ArrowLeft`/`ArrowRight` step through blocks when BlockDetail is open
- [ ] **Ambient mode:** 30 seconds of no input fades panels to 20% opacity; any input restores them
- [ ] **Space toggle:** space bar pauses scene auto-scroll and inhibits ambient mode
- [ ] **Focus ring:** tabbing through the UI shows a 2px rose outline on each focusable element
- [ ] **Focus trap:** tab focus stays within an open detail panel; closing returns focus to trigger
- [ ] **Screen reader:** VoiceOver/NVDA can navigate panels, read block data, and announce status changes
- [ ] **Reduced motion:** enabling `prefers-reduced-motion: reduce` in DevTools disables all animation
- [ ] **High contrast:** enabling `prefers-contrast: more` in DevTools increases dim text and border contrast
- [ ] **Performance tier:** detection returns the correct tier for the current hardware
- [ ] **Manual override:** setting tier via settings panel persists across page reloads
- [ ] **Scene transition:** switching scenes shows the 300ms/100ms/300ms crossfade sequence
- [ ] **Rapid switching:** pressing `1-2-3-4` rapidly does not leave stale scenes or break state
- [ ] **Touch navigation:** swipe left/right switches scenes on a touch device
- [ ] **Pinch zoom:** two-finger pinch zooms terrain and constellation on tablet
- [ ] **Mobile detail:** detail panels render full-screen on narrow viewports
