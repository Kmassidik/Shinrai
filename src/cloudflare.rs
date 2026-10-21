use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::{zone_name_candidates, Creds, TunnelState};
use crate::status::TunnelStatus;

const API: &str = "https://api.cloudflare.com/client/v4";

/// Resolve account + zone from hostname using the API token (no manual zone/account prompts).
pub fn resolve_zone_for_hostname(api_token: &str, hostname: &str) -> Result<(String, String, String)> {
    let http = Client::builder()
        .user_agent(concat!("shinrai/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("HTTP client")?;

    for candidate in zone_name_candidates(hostname) {
        let url = format!("{API}/zones?name={candidate}");
        let res = http
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {api_token}"))
            .send()
            .with_context(|| format!("lookup zone {candidate}"))?;
        let env: CfEnvelope<Vec<Value>> = res.json().context("decode zones list")?;
        if !env.success {
            continue;
        }
        let Some(list) = env.result else {
            continue;
        };
        if let Some(zone) = list.first() {
            let zone_id = zone
                .get("id")
                .and_then(|v| v.as_str())
                .context("zone missing id")?
                .to_string();
            let zone_name = zone
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&candidate)
                .to_string();
            let account_id = zone
                .pointer("/account/id")
                .and_then(|v| v.as_str())
                .context("zone missing account.id")?
                .to_string();
            return Ok((account_id, zone_id, zone_name));
        }
    }

    bail!(
        "no Cloudflare zone found for hostname `{hostname}`. \
         Add the domain to Cloudflare (nameservers active), then retry."
    );
}

#[derive(Debug, Deserialize)]
struct CfEnvelope<T> {
    success: bool,
    errors: Vec<CfError>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct CfError {
    code: u32,
    message: String,
}

pub struct CfClient {
    http: Client,
    token: String,
    account_id: String,
    zone_id: String,
}

impl CfClient {
    pub fn from_creds(creds: &Creds) -> Result<Self> {
        Self::new(
            creds.cloudflare_api_token.clone(),
            creds.cloudflare_account_id.clone(),
            creds.cloudflare_zone_id.clone(),
        )
    }

    pub fn new(token: String, account_id: String, zone_id: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("shinrai/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            token,
            account_id,
            zone_id,
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    pub fn verify_token(&self) -> Result<()> {
        let url = format!("{API}/user/tokens/verify");
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("verify token")?;
        let env: CfEnvelope<Value> = res.json().context("decode verify response")?;
        if !env.success {
            bail!("token verify failed: {}", format_errors(&env.errors));
        }
        Ok(())
    }

    pub fn verify_zone(&self) -> Result<String> {
        let url = format!("{API}/zones/{}", self.zone_id);
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("get zone")?;
        let env: CfEnvelope<Value> = res.json().context("decode zone")?;
        if !env.success {
            bail!("zone lookup failed: {}", format_errors(&env.errors));
        }
        let zone = env.result.context("missing zone result")?;
        let name = zone
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown)")
            .to_string();
        let account = zone
            .pointer("/account/id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !account.is_empty() && account != self.account_id {
            bail!(
                "zone {name} belongs to account {account}, not the account id you entered ({})",
                self.account_id
            );
        }
        Ok(name)
    }

    /// Create remotely-managed tunnel (`config_src: cloudflare`).
    pub fn create_tunnel(&self, name: &str) -> Result<String> {
        let url = format!("{API}/accounts/{}/cfd_tunnel", self.account_id);
        let body = json!({
            "name": name,
            "config_src": "cloudflare"
        });
        let res = self
            .http
            .post(&url)
            .header(AUTHORIZATION, self.auth_header())
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .context("create tunnel")?;
        let env: CfEnvelope<Value> = res.json().context("decode create tunnel")?;
        if !env.success {
            bail!("create tunnel failed: {}", format_errors(&env.errors));
        }
        let id = env
            .result
            .as_ref()
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .context("create tunnel: missing id")?
            .to_string();
        Ok(id)
    }

    pub fn get_tunnel_token(&self, tunnel_id: &str) -> Result<String> {
        let url = format!(
            "{API}/accounts/{}/cfd_tunnel/{tunnel_id}/token",
            self.account_id
        );
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("get tunnel token")?;
        let env: CfEnvelope<String> = res.json().context("decode tunnel token")?;
        if !env.success {
            bail!("get tunnel token failed: {}", format_errors(&env.errors));
        }
        env.result.context("missing tunnel token")
    }

    pub fn get_tunnel_status(&self, tunnel_id: &str) -> Result<TunnelStatus> {
        let url = format!("{API}/accounts/{}/cfd_tunnel/{tunnel_id}", self.account_id);
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("get tunnel")?;
        let env: CfEnvelope<Value> = res.json().context("decode tunnel")?;
        if !env.success {
            bail!("get tunnel failed: {}", format_errors(&env.errors));
        }
        let status = env
            .result
            .as_ref()
            .and_then(|r| r.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("other");
        Ok(TunnelStatus::parse(status))
    }

    /// Ingress: public hostname → origin, plus required catch-all 404.
    pub fn configure_ingress(&self, tunnel_id: &str, hostname: &str, origin: &str) -> Result<()> {
        let url = format!(
            "{API}/accounts/{}/cfd_tunnel/{tunnel_id}/configurations",
            self.account_id
        );
        let body = json!({
            "config": {
                "ingress": [
                    {
                        "hostname": hostname,
                        "service": origin,
                        "originRequest": {}
                    },
                    {
                        "service": "http_status:404"
                    }
                ]
            }
        });
        let res = self
            .http
            .put(&url)
            .header(AUTHORIZATION, self.auth_header())
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .context("configure tunnel ingress")?;
        let env: CfEnvelope<Value> = res.json().context("decode ingress config")?;
        if !env.success {
            bail!("configure ingress failed: {}", format_errors(&env.errors));
        }
        Ok(())
    }

    /// Proxied CNAME → `<tunnel_id>.cfargotunnel.com`
    pub fn ensure_tunnel_dns(&self, hostname: &str, tunnel_id: &str) -> Result<String> {
        let content = format!("{tunnel_id}.cfargotunnel.com");
        if let Some((id, _)) = self.find_dns_record(hostname)? {
            self.update_dns_cname(&id, hostname, &content)?;
            return Ok(id);
        }
        self.create_dns_cname(hostname, &content)
    }

    fn find_dns_record(&self, hostname: &str) -> Result<Option<(String, String)>> {
        let url = format!(
            "{API}/zones/{}/dns_records?name={hostname}&type=CNAME",
            self.zone_id
        );
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("list DNS")?;
        let env: CfEnvelope<Vec<Value>> = res.json().context("decode DNS list")?;
        if !env.success {
            bail!("list DNS failed: {}", format_errors(&env.errors));
        }
        let Some(list) = env.result else {
            return Ok(None);
        };
        if let Some(rec) = list.first() {
            let id = rec
                .get("id")
                .and_then(|v| v.as_str())
                .context("dns record missing id")?
                .to_string();
            let content = rec
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Ok(Some((id, content)));
        }
        // Also check A records that would block CNAME
        let url = format!(
            "{API}/zones/{}/dns_records?name={hostname}",
            self.zone_id
        );
        let res = self
            .http
            .get(&url)
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .context("list DNS any")?;
        let env: CfEnvelope<Vec<Value>> = res.json().context("decode DNS any")?;
        if env.success {
            if let Some(list) = env.result {
                if let Some(rec) = list.first() {
                    let ty = rec.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    bail!(
                        "DNS record already exists for {hostname} (type {ty}). \
                         Delete it in Cloudflare DNS or pick another hostname. \
                         Docs: conflicting A/AAAA/CNAME blocks tunnel publish."
                    );
                }
            }
        }
        Ok(None)
    }

    fn create_dns_cname(&self, hostname: &str, content: &str) -> Result<String> {
        let url = format!("{API}/zones/{}/dns_records", self.zone_id);
        let body = json!({
            "type": "CNAME",
            "proxied": true,
            "name": hostname,
            "content": content
        });
        let res = self
            .http
            .post(&url)
            .header(AUTHORIZATION, self.auth_header())
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .context("create DNS CNAME")?;
        let env: CfEnvelope<Value> = res.json().context("decode create DNS")?;
        if !env.success {
            bail!(
                "create DNS failed: {}. \
                 If a record already exists, delete the conflicting A/AAAA/CNAME first.",
                format_errors(&env.errors)
            );
        }
        Ok(env
            .result
            .as_ref()
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .context("missing DNS id")?
            .to_string())
    }

    fn update_dns_cname(&self, record_id: &str, hostname: &str, content: &str) -> Result<()> {
        let url = format!("{API}/zones/{}/dns_records/{record_id}", self.zone_id);
        let body = json!({
            "type": "CNAME",
            "proxied": true,
            "name": hostname,
            "content": content
        });
        let res = self
            .http
            .put(&url)
            .header(AUTHORIZATION, self.auth_header())
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .context("update DNS CNAME")?;
        let env: CfEnvelope<Value> = res.json().context("decode update DNS")?;
        if !env.success {
            bail!("update DNS failed: {}", format_errors(&env.errors));
        }
        Ok(())
    }

    /// Full API-side bring-up (tunnel + ingress + DNS). Does not start cloudflared.
    pub fn provision_tunnel(&self, creds: &Creds) -> Result<TunnelState> {
        let hostname = creds.hostname()?;
        let tunnel_name = crate::config::tunnel_name_for_host(&hostname);

        println!("Creating tunnel `{tunnel_name}`...");
        let tunnel_id = self.create_tunnel(&tunnel_name)?;
        println!("  tunnel id: {tunnel_id}");

        println!("Fetching tunnel token...");
        let tunnel_token = self.get_tunnel_token(&tunnel_id)?;

        println!(
            "Configuring ingress: {hostname} → {}",
            creds.origin_service
        );
        self.configure_ingress(&tunnel_id, &hostname, &creds.origin_service)?;

        println!("Ensuring DNS CNAME {hostname} → {tunnel_id}.cfargotunnel.com ...");
        let dns_record_id = self.ensure_tunnel_dns(&hostname, &tunnel_id)?;

        Ok(TunnelState {
            hostname,
            origin_service: creds.origin_service.clone(),
            origin_port: creds.origin_port,
            account_id: self.account_id.clone(),
            zone_id: self.zone_id.clone(),
            tunnel_id,
            tunnel_name,
            tunnel_token,
            api_token: creds.cloudflare_api_token.clone(),
            dns_record_id: Some(dns_record_id),
        })
    }
}

fn format_errors(errors: &[CfError]) -> String {
    if errors.is_empty() {
        return "unknown error".into();
    }
    errors
        .iter()
        .map(|e| format!("[{}] {}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_errors_joins() {
        let errs = vec![
            CfError {
                code: 1000,
                message: "a".into(),
            },
            CfError {
                code: 1001,
                message: "b".into(),
            },
        ];
        let s = format_errors(&errs);
        assert!(s.contains("1000"));
        assert!(s.contains("1001"));
    }
}
