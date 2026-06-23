import type {
  API,
  Characteristic,
  DynamicPlatformPlugin,
  Logger,
  PlatformAccessory,
  PlatformConfig,
  Service,
} from 'homebridge';

import { CollarClient } from './client';
import { CollarDeviceAccessory } from './device-accessory';
import {
  DEFAULT_POLL_INTERVAL_SECONDS,
  PLATFORM_NAME,
  PLUGIN_NAME,
  SSE_RECONNECT_MAX_MS,
  SSE_RECONNECT_MIN_MS,
} from './settings';
import type { CollarPlatformConfig, HomeKitEvent, HomeKitSwitchState } from './types';

/**
 * Dynamic platform. Each Collar *device* becomes one HomeKit accessory; each
 * configured switch/lock on that device becomes a HAP service on that
 * accessory. The plugin keeps state via the server's SSE event stream and
 * falls back to a slow poll as a safety net.
 */
export class CollarPlatform implements DynamicPlatformPlugin {
  public readonly Service: typeof Service;
  public readonly Characteristic: typeof Characteristic;

  public readonly client: CollarClient;
  private readonly pollIntervalMs: number;
  private readonly disabled: boolean;

  /** accessory_uuid -> CollarDeviceAccessory. */
  private readonly devices = new Map<string, CollarDeviceAccessory>();
  /** Cached PlatformAccessory objects supplied by Homebridge on boot. */
  private readonly cachedAccessories = new Map<string, PlatformAccessory>();

  private pollTimer: NodeJS.Timeout | null = null;
  private sseController: AbortController | null = null;
  private sseReconnectDelayMs = SSE_RECONNECT_MIN_MS;
  private shuttingDown = false;

  constructor(
    public readonly log: Logger,
    config: PlatformConfig,
    public readonly api: API,
  ) {
    this.Service = this.api.hap.Service;
    this.Characteristic = this.api.hap.Characteristic;

    const platformConfig = config as PlatformConfig & CollarPlatformConfig;

    if (!platformConfig.serverUrl || !platformConfig.apiKey) {
      this.log.error(
        'Collar platform requires both `serverUrl` and `apiKey` in config — disabling.',
      );
      this.client = new CollarClient('http://invalid.local', '', log);
      this.pollIntervalMs = DEFAULT_POLL_INTERVAL_SECONDS * 1000;
      this.disabled = true;
      return;
    }

    this.disabled = false;
    this.client = new CollarClient(platformConfig.serverUrl, platformConfig.apiKey, log);
    this.pollIntervalMs =
      (platformConfig.pollIntervalSeconds ?? DEFAULT_POLL_INTERVAL_SECONDS) * 1000;

    this.api.on('didFinishLaunching', () => {
      this.refresh().catch((err) =>
        this.log.error(`Initial Collar sync failed: ${(err as Error).message}`),
      );
      this.startPolling();
      this.startEventStream();
    });

    this.api.on('shutdown', () => {
      this.shuttingDown = true;
      if (this.pollTimer) {
        clearInterval(this.pollTimer);
        this.pollTimer = null;
      }
      if (this.sseController) {
        this.sseController.abort();
        this.sseController = null;
      }
    });
  }

  configureAccessory(accessory: PlatformAccessory): void {
    this.log.debug(`Loading cached accessory: ${accessory.displayName}`);
    this.cachedAccessories.set(accessory.UUID, accessory);
  }

  private startPolling(): void {
    if (this.pollTimer || this.disabled) {
      return;
    }
    this.pollTimer = setInterval(() => {
      this.refresh().catch((err) =>
        this.log.debug(`Poll failed: ${(err as Error).message}`),
      );
    }, this.pollIntervalMs);
  }

  private startEventStream(): void {
    if (this.disabled || this.shuttingDown) {
      return;
    }
    const controller = new AbortController();
    this.sseController = controller;

    void this.client
      .openEventStream({
        signal: controller.signal,
        onOpen: () => {
          this.log.info('Subscribed to Collar event stream');
          this.sseReconnectDelayMs = SSE_RECONNECT_MIN_MS;
        },
        onEvent: (event) => this.handleEvent(event),
        onClose: (reason) => {
          if (this.shuttingDown) {
            return;
          }
          this.log.warn(
            `Collar event stream closed (${reason}); reconnecting in ${Math.round(
              this.sseReconnectDelayMs / 1000,
            )}s`,
          );
          this.scheduleSseReconnect();
        },
      })
      .catch((err) => {
        if (this.shuttingDown) {
          return;
        }
        this.log.warn(`Event stream error: ${(err as Error).message}`);
        this.scheduleSseReconnect();
      });
  }

