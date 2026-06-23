export const PLATFORM_NAME = 'Collar';
export const PLUGIN_NAME = 'homebridge-collar';
/**
 * Default polling cadence. We rely on SSE for fast updates; polling is a
 * safety net that catches missed events after a network blip, so an interval
 * measured in minutes is fine.
 */
export const DEFAULT_POLL_INTERVAL_SECONDS = 60;
/** Backoff bounds for reconnecting the SSE event stream. */
export const SSE_RECONNECT_MIN_MS = 2_000;
export const SSE_RECONNECT_MAX_MS = 60_000;
