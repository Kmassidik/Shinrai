//! Tunnel connection states from Cloudflare docs:
//! https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/troubleshoot-tunnels/common-errors/

use std::fmt;

/// Cloudflare Tunnel connector status (`status` field on the tunnel object).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelStatus {
    /// Active and serving traffic (success for connector).
    Healthy,
    /// Tunnel exists but cloudflared has never connected.
    Inactive,
    /// Was connected; cloudflared process stopped.
    Down,
    /// Serving but at least one connection failed.
    Degraded,
    /// Unknown / future API value.
    Other,
}

impl TunnelStatus {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "healthy" => Self::Healthy,
            "inactive" => Self::Inactive,
            "down" => Self::Down,
            "degraded" => Self::Degraded,
            _ => Self::Other,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Inactive => "inactive",
            Self::Down => "down",
            Self::Degraded => "degraded",
            Self::Other => "other",
        }
    }

    /// Bottleneck + recommended action (from Cloudflare Tunnel common-errors docs).
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Healthy => {
                "Tunnel connector is Healthy — serving traffic to Cloudflare. \
                 If the public URL still fails, check origin (502) or DNS (1033)."
            }
            Self::Inactive => {
                "Tunnel is Inactive — created but cloudflared never connected. \
                 Run: shinrai start (installs/runs the connector)."
            }
            Self::Down => {
                "Tunnel is Down — connector was connected but stopped. \
                 Run: shinrai start. Check the machine is on and cloudflared is running."
            }
            Self::Degraded => {
                "Tunnel is Degraded — partial connections. \
                 Check firewall allows outbound to Cloudflare on port 7844; review cloudflared logs."
            }
            Self::Other => "Unknown tunnel status from API — check Networking > Tunnels in the dashboard.",
        }
    }
}

impl fmt::Display for TunnelStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Classify HTTP outcome when curling the public hostname.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicProbe {
    Ok,
    /// Cloudflare error 1033 — no healthy cloudflared to receive traffic.
    Error1033,
    /// Tunnel up but origin unreachable from cloudflared.
    BadGateway502,
    OtherFailure,
}

impl PublicProbe {
    pub fn from_http(status: u16, body: &str) -> Self {
        if (200..300).contains(&status) {
            return Self::Ok;
        }
        let lower = body.to_ascii_lowercase();
        if status == 530 || lower.contains("1033") || lower.contains("error 1033") {
            return Self::Error1033;
        }
        if status == 502 {
            return Self::BadGateway502;
        }
        Self::OtherFailure
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::Ok => "Public hostname responds on the internet.",
            Self::Error1033 => {
                "Cloudflare 1033: no healthy tunnel connector. Fix with shinrai start \
                 (Inactive/Down) and confirm DNS CNAME points at <tunnel_id>.cfargotunnel.com."
            }
            Self::BadGateway502 => {
                "502: tunnel is connected but cloudflared cannot reach your origin. \
                 Start the local app and check the Service URL (host:port / http vs https)."
            }
            Self::OtherFailure => "Public hostname did not return success — see HTTP status and body.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_statuses() {
        assert_eq!(TunnelStatus::parse("healthy"), TunnelStatus::Healthy);
        assert_eq!(TunnelStatus::parse("INACTIVE"), TunnelStatus::Inactive);
        assert_eq!(TunnelStatus::parse("Down"), TunnelStatus::Down);
        assert_eq!(TunnelStatus::parse("degraded"), TunnelStatus::Degraded);
        assert_eq!(TunnelStatus::parse("weird"), TunnelStatus::Other);
    }

    #[test]
    fn healthy_is_success_others_are_not() {
        assert!(TunnelStatus::Healthy.is_success());
        assert!(!TunnelStatus::Inactive.is_success());
        assert!(!TunnelStatus::Down.is_success());
        assert!(!TunnelStatus::Degraded.is_success());
    }

    #[test]
    fn guidance_mentions_cloudflared_for_inactive() {
        let g = TunnelStatus::Inactive.guidance();
        assert!(g.contains("cloudflared") || g.contains("shinrai start"));
    }

    #[test]
    fn probe_detects_1033_and_502() {
        assert_eq!(
            PublicProbe::from_http(200, "ok"),
            PublicProbe::Ok
        );
        assert_eq!(
            PublicProbe::from_http(530, "error code: 1033"),
            PublicProbe::Error1033
        );
        assert_eq!(
            PublicProbe::from_http(502, "Unable to reach the origin service"),
            PublicProbe::BadGateway502
        );
    }
}
