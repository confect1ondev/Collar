import type { CharacteristicValue, PlatformAccessory, Service } from 'homebridge';
import type { CollarPlatform } from './platform';
import type { HomeKitSwitchState } from './types';
import { sendMagicPacket } from './wol';

/**
 * One HomeKit service backed by one Collar entry. Concrete subclasses pick
 * the HAP service type (Switch, LockMechanism, …) and the characteristic
 * wiring; the base class handles state caching and the network call to the
 * server on SET.
 */
export abstract class CollarServiceHandler {
  protected service: Service;
  protected state: HomeKitSwitchState;

  constructor(
    protected readonly platform: CollarPlatform,
    protected readonly accessory: PlatformAccessory,
    initialState: HomeKitSwitchState,
  ) {
    this.state = initialState;
    this.service = this.ensureService();
    this.service.setCharacteristic(
      this.platform.Characteristic.Name,
      initialState.name,
    );
    this.wireCharacteristics();
    this.pushUpdate(initialState);
  }

  get subtype(): string {
    return this.state.service_subtype;
  }

  get switchId(): string {
    return this.state.id;
  }

  /** Called by platform when a fresh state lands (poll or SSE). */
  update(next: HomeKitSwitchState): void {
    this.state = next;
    // Keep the display name in sync if the user renames in config.
    this.service.updateCharacteristic(this.platform.Characteristic.Name, next.name);
    this.pushUpdate(next);
  }

  /** Service factored out for shutdown — invoked when the entry vanishes. */
  remove(): void {
    this.accessory.removeService(this.service);
  }

  protected abstract ensureService(): Service;
  protected abstract wireCharacteristics(): void;
  protected abstract pushUpdate(state: HomeKitSwitchState): void;

  protected async dispatchSet(on: boolean): Promise<void> {
    this.platform.log.info(
      `${this.state.accessory_type} ${this.state.id} (${this.state.name}) → ${on ? 'ON' : 'OFF'}`,
    );
    try {
      await this.platform.client.setSwitch(this.state.id, on);
      // Optimistic local update; the next SSE event or poll reconciles.
      this.state = { ...this.state, on };
    } catch (err) {
      this.platform.log.warn(
        `Failed to set ${this.state.id}: ${(err as Error).message}`,
      );
      // Throw so HomeKit reverts the UI toggle.
      throw err;
    }
  }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

export class CollarSwitchHandler extends CollarServiceHandler {
  protected ensureService(): Service {
    const Service = this.platform.Service;
    return (
      this.accessory.getServiceById(Service.Switch, this.state.service_subtype) ||
      this.accessory.addService(
        Service.Switch,
        this.state.name,
        this.state.service_subtype,
      )
    );
  }

  protected wireCharacteristics(): void {
    const C = this.platform.Characteristic;
    this.service
      .getCharacteristic(C.On)
      .onGet(this.handleGetOn.bind(this))
      .onSet(this.handleSetOn.bind(this));
  }

  protected pushUpdate(state: HomeKitSwitchState): void {
    const C = this.platform.Characteristic;
    if (state.on !== null) {
      this.service.updateCharacteristic(C.On, state.on);
    }
    this.service.updateCharacteristic(
      C.StatusFault,
      state.device_online
        ? C.StatusFault.NO_FAULT
        : C.StatusFault.GENERAL_FAULT,
    );
  }

  private async handleGetOn(): Promise<CharacteristicValue> {
    return this.state.on === null ? false : this.state.on;
  }

  private async handleSetOn(value: CharacteristicValue): Promise<void> {
    const on = Boolean(value);

    // Wake-on-LAN short-circuit: when turning a switch ON for a device
    // that's currently offline, asking the server to dispatch a command is
    // pointless (the daemon isn't there to receive it). If we have a MAC,
    // fire a magic packet on the LAN instead. The daemon will reconnect
    // once the host boots and the switch state reconciles from there.
    if (
      on &&
      !this.state.device_online &&
      this.state.wol_mac
    ) {
      const unicastIp = this.state.wol_ip ?? null;
      this.platform.log.info(
        `Wake-on-LAN: sending magic packet for ${this.state.id} (mac=${this.state.wol_mac}, unicast=${unicastIp ?? 'none'})`,
      );
      try {
        await sendMagicPacket(this.state.wol_mac, { unicastIp });
        // Optimistic: claim ON until the daemon reconnects (or doesn't,
        // in which case the next poll will reset us).
        this.state = { ...this.state, on: true };
        return;
      } catch (err) {
        this.platform.log.warn(
          `WoL packet for ${this.state.id} failed: ${(err as Error).message}`,
        );
        throw err;
      }
    }

    await this.dispatchSet(on);
  }
}

// ---------------------------------------------------------------------------
// LockMechanism
// ---------------------------------------------------------------------------

export class CollarLockHandler extends CollarServiceHandler {
  protected ensureService(): Service {
    const Service = this.platform.Service;
    return (
      this.accessory.getServiceById(
        Service.LockMechanism,
        this.state.service_subtype,
      ) ||
      this.accessory.addService(
        Service.LockMechanism,
        this.state.name,
        this.state.service_subtype,
      )
    );
  }

