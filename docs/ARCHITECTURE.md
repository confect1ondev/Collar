# Collar - Remote Control Architecture

## Overview

Collar is a three-component system for remotely controlling a computer from a phone or any web browser.

```
┌─────────────┐       ┌─────────────┐       ┌─────────────┐
│   Phone/    │ ◄───► │   Server    │ ◄───► │   Collar    │
│   Browser   │ HTTPS │   (API)     │  WSS  │   (Daemon)  │
└─────────────┘       └─────────────┘       └─────────────┘
    Frontend            VPS/Cloud           Your Computer
```

## System Architecture

```mermaid
flowchart TB
    subgraph Client["Your Computer"]
        Collar[Collar Daemon]
        Scripts[(Script Registry)]
        Collar --> Scripts
    end

    subgraph Cloud["VPS/Server"]
        API[REST API]
        WS[WebSocket Hub]
        Auth[Auth Middleware]
        DB[(SQLite/State)]
        API --> Auth
        WS --> Auth
        API --> DB
        WS --> DB
    end

    subgraph Frontend["Web Interface"]
        React[React App]
        Panel[Control Panel]
        Status[Status Display]
        React --> Panel
        React --> Status
    end

    React <-->|HTTPS| API
    React <-->|WSS| WS
    Collar <-->|WSS| WS
```

## Component Details

### 1. Collar (Client Daemon)

The background service running on your computer.

```mermaid
flowchart LR
    subgraph Collar
        Conn[Connection Manager]
        Exec[Command Executor]
        Poll[Status Poller]
        Scripts[(Scripts)]

        Conn --> Exec
        Conn --> Poll
        Exec --> Scripts
        Poll --> Scripts
    end

    WS[Server WebSocket] <--> Conn
```

**Responsibilities:**
- Maintain persistent WebSocket connection to server
- Execute registered scripts on command
- Poll system status periodically
- Auto-reconnect on disconnect

### 2. Server (API)

The intermediary that routes commands and maintains state.

```mermaid
flowchart TB
    subgraph Server
        subgraph Routes
            R1[POST /auth/login]
            R1b[POST /auth/logout]
            R2[GET /devices]
            R2b[GET /devices/:id]
            R3[POST /devices/:id/command]
            R4[GET /devices/:id/status]
            R4b[POST /devices/:id/refresh]
            R4c[GET /devices/:id/scripts]
            R5[WS /ws]
        end

        subgraph Core
            Hub[WebSocket Hub]
            State[Device State]
        end

        R3 --> Hub
        R4 --> State
        R5 --> Hub
        Hub --> State
    end
```

**Responsibilities:**
- Authenticate users and devices
- Route commands from frontend to correct device
- Maintain device connection state
- Reject commands when target device is offline (no queueing — keeps retries predictable)

### 3. Frontend (React)

Clean control panel for sending commands and viewing status.

```mermaid
flowchart TB
    subgraph Frontend
        App[App]
        Auth[Auth Context]
        WS[WebSocket Hook]

        subgraph Pages
            Login[Login]
            Dashboard[Dashboard]
        end

        subgraph Components
            DeviceCard[Device Card]
            ScriptButton[Script Button]
            StatusBadge[Status Badge]
        end

        App --> Auth
        App --> WS
        App --> Pages
        Dashboard --> Components
    end
```

## Data Flow

### Command Execution Flow

```mermaid
sequenceDiagram
    participant U as User (Phone)
    participant F as Frontend
    participant S as Server
    participant C as Collar

    U->>F: Click "Lock Screen"
    F->>S: POST /devices/:id/command {script: "lock"}
    S->>S: Validate & route
    S->>C: WS: {type: "execute", script: "lock"}
    C->>C: Run lock script
    C->>S: WS: {type: "result", success: true}
    S->>F: WS: {type: "command_result", ...}
    F->>U: Show success toast
```

### Status Polling Flow

