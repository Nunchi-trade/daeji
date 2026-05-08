import { useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useUIStore } from '@/data/store';
import type { SceneName } from '@/data/types';

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
  const setScene = useUIStore((s) => s.setScene);

  // URL -> Store: when user navigates via browser bar or back/forward
  useEffect(() => {
    const sceneName = SCENE_PATHS[location.pathname];
    if (sceneName && sceneName !== activeScene) {
      setScene(sceneName);
    }
  }, [location.pathname, activeScene, setScene]);

  // Store -> URL: when scene changes via keyboard shortcut or programmatic switch
  useEffect(() => {
    const expectedPath = SCENE_TO_PATH[activeScene];
    const currentPath = location.pathname;

    // Only navigate if we are on a scene route (not a detail route)
    const isOnSceneRoute =
      Object.keys(SCENE_PATHS).includes(currentPath) || currentPath === '/';
    if (isOnSceneRoute && currentPath !== expectedPath) {
      navigate(expectedPath, { replace: false });
    }
  }, [activeScene, location.pathname, navigate]);
}