  protected wireCharacteristics(): void {
    const C = this.platform.Characteristic;
    this.service
      .getCharacteristic(C.LockTargetState)
      .onGet(this.handleGetTarget.bind(this))
      .onSet(this.handleSetTarget.bind(this));
    this.service
      .getCharacteristic(C.LockCurrentState)
      .onGet(this.handleGetCurrent.bind(this));
  }

  protected pushUpdate(state: HomeKitSwitchState): void {
    const C = this.platform.Characteristic;
    // Offline → present as SECURED regardless of cached on-value (a
    // powered-off PC is effectively locked from the user's perspective).
    if (!state.device_online) {
      this.service.updateCharacteristic(
        C.LockTargetState,
        C.LockTargetState.SECURED,
      );
      this.service.updateCharacteristic(
        C.LockCurrentState,
        C.LockCurrentState.SECURED,
      );
      return;
    }
    if (state.on !== null) {
      this.service.updateCharacteristic(
        C.LockTargetState,
        state.on
          ? C.LockTargetState.SECURED
          : C.LockTargetState.UNSECURED,
      );
      this.service.updateCharacteristic(
        C.LockCurrentState,
        this.currentStateFor(state.on, state.device_online),
      );
    }
  }

  private currentStateFor(on: boolean, online: boolean): CharacteristicValue {
    const C = this.platform.Characteristic;
    // Semantic: a PC that's powered off is, for any meaningful purpose, locked.
    // Force SECURED whenever the device is offline so HomeKit doesn't show
    // an "unlocked" icon for a machine you can't actually use.
    if (!online) {
      return C.LockCurrentState.SECURED;
    }
    return on ? C.LockCurrentState.SECURED : C.LockCurrentState.UNSECURED;
  }

  private async handleGetTarget(): Promise<CharacteristicValue> {
    const C = this.platform.Characteristic;
    // Offline implies SECURED — keeps the Home app's lock icon consistent
    // with `handleGetCurrent` below.
    if (!this.state.device_online) {
      return C.LockTargetState.SECURED;
    }
    return this.state.on
      ? C.LockTargetState.SECURED
      : C.LockTargetState.UNSECURED;
  }

  private async handleGetCurrent(): Promise<CharacteristicValue> {
    const C = this.platform.Characteristic;
    if (!this.state.device_online) {
      return C.LockCurrentState.SECURED;
    }
    if (this.state.on === null) {
      return C.LockCurrentState.UNSECURED;
    }
    return this.currentStateFor(this.state.on, this.state.device_online);
  }

  private async handleSetTarget(value: CharacteristicValue): Promise<void> {
    const C = this.platform.Characteristic;
    const on = value === C.LockTargetState.SECURED;
    try {
      await this.dispatchSet(on);
      // Optimistically reflect the target as current; the next event from
      // the server confirms.
      this.service.updateCharacteristic(
        C.LockCurrentState,
        on ? C.LockCurrentState.SECURED : C.LockCurrentState.UNSECURED,
      );
    } catch (err) {
      // Surface JAMMED so HomeKit shows a clear failure indicator.
      this.service.updateCharacteristic(
        C.LockCurrentState,
        C.LockCurrentState.JAMMED,
      );
      throw err;
    }
  }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

export function createServiceHandler(
  platform: CollarPlatform,
  accessory: PlatformAccessory,
  state: HomeKitSwitchState,
): CollarServiceHandler {
  switch (state.accessory_type) {
    case 'lock':
      return new CollarLockHandler(platform, accessory, state);
    case 'switch':
    default:
      return new CollarSwitchHandler(platform, accessory, state);
  }
}
