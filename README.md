# RusTunnel

A self-hosted TCP/HTTP tunnel written in Rust — similar to ngrok but running entirely on your own VPS. It exposes local services to the internet through a persistent WebSocket control connection.

## How it works

```
[local service] <──> [rustunnel-client] <──WS──> [rustunnel-server on VPS] <──> [internet]
```

- The **client** runs on your local machine and connects to the server over WebSocket.
- The **server** runs on your VPS, accepts public TCP connections and HTTP requests, and forwards them through the tunnel to the client.
- **TCP tunnels** bind a specific port on the VPS (e.g. 25565 for Minecraft).
- **HTTP tunnels** get a UUID-based URL automatically (e.g. `https://yourdomain.com/t/<uuid>`).

## Architecture

```
port-repeat/
├── crates/
│   ├── common/     # shared protocol types, framing, codec
│   ├── server/     # VPS binary
│   └── client/     # local machine binary
├── config/
│   ├── server.toml
│   └── client.toml
└── Dockerfile      # builds rustunnel-server
```

## Configuration

### Server (`config/server.toml`)

```toml
control_port           = 7000          # WebSocket control port
auth_token             = "change-me-secret"
max_streams            = 1024

# HTTP proxy for UUID-based tunnel endpoints
http_proxy_port        = 9000
http_base_url          = "https://tunnel.yourdomain.com"
http_idle_timeout_secs = 300
```

### Client (`config/client.toml`)

```toml
ws_url     = "wss://tunnel.yourdomain.com/tunnel"
auth_token = "change-me-secret"

[[tunnel]]
name        = "minecraft"
remote_port = 25565
local_port  = 25565
protocol    = "tcp"

[[tunnel]]
name       = "api"
local_port = 3000
protocol   = "http"   # server assigns a UUID URL automatically
```

## Building locally

```bash
# Build both binaries
cargo build --release

# Run the server
./target/release/rustunnel-server --config config/server.toml

# Run the client
./target/release/rustunnel-client --config config/client.toml
```

Requires Rust 1.87+ and OpenSSL dev headers (`libssl-dev` on Debian/Ubuntu).

---

## Deploying on a VPS with Coolify + Traefik

This guide assumes you have [Coolify](https://coolify.io) installed on your VPS with Traefik as the reverse proxy (the default setup).

### Ports required

| Port | Purpose |
|------|---------|
| `7000` | WebSocket control connection (client → server) |
| `9000` | HTTP proxy for UUID-based tunnels |
| `25565` (optional) | Example TCP tunnel port (Minecraft, etc.) |

> TCP tunnel ports (non-HTTP) must be opened directly on the VPS — Traefik handles only HTTP/HTTPS traffic.

---

### Step 1 — Open firewall ports

On your VPS, open the required ports:

```bash
# Control port and HTTP proxy
ufw allow 7000/tcp
ufw allow 9000/tcp

# Any TCP tunnel ports you plan to use
ufw allow 25565/tcp
```

---

### Step 2 — Deploy with Coolify

1. In Coolify, create a new **Docker Image** or **Git-based** service pointing to this repository.
2. Set the **Dockerfile** path to `Dockerfile` (root of the project).
3. Under **Environment Variables**, set:

   ```
   RUST_LOG=info
   ```

4. Under **Ports**, add:
   - `7000:7000` — control port
   - `9000:9000` — HTTP proxy

5. If you need TCP tunnel ports (e.g. Minecraft), also add:
   - `25565:25565`

6. Mount a persistent volume or use an **Environment variable** / config file override for `config/server.toml`. The easiest approach in Coolify is to use a **Custom Config File** mount:

   - Host path: `/data/rustunnel/server.toml`
   - Container path: `/app/config/server.toml`

   Then place your `server.toml` at `/data/rustunnel/server.toml` on the VPS.

---

### Step 3 — Configure Traefik for HTTPS (WebSocket + HTTP proxy)

Coolify manages Traefik labels automatically for HTTP services. For the WebSocket control port and the HTTP proxy to work behind HTTPS, add these labels in Coolify under **Advanced → Labels**:

```yaml
# Route HTTPS traffic on /tunnel to the WebSocket control port
traefik.enable=true

# WebSocket upgrade label (required for WS connections)
traefik.http.middlewares.ws-headers.headers.customrequestheaders.X-Forwarded-Proto=https

# Service for the HTTP proxy (port 9000)
traefik.http.routers.rustunnel-http.rule=Host(`tunnel.yourdomain.com`)
traefik.http.routers.rustunnel-http.entrypoints=https
traefik.http.routers.rustunnel-http.tls=true
traefik.http.routers.rustunnel-http.tls.certresolver=letsencrypt
traefik.http.services.rustunnel-http.loadbalancer.server.port=9000

# WebSocket control endpoint (/tunnel path)
traefik.http.routers.rustunnel-ws.rule=Host(`tunnel.yourdomain.com`) && PathPrefix(`/tunnel`)
traefik.http.routers.rustunnel-ws.entrypoints=https
traefik.http.routers.rustunnel-ws.tls=true
traefik.http.routers.rustunnel-ws.tls.certresolver=letsencrypt
traefik.http.services.rustunnel-ws.loadbalancer.server.port=7000
```

Replace `tunnel.yourdomain.com` with your actual domain pointed to the VPS.

---

### Step 4 — Update server.toml

Once the domain is set up, update `http_base_url` in `server.toml`:

```toml
http_base_url = "https://tunnel.yourdomain.com"
```

And update the client's `ws_url`:

```toml
ws_url = "wss://tunnel.yourdomain.com/tunnel"
```

---

### Step 5 — Run the client locally

Download or build the client binary and run:

```bash
./rustunnel-client --config config/client.toml
```

The client will connect, authenticate, and print the assigned endpoints:

```
TCP tunnel "minecraft" → yourvps.com:25565
HTTP tunnel "api"      → https://tunnel.yourdomain.com/t/3f2a1b4c-...
```

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Log level (`debug`, `info`, `warn`, `error`) |

---

## Security notes

- Change `auth_token` from the default `change-me-secret` before deploying.
- TCP tunnel ports bypass Traefik and are exposed directly — only open ports you actually use.
- HTTP tunnel UUIDs are not secret by default; add auth middleware at the application level if needed.
