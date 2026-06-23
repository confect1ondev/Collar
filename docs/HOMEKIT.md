# HomeKit / Homebridge Integration

Collar can expose commands as HomeKit switches via a Homebridge plugin. This
sits alongside the web UI as a parallel control surface — both consume the
server's REST API, but with different auth and a different command vocabulary.

## Architecture

```
┌──────────────┐    HTTPS     ┌──────────────┐    WSS     ┌──────────────┐
│  Home app    │ ◄──────────► │ Homebridge   │ ◄────────► │ collar-server│
└──────────────┘    HAP        │ +collar plug │    /ws     └──────┬───────┘
                               └──────────────┘                   │
                                                                  │ WSS
┌──────────────┐    HTTPS     ┌──────────────┐                    ▼
│  Phone web   │ ◄──────────► │ collar-server│            ┌──────────────┐
└──────────────┘              └──────────────┘            │ collar-daemon│
                                                          └──────────────┘
```

The web UI and Homebridge are independent clients. They never see each other.
The server is the source of truth for device state and routes commands to
daemons.

## Auth model

| Surface           | Auth                                      |
|-------------------|-------------------------------------------|
| Web UI            | Admin password → JWT in httpOnly cookie   |
| Daemon (WS)       | Per-device `api_key` in the Auth message  |
| **Homebridge**    | Dedicated `[homekit].api_key`, `Bearer …` |

The Homebridge key is intentionally separate from the admin login so the
plugin can run unattended without holding a user session.

## Persistence

For HomeKit accessories to survive server restarts and brief daemon
outages, set `[server].state_path` in `server.toml`:

```toml
[server]
state_path = "/var/lib/collar/state.json"
```

The server persists:
- Every known device's id, name, last status, and last-reported script list.
- Updates on every status poll, register, and unregister.

If `state_path` is unset, the server runs as before — device state lives only
in memory and a restart will look like every daemon just disconnected.

## Switch definition

Each switch ties on/off/state scripts on one daemon to one HomeKit service:

```toml
[[homekit.switches]]
id           = "desktop_lock"   # cosmetic; renaming it doesn't re-pair
name         = "Desktop Lock"   # shown in the Home app
device_id    = "550e8400-..."   # daemon UUID
on_script    = "lock"           # fired when HomeKit turns it ON
off_script   = "unlock"         # fired when HomeKit turns it OFF
state_script = "is_locked"      # daemon-polled script that reports current state
state_on_value = "yes"          # case-insensitive match for "switch is ON"
```

Switch state is read from the device's most recent status snapshot, looking
up the value of the `state_script` field. So `state_script` **must** appear
in the daemon's `polling.status_scripts` list — otherwise the field never
gets populated and HomeKit will show the switch as OFF indefinitely.

### Accessory type

`accessory_type` picks the HAP service the entry exposes:

| Value           | HomeKit service   | Notes                                       |
|-----------------|-------------------|---------------------------------------------|
| `switch` (default) | Switch         | Plain on/off toggle.                        |
| `lock`          | LockMechanism     | Lock icon, "Lock my desktop" via Siri. `on=true` is interpreted as Secured. An offline device always reports Secured. |

### State source

`state_source` controls where on/off is derived from. Defaults to `status`.

- `state_source = "status"` — reads `state_script` out of the daemon's last
  polled status. Requires `state_script` and `state_on_value`.
- `state_source = "online"` — the service reads ON exactly while the daemon's
  WebSocket is connected. `state_script` / `state_on_value` are ignored.
  Useful for a "power" switch whose `off_script` is `shutdown`: the moment
  the daemon disconnects, HomeKit flips it to OFF.

### Wake-on-LAN

Setting `wol_mac` on a `[[devices]]` entry lets the Homebridge plugin send
a magic packet directly on the LAN when the user turns a switch ON for an
offline device — useful for the power-switch pattern above. The server
also forwards the daemon's last-reported LAN IP, which the plugin uses for
unicast WoL on networks (e.g. eero mesh) that drop UDP broadcasts.

```toml
[[devices]]
device_id = "550e8400-..."
name      = "Desktop"
api_key   = "..."
wol_mac   = "aa:bb:cc:dd:ee:ff"   # 12 hex digits, `:`/`-`/`.` separators optional
```

### Multiple services per device

Several `[[homekit.switches]]` entries with the same `device_id` end up
grouped under one HomeKit accessory. Identity is derived from the device
UUID and the script triple, so:

- Renaming the user-facing `id` is free (no re-pairing).
- Swapping `on_script`/`off_script`/`state_script` is treated as a new service.

## API surface

All endpoints under `/api/homekit` require
`Authorization: Bearer <homekit.api_key>`. The key is checked in constant
time. If `[homekit]` is not configured in `server.toml`, all `/api/homekit/*`
endpoints respond `404 Not Found`.

| Method | Path                            | Body               | Returns                  |
|--------|---------------------------------|--------------------|--------------------------|
| GET    | `/api/homekit/switches`         | —                  | `HomeKitSwitchState[]`   |
| GET    | `/api/homekit/switches/:id`     | —                  | `HomeKitSwitchState`     |
| POST   | `/api/homekit/switches/:id/set` | `{ "on": bool }`   | `HomeKitSetResponse`     |
| GET    | `/api/homekit/events`           | —                  | `text/event-stream`      |

### `/api/homekit/events` (SSE)

Long-lived server-sent event stream. The plugin uses this to push state into
HomeKit in real time instead of relying on polling.

Event types:

- `switch_updated` — `data:` is a `HomeKitSwitchState`. Emitted whenever a
  device reports status (so the switch's `on` may have changed), connects,
  or disconnects. The plugin idempotently applies the new state.
- `heartbeat` — periodic keep-alive (and axum sends comment-style keepalives
  every 15s on top of this; clients should ignore lines starting with `:`).

If a slow consumer falls behind by more than 256 events, events get dropped
silently — the plugin's periodic `/switches` poll catches it back up.

### Switch identity

Each `HomeKitSwitchState` carries `accessory_uuid`, a stable UUID v5 derived
server-side from the `device_id` plus the script triple
(`on_script`/`off_script`/`state_script`). The plugin uses this — not the
user-facing `id` — as the HomeKit accessory UUID, so renaming the `id` in
`server.toml` is a free operation that preserves pairing and automations.
Changing the underlying scripts *is* treated as a new accessory.

## Setup checklist

1. Add `[server].state_path` to `server.toml`.
2. Add `[homekit]` with an `api_key` and one or more `[[homekit.switches]]`.
3. Make sure every `state_script` is listed in the daemon's
   `polling.status_scripts`.
4. Restart the server. The startup log should say
   `HomeKit integration enabled at /api/homekit`.
5. Install + configure the Homebridge plugin (see
   [`collar-homebridge/README.md`](../collar-homebridge/README.md)).