```mermaid
sequenceDiagram
    participant F as Frontend
    participant S as Server
    participant C as Collar

    loop Every 30s
        C->>C: Run status scripts
        C->>S: WS: {type: "status", data: {...}}
        S->>S: Update device state
    end

    F->>S: GET /devices/:id/status
    S->>F: {locked: false, battery: 85, ...}
```

## Script System

Scripts are the extensible unit of functionality.

```mermaid
classDiagram
    class Script {
        +String id
        +String name
        +String description
        +ScriptType type
        +String command
        +execute() Result
    }

    class ScriptType {
        <<enumeration>>
        Action
        Status
    }

    class ScriptRegistry {
        +Vec~Script~ scripts
        +register(Script)
        +get(id) Option~Script~
        +execute(id) Result
    }

    ScriptRegistry --> Script
    Script --> ScriptType
```

### Script Configuration (TOML)

```toml
[[scripts]]
id = "lock"
name = "Lock Screen"
description = "Lock the desktop session"
type = "action"
command = "loginctl lock-session"

[[scripts]]
id = "unlock"
name = "Unlock Screen"
description = "Unlock the desktop session"
type = "action"
command = "loginctl unlock-session"

[[scripts]]
id = "is_locked"
name = "Lock Status"
description = "Check if screen is locked"
type = "status"
command = "loginctl show-session -p LockedHint --value"
```

## Security Model

```mermaid
flowchart TB
    subgraph Auth
        Cookie[httpOnly Cookie]
        DeviceKey[Device API Key]
    end

    subgraph Frontend
        Login[Login with Password]
        Session[Session via Cookie]
    end

    subgraph Collar
        Key[Stored Device Key]
        Connect[Connect with Key]
    end

    Login --> Cookie
    Cookie --> Session
    Key --> Connect
    Connect --> DeviceKey
```

**Security Measures:**
- JWT authentication via httpOnly cookies (XSS-resistant)
- Rate limiting (100 requests/minute per IP)
- Device-specific API keys for daemons
- Scripts defined locally on daemon (server cannot send arbitrary commands)

## Project Structure

```
collar/
├── collar-daemon/          # Rust - Background service
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs       # Configuration
│   │   ├── connection.rs   # WebSocket client
│   │   ├── executor.rs     # Script execution
│   │   └── scripts.rs      # Script registry
│   └── Cargo.toml
│
├── collar-server/          # Rust - API server
│   ├── src/
│   │   ├── main.rs
│   │   ├── api.rs          # REST endpoints
│   │   ├── auth.rs         # JWT + cookie authentication
│   │   ├── config.rs       # Configuration
│   │   ├── ratelimit.rs    # Rate limiting middleware
│   │   ├── state.rs        # Shared app state
│   │   └── ws.rs           # WebSocket handler
│   └── Cargo.toml
│
├── collar-web/             # React - Frontend
│   ├── src/
│   │   ├── App.tsx
│   │   ├── main.tsx
│   │   ├── api.ts          # API client
│   │   ├── types.ts        # TypeScript types
│   │   ├── styles.css
│   │   ├── pages/
│   │   ├── components/
│   │   └── hooks/
│   └── package.json
│
├── collar-common/          # Shared Rust types
│   ├── src/
│   │   └── lib.rs
│   └── Cargo.toml
│
├── deploy/                 # Deployment configs
│   ├── nginx/
│   │   └── collar.conf
│   ├── collar-daemon.service
│   ├── collar-server.service
│   └── install-daemon.sh
│
├── collar.example.toml     # Daemon config example
├── server.example.toml     # Server config example
└── docs/
    └── ARCHITECTURE.md
```

## Technology Choices

| Component | Technology | Rationale |
|-----------|------------|-----------|
| Daemon | Rust + tokio | Reliable, low resource usage |
| Server | Rust + axum | Fast, type-safe, async |
| Frontend | React + TypeScript | Clean, maintainable UI |
| WebSocket | tokio-tungstenite | Native async Rust |
| Auth | JWT | Stateless, secure |
| Config | TOML | Human-readable, Rust-native |
