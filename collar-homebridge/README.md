# homebridge-collar

Homebridge plugin that exposes [Collar](../README.md) commands as HomeKit
switches. Each switch has an ON action, an OFF action, and a state script
that reports back whether the switch should currently read as ON or OFF.

Switches are configured on the **Collar server**, not in Homebridge. The
plugin just discovers and surfaces them.

## How it works

```
HomeKit ───► Homebridge ───► collar-server ───► collar-daemon
                  ▲                ▲                  │
                  │ SSE events     └─── status ───────┘
                  │ + slow poll
                  └────────────────────────────────────┘
```

- The plugin subscribes to `GET /api/homekit/events` (SSE) for near
  real-time switch updates and falls back to polling
  `GET /api/homekit/switches` every 60 seconds as a safety net.
- Each switch becomes a HomeKit Switch accessory whose `On` characteristic
  reflects the daemon's last-reported state.
- Toggling the switch in the Home app calls `POST /api/homekit/switches/:id/set`
  with `{ on: true | false }`, which dispatches the configured on/off script.
- Switches survive server restarts (the server persists last-known state via
  `state_path`) — the accessory does not disappear from HomeKit while the
  daemon is offline.
- Each switch carries a stable `accessory_uuid` derived from the device id +
  script triple, so renaming the user-facing `id` in `server.toml` is
  pairing-safe.

## Install

```bash
# Build the plugin
cd collar-homebridge
npm install
npm run build

# Install into your Homebridge instance (development link):
npm link
cd /path/to/homebridge
npm link homebridge-collar
```

Or publish + `npm install -g homebridge-collar` once published.

## Configure

In Homebridge's `config.json`:

```json
{
  "platforms": [
    {
      "platform": "Collar",
      "name": "Collar",
      "serverUrl": "https://your-collar-api.example.com",
      "apiKey": "homebridge-collar-key-change-this",
      "pollIntervalSeconds": 60
    }
  ]
}
```

`apiKey` must match `[homekit].api_key` in your Collar server's `server.toml`.

## Switch configuration

Switches are defined **server-side** in `server.toml`:

```toml
[homekit]
api_key = "homebridge-collar-key-change-this"

[[homekit.switches]]
id = "desktop_lock"
name = "Desktop Lock"
device_id = "550e8400-e29b-41d4-a716-446655440000"
on_script  = "lock"
off_script = "unlock"
state_script  = "is_locked"
state_on_value = "yes"
```

The `on_script`, `off_script`, and `state_script` ids must exist in the
daemon's local script registry, and `state_script` must also appear in the
daemon's `polling.status_scripts` list — otherwise the switch state never
updates.

See [docs/HOMEKIT.md](../docs/HOMEKIT.md) for all options:
`accessory_type = "lock"` (HomeKit lock instead of switch),
`state_source = "online"` (state mirrors WebSocket connection), and per-device
`wol_mac` for Wake-on-LAN.

## Service state semantics

- **Switch**: the cached `on` value from the most recent server update.
  `StatusFault` is set when the daemon is offline so the Home app shows it
  as unreachable. Toggling an offline switch fails the SET and HomeKit
  reverts the UI — unless `wol_mac` is set and the user toggled ON, in
  which case the plugin fires a Wake-on-LAN magic packet on the LAN
  (broadcast plus unicast to the daemon's last-known IP) and reports the
  switch as ON optimistically.
- **Lock** (`accessory_type = "lock"`): `on=true` ↔ Secured. An offline
  daemon always reports Secured (a powered-off PC is effectively locked).
  A failed SET surfaces as `Jammed` so the failure is visible in the Home
  app.
