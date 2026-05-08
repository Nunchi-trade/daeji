import { client } from './rpc';
import { bus } from './bus';
import type { ConnectionState } from './types';

// ================================================================
// CONFIGURATION
// ================================================================

const HEALTH_CHECK_INTERVAL = 5_000; // 5s between health pings
const INITIAL_RECONNECT_DELAY = 1_000; // 1s first retry
const MAX_RECONNECT_DELAY = 30_000; // 30s ceiling
const RECONNECT_BACKOFF_FACTOR = 2; // double each attempt
const JITTER_FACTOR = 0.2; // +/- 20% randomization

// ================================================================
// CONNECTION MANAGER
// ================================================================

export class ConnectionManager {
  private state: ConnectionState = 'disconnected';
  private reconnectAttempt = 0;
  private healthCheckTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;

  // -- Callbacks --
  onConnect: (() => void) | null = null;
  onDisconnect: (() => void) | null = null;
  onReconnect: ((attempt: number) => void) | null = null;

  // ================================================================
  // PUBLIC API
  // ================================================================

  /** Current connection state */
  getState(): ConnectionState {
    return this.state;
  }

  /** Initiate connection. Resolves when first health check passes. */
  async connect(): Promise<void> {
    if (this.destroyed) return;
    this.setState('connecting');

    try {
      await client.getBlockNumber();
      this.setState('connected');
      this.reconnectAttempt = 0;
      this.onConnect?.();
      this.startHealthCheck();
    } catch (err) {
      console.warn('[ConnectionManager] Initial connection failed:', err);
      this.scheduleReconnect();
    }
  }

  /** Gracefully shut down. No reconnection after this. */
  disconnect(): void {
    this.destroyed = true;
    this.stopHealthCheck();
    this.clearReconnectTimer();
    this.setState('disconnected');
    this.onDisconnect?.();
  }

  // ================================================================
  // HEALTH CHECK
  // ================================================================

  private startHealthCheck(): void {
    this.stopHealthCheck();

    this.healthCheckTimer = setInterval(async () => {
      if (this.destroyed) return;

      try {
        await client.getBlockNumber();

        // If we were reconnecting, we are now recovered
        if (this.state === 'reconnecting') {
          this.setState('connected');
          this.reconnectAttempt = 0;
          this.onConnect?.();
        }
      } catch {
        if (this.state === 'connected') {
          // First failure: transition to reconnecting
          this.scheduleReconnect();
        }
      }
    }, HEALTH_CHECK_INTERVAL);
  }

  private stopHealthCheck(): void {
    if (this.healthCheckTimer !== null) {
      clearInterval(this.healthCheckTimer);
      this.healthCheckTimer = null;
    }
  }

  // ================================================================
  // RECONNECTION (exponential backoff with jitter)
  // ================================================================

  private scheduleReconnect(): void {
    if (this.destroyed) return;

    this.setState('reconnecting');

    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s...
    const baseDelay = Math.min(
      MAX_RECONNECT_DELAY,
      INITIAL_RECONNECT_DELAY *
        Math.pow(RECONNECT_BACKOFF_FACTOR, this.reconnectAttempt),
    );

    // Jitter: randomize by +/- 20% to prevent thundering herd
    const jitter = baseDelay * JITTER_FACTOR * (Math.random() * 2 - 1);
    const delay = Math.max(0, Math.round(baseDelay + jitter));

    this.reconnectAttempt++;
    this.onReconnect?.(this.reconnectAttempt);

    console.info(
      `[ConnectionManager] Reconnect attempt ${this.reconnectAttempt} in ${delay}ms`,
    );

    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(async () => {
      if (this.destroyed) return;

      try {
        await client.getBlockNumber();
        this.setState('connected');
        this.reconnectAttempt = 0;
        this.onConnect?.();
        this.startHealthCheck();
      } catch {
        // Still failing: schedule next attempt
        this.scheduleReconnect();
      }
    }, delay);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  // ================================================================
  // STATE TRANSITIONS
  // ================================================================

  private setState(next: ConnectionState): void {
    if (this.state === next) return;
    const prev = this.state;
    this.state = next;

    console.debug(`[ConnectionManager] ${prev} -> ${next}`);
    bus.emit('connection:state', next);
  }
}
