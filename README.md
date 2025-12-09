# Collar

Discipline for your clankers. Remote control your computers from your phone!

```
┌─────────────┐       ┌─────────────┐       ┌─────────────┐
│   Phone     │ ◄───► │   Server    │ ◄───► │   Collar    │
│   Browser   │ HTTPS │   (API)     │  WSS  │   (Daemon)  │
└─────────────┘       └─────────────┘       └─────────────┘
```

## Components

- **collar-daemon** - Background service on your computer (Rust)
- **collar-server** - API server on VPS (Rust)
- **collar-web** - Control panel frontend (React)

## Quick Start

### 1. Build Everything

```bash
# Build Rust components
cargo build --release

# Build frontend
cd collar-web && npm install && npm run build
```

### 2. Configure Server

```bash
cp server.example.toml server.toml
# Edit server.toml:
# - Set jwt_secret to a secure random string
# - Set admin password hash
# - Add your devices
```

Generate password hash:
```bash
echo -n "yourpassword" | argon2 $(openssl rand -base64 16) -e
```

### 3. Configure Daemon

```bash
cp collar.example.toml ~/.config/collar/config.toml
# Edit config.toml:
# - Set server URL
# - Set device_key (from server config)
# - Customize scripts
```

### 4. Configure nginx (VPS)

```bash
# Edit deploy/nginx/collar.conf
# Replace FRONTEND_DOMAIN and API_DOMAIN with your domains
sudo cp deploy/nginx/collar.conf /etc/nginx/sites-available/collar
sudo ln -s /etc/nginx/sites-available/collar /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx

# Get TLS certificates
sudo certbot --nginx -d your-frontend.example.com -d your-api.example.com
```

### 5. Build Frontend (with API URL if cross-domain)

```bash
cd collar-web

# If API is on same domain, just build:
npm run build

# If API is on different domain:
VITE_API_URL=https://your-api.example.com/api npm run build
```

### 6. Run

```bash
# On VPS
./target/release/collar-server server.toml

# On your computer
./target/release/collar-daemon
```

## Scripts

Scripts are the extensible unit of functionality. Define them in your daemon config:

```toml
[[scripts]]
id = "lock"
name = "Lock Screen"
type = "action"
command = "loginctl lock-session"

[[scripts]]
id = "is_locked"
name = "Lock Status"
type = "status"
command = "loginctl show-session -p LockedHint --value"
```

**Script Types:**
- `action` - Performs an action (lock, mute, sleep)
- `status` - Returns status info (is locked?, battery level)

## Security

- JWT authentication via httpOnly cookies
- Rate limiting (100 req/min per IP)
- Per-device API keys
- Scripts defined locally (server cannot send arbitrary commands)

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed diagrams.

## License

MIT