  private scheduleSseReconnect(): void {
    const delay = this.sseReconnectDelayMs;
    this.sseReconnectDelayMs = Math.min(
      this.sseReconnectDelayMs * 2,
      SSE_RECONNECT_MAX_MS,
    );
    setTimeout(() => {
      if (!this.shuttingDown) {
        this.startEventStream();
      }
    }, delay);
  }

  private handleEvent(event: HomeKitEvent): void {
    if (event.type === 'heartbeat') {
      return;
    }
    if (event.type === 'switch_updated') {
      this.applyServiceState(event.state);
    }
  }

  /** Full reconciliation against the server's current config. */
  private async refresh(): Promise<void> {
    if (this.disabled) {
      return;
    }
    const states = await this.client.listSwitches();
    const seenAccessoryUuids = new Set<string>();
    const subtypesByAccessory = new Map<string, Set<string>>();

    for (const state of states) {
      seenAccessoryUuids.add(state.accessory_uuid);
      let subtypes = subtypesByAccessory.get(state.accessory_uuid);
      if (!subtypes) {
        subtypes = new Set();
        subtypesByAccessory.set(state.accessory_uuid, subtypes);
      }
      subtypes.add(state.service_subtype);
      this.applyServiceState(state);
    }

    // Prune services that vanished from server config.
    for (const [uuid, device] of this.devices.entries()) {
      const keep = subtypesByAccessory.get(uuid);
      if (!keep) {
        // Entire accessory vanished — daemon may still exist, but no
        // services configured for it. Unregister the accessory.
        this.removeDevice(uuid);
        continue;
      }
      device.pruneTo(keep);
      if (!device.hasServices) {
        this.removeDevice(uuid);
      }
    }

    // Prune cached accessories Homebridge had but no longer match anything.
    for (const [uuid, accessory] of this.cachedAccessories.entries()) {
      if (!seenAccessoryUuids.has(uuid) && !this.devices.has(uuid)) {
        this.log.info(
          `Removing stale cached accessory: ${accessory.displayName}`,
        );
        this.api.unregisterPlatformAccessories(PLUGIN_NAME, PLATFORM_NAME, [
          accessory,
        ]);
        this.cachedAccessories.delete(uuid);
      }
    }
  }

  private applyServiceState(state: HomeKitSwitchState): void {
    const device = this.getOrCreateDevice(state);
    device.setDisplayName(state.device_name);
    device.upsertService(state);
  }

  private getOrCreateDevice(state: HomeKitSwitchState): CollarDeviceAccessory {
    const existing = this.devices.get(state.accessory_uuid);
    if (existing) {
      return existing;
    }

    const cached = this.cachedAccessories.get(state.accessory_uuid);
    if (cached) {
      cached.context.deviceId = state.device_id;
      const device = new CollarDeviceAccessory(this, cached, state.device_name);
      this.devices.set(state.accessory_uuid, device);
      this.api.updatePlatformAccessories([cached]);
      this.log.info(`Restored accessory: ${state.device_name}`);
      return device;
    }

    const accessory = new this.api.platformAccessory(
      state.device_name,
      state.accessory_uuid,
    );
    accessory.context.deviceId = state.device_id;
    const device = new CollarDeviceAccessory(this, accessory, state.device_name);
    this.devices.set(state.accessory_uuid, device);
    this.cachedAccessories.set(state.accessory_uuid, accessory);
    this.api.registerPlatformAccessories(PLUGIN_NAME, PLATFORM_NAME, [accessory]);
    this.log.info(`Added accessory: ${state.device_name}`);
    return device;
  }

  private removeDevice(uuid: string): void {
    const accessory = this.cachedAccessories.get(uuid);
    if (accessory) {
      this.log.info(`Removing accessory: ${accessory.displayName}`);
      this.api.unregisterPlatformAccessories(PLUGIN_NAME, PLATFORM_NAME, [
        accessory,
      ]);
      this.cachedAccessories.delete(uuid);
    }
    this.devices.delete(uuid);
  }
}
