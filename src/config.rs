use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use url::Url;

/// Uncommon local port so we don't collide with 80/443/3000/8080 apps.
pub const DEFAULT_ORIGIN_PORT: u16 = 47821;

pub fn default_origin_service() -> String {
    format!("http://127.0.0.1:{DEFAULT_ORIGIN_PORT}")
}

fn default_origin_port_serde() -> u16 {
    DEFAULT_ORIGIN_PORT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Creds {
    pub public_base_url: String,
    pub cloudflare_api_token: String,
    pub cloudflare_account_id: String,
    pub cloudflare_zone_id: String,
    /// Always `http://127.0.0.1:{DEFAULT_ORIGIN_PORT}` unless overridden later.
    pub origin_service: String,
    pub origin_port: u16,
}

impl Creds {
    pub fn hostname(&self) -> Result<String> {
        parse_hostname(&self.public_base_url)
    }
}

pub fn parse_hostname(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme).context("invalid public host URL")?;
    let host = url
        .host_str()
        .context("public host must include a hostname")?
        .to_string();
    if host.is_empty() {
        bail!("public host hostname is empty");
    }
    Ok(host)
}

/// Candidate zone names for `app.example.com` → `app.example.com`, `example.com`, …
pub fn zone_name_candidates(hostname: &str) -> Vec<String> {
    let parts: Vec<&str> = hostname.split('.').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return vec![hostname.to_string()];
    }
    let mut out = Vec::new();
    for i in 0..=parts.len().saturating_sub(2) {
        out.push(parts[i..].join("."));
    }
    out
}

/// Prompt: public domain + API token only. Port is fixed default. Account/zone filled by API later.
pub fn resolve_interactive() -> Result<(String, String, u16)> {
    println!("Enter details in this terminal:\n");
    println!("Default origin port: {DEFAULT_ORIGIN_PORT}  →  {}", default_origin_service());
    println!("(Point your local app at that port — uncommon so it rarely clashes.)\n");

    let public_domain = prompt("Public domain (e.g. app.example.com)", false)?;
    let api_token = prompt("Cloudflare API token", true)?;

    Ok((public_domain, api_token, DEFAULT_ORIGIN_PORT))
}

pub fn build_creds(
    public_domain: String,
    api_token: String,
    account_id: String,
    zone_id: String,
    port: u16,
) -> Creds {
    Creds {
        public_base_url: public_domain,
        cloudflare_api_token: api_token,
        cloudflare_account_id: account_id,
        cloudflare_zone_id: zone_id,
        origin_service: format!("http://127.0.0.1:{port}"),
        origin_port: port,
    }
}

fn prompt(label: &str, secret: bool) -> Result<String> {
    eprint!("{label}: ");
    io::stderr().flush()?;

    let value = if secret {
        rpassword::read_password().context("failed to read secret input")?
    } else {
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .context("failed to read input")?;
        line
    };

    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

/// Persisted tunnel state (mode 600).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelState {
    pub hostname: String,
    pub origin_service: String,
    #[serde(default = "default_origin_port_serde")]
    pub origin_port: u16,
    pub account_id: String,
    pub zone_id: String,
    pub tunnel_id: String,
    pub tunnel_name: String,
    pub tunnel_token: String,
    pub api_token: String,
    pub dns_record_id: Option<String>,
}

impl TunnelState {
    pub fn public_url(&self) -> String {
        format!("https://{}", self.hostname)
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not resolve config dir")?
        .join("shinrai");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn state_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("tunnel.toml"))
}

pub fn save_state(state: &TunnelState) -> Result<PathBuf> {
    let path = state_path()?;
    let text = toml::to_string_pretty(state).context("serialize tunnel state")?;
    fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

pub fn load_state() -> Result<TunnelState> {
    let path = state_path()?;
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no tunnel state at {} — run `shinrai init` first",
            path.display()
        )
    })?;
    toml::from_str(&text).context("parse tunnel.toml")
}

pub fn tunnel_name_for_host(hostname: &str) -> String {
    let slug: String = hostname
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_lowercase();
    let name = format!("shinrai-{slug}");
    name.chars().take(58).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_uncommon() {
        assert_eq!(DEFAULT_ORIGIN_PORT, 47821);
        assert!(!matches!(
            DEFAULT_ORIGIN_PORT,
            80 | 443 | 3000 | 8000 | 8080 | 5173
        ));
    }

    #[test]
    fn parse_hostname_strips_scheme_and_path() {
        assert_eq!(
            parse_hostname("https://app.example.com/foo").unwrap(),
            "app.example.com"
        );
        assert_eq!(
            parse_hostname("app.example.com").unwrap(),
            "app.example.com"
        );
    }

    #[test]
    fn zone_candidates_walk_parents() {
        let c = zone_name_candidates("api.app.example.com");
        assert_eq!(
            c,
            vec![
                "api.app.example.com".to_string(),
                "app.example.com".to_string(),
                "example.com".to_string(),
            ]
        );
    }

    #[test]
    fn tunnel_name_is_slug() {
        let n = tunnel_name_for_host("app.example.com");
        assert!(n.starts_with("shinrai-"));
        assert!(!n.contains('.'));
    }
}
