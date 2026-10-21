# shinrai — Cloudflare Tunnel CLI (public host live on the internet)

Rust CLI: install `cloudflared` if missing, create Tunnel + DNS via API, **start/stop** the connector so your public hostname answers on the internet — no dashboard.

## How to run

```bash
shinrai install
shinrai init      # prompts (no .env)
shinrai start
shinrai status
shinrai stop
# or:
shinrai up        # init + start + curl public URL
```

## Prompts (terminal only)

| Prompt | Role |
|--------|------|
| Public domain | e.g. `app.example.com` |
| Cloudflare API token | Tunnel Edit + Zone DNS Edit |

Account + zone are resolved via `GET /zones?name=…`.  
Origin is fixed uncommon port **47821** → `http://127.0.0.1:47821` (app must listen there).

## Success / failure (Cloudflare docs)

| State | Kind | Bottleneck |
|-------|------|------------|
| **Healthy** | Success | Connector serving; then curl public host |
| **Inactive** | Failure | Tunnel created, `cloudflared` never run → `shinrai start` |
| **Down** | Failure | Connector stopped → `shinrai start` |
| **Degraded** | Failure | Partial links → firewall / outbound **7844** |
| HTTP **1033** | Failure | No healthy connector or bad CNAME |
| HTTP **502** | Failure | Tunnel OK, origin down / wrong service URL |
| HTTP **2xx** | Success | Public domain live on the internet |

Prerequisites from docs: domain on Cloudflare; machine can reach CF on 7844; only one `cloudflared` service per machine.

## API flow (`init`)

1. `POST /accounts/{id}/cfd_tunnel` (`config_src: cloudflare`)
2. `GET .../cfd_tunnel/{id}/token`
3. `PUT .../configurations` ingress hostname → origin + catch-all `http_status:404`
4. DNS CNAME `hostname` → `{tunnel_id}.cfargotunnel.com` (proxied)
5. Save `~/.config/shinrai/tunnel.toml` (mode 600)

## Nix

```bash
nix develop   # rustc + cloudflared + curl
cargo test && cargo run -- hello
```
