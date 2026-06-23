import type { PlatformAccessory } from 'homebridge';
import type { CollarPlatform } from './platform';
import { CollarServiceHandler, createServiceHandler } from './services';
import type { HomeKitSwitchState } from './types';

/**
 * Wraps a single HomeKit accessory representing one Collar device. The
 * accessory may carry multiple services — e.g. a Switch for Power and a
 * LockMechanism for the screen lock — keyed by their stable service_subtype.
 *
 * Reconciliation lives on the platform; this class is just the per-device
 * bookkeeping.
 */
export class CollarDeviceAccessory {
  /** Map of service_subtype -> handler. */
  private readonly services = new Map<string, CollarServiceHandler>();

  constructor(
    private readonly platform: CollarPlatform,
    public readonly accessory: PlatformAccessory,
    deviceName: string,
  ) {
    this.setAccessoryInformation(deviceName);
  }

  get uuid(): string {
    return this.accessory.UUID;
  }

  /** Rename when the device's display name changes upstream. */
  setDisplayName(name: string): void {
    if (this.accessory.displayName !== name) {
      this.accessory.displayName = name;
      this.setAccessoryInformation(name);
    }
  }

  /** Insert or refresh a service on this accessory. */
  upsertService(state: HomeKitSwitchState): void {
    const existing = this.services.get(state.service_subtype);
    if (existing) {
      existing.update(state);
      return;
    }
    const handler = createServiceHandler(this.platform, this.accessory, state);
    this.services.set(state.service_subtype, handler);
  }

  /**
   * Drop services whose subtype isn't in `keep`. Used when an entry is
   * removed from server config — the corresponding HAP service disappears
   * from the accessory but the rest of the accessory survives.
   *
   * Also sweeps *orphan* Switch/LockMechanism services persisted on the
   * accessory but never re-claimed by the plugin (left behind by earlier
   * plugin versions that registered services without a subtype, or with a
   * different subtype scheme). Without this, accessories accumulate ghost
   * services across upgrades.
   */
  pruneTo(keep: Set<string>): void {
    for (const [subtype, handler] of this.services.entries()) {
      if (!keep.has(subtype)) {
        this.platform.log.info(
          `Removing vanished service ${handler.switchId} from ${this.accessory.displayName}`,
        );
        handler.remove();
        this.services.delete(subtype);
      }
    }

    const SwitchUUID = this.platform.Service.Switch.UUID;
    const LockUUID = this.platform.Service.LockMechanism.UUID;
    for (const svc of [...this.accessory.services]) {
      const isOurType = svc.UUID === SwitchUUID || svc.UUID === LockUUID;
      if (!isOurType) {
        continue;
      }
      // Untyped because subtype is optional on the HAP Service type.
      const subtype = (svc as { subtype?: string }).subtype;
      if (!subtype || !keep.has(subtype)) {
        this.platform.log.info(
          `Removing orphan service (subtype=${subtype ?? '<none>'}) from ${this.accessory.displayName}`,
        );
        this.accessory.removeService(svc);
      }
    }
  }

  get hasServices(): boolean {
    return this.services.size > 0;
  }

  private setAccessoryInformation(name: string): void {
    const info = this.accessory.getService(
      this.platform.Service.AccessoryInformation,
    );
    if (!info) {
      return;
    }
    info
      .setCharacteristic(this.platform.Characteristic.Manufacturer, 'Collar')
      .setCharacteristic(this.platform.Characteristic.Model, 'Collar Device')
      .setCharacteristic(this.platform.Characteristic.SerialNumber, this.uuid)
      .setCharacteristic(this.platform.Characteristic.Name, name);
  }
}
