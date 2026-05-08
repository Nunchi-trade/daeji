import { Suspense, lazy, memo } from 'react';
import type { SceneName } from '@/data/types';

// Lazy-loaded scenes (code-split per scene)
const scenes: Record<SceneName, React.LazyExoticComponent<React.ComponentType>> = {
  terrain: lazy(() => import('./terrain/TerrainScene')),
  constellation: lazy(() => import('./constellation/ConstellationCanvas')),
  waterfall: lazy(() => import('./waterfall/WaterfallScene')),
  consensus: lazy(() => import('./consensus/ConsensusScene')),
};

function SceneLoading({ scene }: { scene: string }) {
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-text-dim)',
        letterSpacing: 'var(--rd-tracking-label)',
        textTransform: 'uppercase',
      }}
    >
      Loading {scene}
    </div>
  );
}

function SceneError({ scene }: { scene: string }) {
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-danger, #ff4444)',
        letterSpacing: 'var(--rd-tracking-label)',
        textTransform: 'uppercase',
      }}
    >
      Failed to load {scene}
    </div>
  );
}

// Minimal error boundary for scene rendering
import { Component } from 'react';
import type { ReactNode, ErrorInfo } from 'react';

interface ErrorBoundaryProps {
  fallback: ReactNode;
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
}

class SceneErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(): ErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[SceneManager] Error rendering scene:', error, info);
  }

  render() {
    if (this.state.hasError) return this.props.fallback;
    return this.props.children;
  }
}

interface SceneManagerProps {
  activeScene: SceneName;
}

export const SceneManager = memo(function SceneManager({
  activeScene,
}: SceneManagerProps) {
  const SceneComponent = scenes[activeScene];

  return (
    <SceneErrorBoundary fallback={<SceneError scene={activeScene} />}>
      <Suspense fallback={<SceneLoading scene={activeScene} />}>
        <SceneComponent />
      </Suspense>
    </SceneErrorBoundary>
  );
});
