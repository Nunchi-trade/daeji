import { Routes, Route, Navigate } from 'react-router';
import { Suspense, lazy } from 'react';
import { Explorer } from './Explorer';

// Lazy-loaded detail views
const BlockDetail = lazy(() =>
  import('./panels/DetailView/BlockDetail').then((m) => ({
    default: m.BlockDetail,
  })),
);
const TxDetail = lazy(() =>
  import('./panels/DetailView/TxDetail').then((m) => ({
    default: m.TxDetail,
  })),
);
const AddressDetail = lazy(() =>
  import('./panels/DetailView/AddressDetail').then((m) => ({
    default: m.AddressDetail,
  })),
);

function DetailLoading() {
  return (
    <div
      style={{
        padding: 'var(--rd-space-lg)',
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: 'var(--rd-text-dim)',
        letterSpacing: 'var(--rd-tracking-label)',
        textTransform: 'uppercase',
      }}
    >
      Loading...
    </div>
  );
}

export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<Explorer />}>
        {/* Default redirect to terrain scene */}
        <Route index element={<Navigate to="/terrain" replace />} />

        {/* Scene routes -- these just set activeScene via useRouteSync */}
        <Route path="terrain" element={null} />
        <Route path="constellation" element={null} />
        <Route path="waterfall" element={null} />
        <Route path="consensus" element={null} />

        {/* Entity detail routes -- rendered as overlays on the current scene */}
        <Route
          path="block/:numberOrHash"
          element={
            <Suspense fallback={<DetailLoading />}>
              <BlockDetail />
            </Suspense>
          }
        />
        <Route
          path="tx/:hash"
          element={
            <Suspense fallback={<DetailLoading />}>
              <TxDetail />
            </Suspense>
          }
        />
        <Route
          path="address/:address"
          element={
            <Suspense fallback={<DetailLoading />}>
              <AddressDetail />
            </Suspense>
          }
        />
      </Route>
    </Routes>
  );
}
