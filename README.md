# Shinrai

Bring a **public hostname live on the internet** via [Cloudflare Tunnel](https://developers.cloudflare.com/tunnel/get-started/) — CLI only, no dashboard.

## How to run

```bash
nix develop
cargo build --release

./target/release/shinrai install
./target/release/shinrai up
```

You only enter:

1. **Public domain** (e.g. `app.example.com`)
2. **Cloudflare API token** (Tunnel Edit + DNS Edit)

Account id + zone id are looked up from the domain.  
Local origin defaults to an uncommon port:

```text
http://127.0.0.1:47821
```

Run your app on **47821** before expecting the public URL to work (otherwise Cloudflare returns **502**).

## Commands

| Command | What it does |
|---------|----------------|
| `shinrai install` | Install `cloudflared` if missing |
| `shinrai init` | Create tunnel + DNS |
| `shinrai start` / `stop` | Connect / disconnect connector |
| `shinrai status` | Healthy/Inactive/Down/Degraded + curl public host |
| `shinrai up` | init + start + verify public URL |

## Success / failure

| Signal | Meaning |
|--------|---------|
| Tunnel **Healthy** + HTTP 2xx | Public domain live |
| **Inactive** / **1033** | `cloudflared` not connected → `shinrai start` |
| **Down** | Connector stopped → `shinrai start` |
| **Degraded** | Check outbound port **7844** |
| **502** | App not listening on `127.0.0.1:47821` |

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Kmassidik/Shinrai/main/install.sh | bash
```

## Tests

```bash
cargo test
```
# Shinrai
