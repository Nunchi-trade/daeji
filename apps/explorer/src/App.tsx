import { BrowserRouter } from 'react-router';
import { AppRoutes } from './routes';
import { useRouteSync } from './hooks/useRouteSync';
import { initializeStores } from './data/store';

// Wire bus events to stores on import
initializeStores();

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
