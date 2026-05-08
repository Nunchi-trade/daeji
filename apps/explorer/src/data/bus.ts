import mitt from 'mitt';
import type { BusEvents } from './types';

// ================================================================
// SINGLETON BUS INSTANCE
// ================================================================

export const bus = mitt<BusEvents>();

// ================================================================
// DEVELOPMENT HELPERS
// ================================================================

if (import.meta.env.DEV) {
  bus.on('*', (type, payload) => {
    console.debug(`[bus] ${type as string}`, payload);
  });
}
