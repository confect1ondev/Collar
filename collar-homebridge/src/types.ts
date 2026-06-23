// Mirror of collar-common's HomeKit types. Kept hand-maintained to avoid
// dragging a codegen step into a tiny TS project; if these drift, the plugin
// will fail at runtime when it can't decode a response.

export type HomeKitAccessoryType = 'switch' | 'lock';

export interface HomeKitSwitchState {
  /** User-facing service id. Cosmetic. */
  id: string;
  /** Stable accessory UUID, shared across every service on the same device. */
  accessory_uuid: string;
  /** Stable per-service identifier within the accessory. */
  service_subtype: string;
  /** Which HAP service type to expose this as. Defaults to 'switch'. */
  accessory_type: HomeKitAccessoryType;
  /** Display name for this service. */
  name: string;
  device_id: string;
  /** Display name for the device-level accessory. */
  device_name: string;
  device_online: boolean;
  on: boolean | null;
  last_observed: string | null;
  /**
   * MAC address for Wake-on-LAN. When set, the plugin sends a magic packet
   * on the local LAN when the user toggles ON and the device is offline.
   * Server omits the field when no MAC is configured.
   */
  wol_mac?: string | null;
  /**
   * Last-known LAN IP for the device, as reported by the daemon at its most
   * recent connect. Used for *unicast* WoL on networks (e.g. eero mesh)
   * that drop UDP broadcasts. Stale once the router's ARP cache for this
   * IP expires.
   */
  wol_ip?: string | null;
}

export interface HomeKitSetRequest {
  on: boolean;
}

export interface HomeKitSetResponse {
  command_id: string;
  dispatched_script: string;
}

export type HomeKitEvent =
  | { type: 'switch_updated'; state: HomeKitSwitchState }
  | { type: 'heartbeat' };

export interface CollarPlatformConfig {
  platform: 'Collar';
  name?: string;
  serverUrl: string;
  apiKey: string;
  pollIntervalSeconds?: number;
}
