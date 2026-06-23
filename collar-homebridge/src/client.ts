import type { Logger } from 'homebridge';
import type {
  HomeKitEvent,
  HomeKitSetRequest,
  HomeKitSetResponse,
  HomeKitSwitchState,
} from './types';

/**
 * Thin HTTP + SSE client for the Collar server's /api/homekit surface.
 *
 * All requests carry `Authorization: Bearer <apiKey>`. The server treats this
 * key as dedicated — it never grants user-level access.
 */
export class CollarClient {
  private readonly baseUrl: string;

  constructor(
    serverUrl: string,
    private readonly apiKey: string,
    private readonly log: Logger,
  ) {
    // Normalise: strip trailing slash, then append /api/homekit.
    this.baseUrl = serverUrl.replace(/\/+$/, '') + '/api/homekit';
  }

  async listSwitches(): Promise<HomeKitSwitchState[]> {
    return this.request<HomeKitSwitchState[]>('GET', '/switches');
  }

  async setSwitch(id: string, on: boolean): Promise<HomeKitSetResponse> {
    const body: HomeKitSetRequest = { on };
    return this.request<HomeKitSetResponse>(
      'POST',
      `/switches/${encodeURIComponent(id)}/set`,
      body,
    );
  }

  /**
   * Open the SSE event stream. Returns an `AbortController` whose `abort()`
   * closes the connection. Events are delivered via the `onEvent` callback;
   * lifecycle changes (open/close/error) via the others.
   *
   * This is a long-lived call that resolves once the stream is established
   * and then writes events into the callbacks until aborted.
   */
  async openEventStream(handlers: {
    onEvent: (event: HomeKitEvent) => void;
    onOpen: () => void;
    onClose: (reason: string) => void;
    signal: AbortSignal;
  }): Promise<void> {
    const url = this.baseUrl + '/events';

    let response: Response;
    try {
      response = await fetch(url, {
        method: 'GET',
        headers: {
          Authorization: `Bearer ${this.apiKey}`,
          Accept: 'text/event-stream',
          'Cache-Control': 'no-cache',
        },
        signal: handlers.signal,
      });
    } catch (err) {
      if ((err as Error).name === 'AbortError') {
        handlers.onClose('aborted');
        return;
      }
      handlers.onClose(`fetch error: ${(err as Error).message}`);
      return;
    }

    if (!response.ok) {
      handlers.onClose(`HTTP ${response.status} ${response.statusText}`);
      return;
    }
    if (!response.body) {
      handlers.onClose('response has no body');
      return;
    }

    handlers.onOpen();

    const reader = response.body.getReader();
    const decoder = new TextDecoder('utf-8');
    let buffer = '';

    try {
      // eslint-disable-next-line no-constant-condition
      while (true) {
        const { value, done } = await reader.read();
        if (done) {
          handlers.onClose('stream ended');
          return;
        }
        buffer += decoder.decode(value, { stream: true });

        // SSE event blocks are separated by blank lines (\n\n).
        let sep: number;
        while ((sep = buffer.indexOf('\n\n')) !== -1) {
          const rawBlock = buffer.slice(0, sep);
          buffer = buffer.slice(sep + 2);
          const parsed = parseSseBlock(rawBlock);
          if (!parsed) {
            continue;
          }
          // Server sends comment-style keepalives via axum's KeepAlive; those
          // arrive with no `data:` line and parseSseBlock returns null.
          try {
            const event = decodeEvent(parsed.event, parsed.data);
            if (event) {
              handlers.onEvent(event);
            }
          } catch (err) {
            this.log.debug(`Discarding malformed SSE event: ${(err as Error).message}`);
          }
        }
      }
    } catch (err) {
      if ((err as Error).name === 'AbortError') {
        handlers.onClose('aborted');
        return;
      }
      handlers.onClose(`read error: ${(err as Error).message}`);
    }
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const url = this.baseUrl + path;
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.apiKey}`,
      Accept: 'application/json',
    };
    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
    }

    let response: Response;
    try {
      response = await fetch(url, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch (err) {
      this.log.debug(`HTTP ${method} ${url} threw: ${(err as Error).message}`);
      throw new Error(`Network error talking to Collar: ${(err as Error).message}`);
    }

    if (!response.ok) {
      const text = await response.text().catch(() => '');
      throw new Error(
        `Collar API ${method} ${path} failed: ${response.status} ${response.statusText} ${text}`,
      );
    }

    return (await response.json()) as T;
  }
}

interface ParsedSseBlock {
  event: string;
  data: string;
}

function parseSseBlock(block: string): ParsedSseBlock | null {
  let event = 'message';
  const dataLines: string[] = [];
  for (const rawLine of block.split('\n')) {
    const line = rawLine.replace(/\r$/, '');
    if (!line || line.startsWith(':')) {
      // Comment / keepalive. Skip.
      continue;
    }
    const colonIdx = line.indexOf(':');
    if (colonIdx === -1) {
      continue;
    }
    const field = line.slice(0, colonIdx);
    // Per spec, single optional space after colon is stripped.
    let value = line.slice(colonIdx + 1);
    if (value.startsWith(' ')) {
      value = value.slice(1);
    }
    if (field === 'event') {
      event = value;
    } else if (field === 'data') {
      dataLines.push(value);
    }
  }
  if (dataLines.length === 0) {
    return null;
  }
  return { event, data: dataLines.join('\n') };
}

function decodeEvent(type: string, data: string): HomeKitEvent | null {
  if (type === 'switch_updated') {
    const state = JSON.parse(data) as HomeKitSwitchState;
    return { type: 'switch_updated', state };
  }
  if (type === 'heartbeat') {
    return { type: 'heartbeat' };
  }
  return null;
}
