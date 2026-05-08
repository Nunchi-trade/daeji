import mitt from 'mitt';
// ================================================================
// SINGLETON BUS INSTANCE
// ================================================================
export const bus = mitt();
// ================================================================
// DEVELOPMENT HELPERS
// ================================================================
if (import.meta.env.DEV) {
    bus.on('*', (type, payload) => {
        console.debug(`[bus] ${type}`, payload);
    });
}
