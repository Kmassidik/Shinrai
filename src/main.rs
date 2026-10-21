mod cloudflare;
mod cloudflared;
mod config;
mod status;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cloudflare::{resolve_zone_for_hostname, CfClient};
use config::{
    build_creds, default_origin_service, load_state, parse_hostname, resolve_interactive,
    save_state, DEFAULT_ORIGIN_PORT,
};
use status::{PublicProbe, TunnelStatus};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "shinrai",
    version,
    about = "Cloudflare Tunnel from the CLI — public host live on the internet, no dashboard.",
    long_about = "shinrai installs cloudflared if needed, creates a Tunnel + DNS via API,\n\
                  then start/stop the connector so your public hostname is reachable."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Welcome text
    Hello,

    /// Install cloudflared if missing (brew or GitHub release)
    Install,

    /// Create tunnel + ingress + DNS (prompts). Does not start the connector yet.
    Init,

    /// Start cloudflared with the saved tunnel token
    Start,

    /// Stop cloudflared (service uninstall / kill background pid)
    Stop,

    /// Show tunnel status (Healthy / Inactive / Down / Degraded) + curl public host
    Status,

    /// init + start + wait for Healthy + curl public host
    Up,

    /// Verify Cloudflare API token + zone (prompts)
    Auth {
        #[command(subcommand)]
        action: AuthCmd,
    },
}

#[derive(Subcommand, Debug)]
enum AuthCmd {
    /// Check token + account + zone
    Verify,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Commands::Hello) => {
            print_welcome();
            Ok(())
        }
        Some(Commands::Install) => cmd_install(),
        Some(Commands::Init) => cmd_init(),
        Some(Commands::Start) => cmd_start(),
        Some(Commands::Stop) => cmd_stop(),
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Up) => cmd_up(),
        Some(Commands::Auth {
            action: AuthCmd::Verify,
        }) => cmd_auth_verify(),
    }
}

fn print_welcome() {
    println!(
        "\
Welcome to shinrai.

Bring a public hostname live on the internet via Cloudflare Tunnel
(cloudflared on this machine — auto-installed if missing).

How to run:
  cargo build --release
  ./target/release/shinrai install
  ./target/release/shinrai up     # prompts: public domain + API token only

Prompts:
  public domain
  Cloudflare API token

Default local origin (uncommon port, rarely clashes):
  {default_origin}

Account id + zone id are resolved from the domain via API.

Success: tunnel Healthy + curl https://your.host → live on the internet.
Failure: Inactive/Down/1033 → start connector; Degraded → :7844; 502 → app not on port {port}
",
        default_origin = default_origin_service(),
        port = DEFAULT_ORIGIN_PORT,
    );
}

fn prompt_and_resolve_creds() -> Result<config::Creds> {
    let (domain, token, port) = resolve_interactive()?;
    let hostname = parse_hostname(&domain)?;
    println!("Resolving Cloudflare zone for `{hostname}`...");
    let (account_id, zone_id, zone_name) = resolve_zone_for_hostname(&token, &hostname)?;
    println!("  zone: {zone_name} ({zone_id})");
    println!("  account: {account_id}");
    println!("  origin: {}", format!("http://127.0.0.1:{port}"));
    Ok(build_creds(domain, token, account_id, zone_id, port))
}

fn cmd_install() -> Result<()> {
    let bin = cloudflared::ensure_installed()?;
    println!("{}", cloudflared::version(&bin)?);
    Ok(())
}

fn cmd_auth_verify() -> Result<()> {
    let creds = prompt_and_resolve_creds()?;
    let host = creds.hostname()?;
    let cf = CfClient::from_creds(&creds)?;
    print!("Verifying API token... ");
    cf.verify_token()?;
    println!("ok");
    print!("Verifying zone... ");
    let zone = cf.verify_zone()?;
    println!("ok ({zone})");
    println!("Public host: {host}");
    println!("Origin: {}", creds.origin_service);
    Ok(())
}

fn cmd_init() -> Result<()> {
    print_welcome();
    cloudflared::ensure_installed()?;

    let creds = prompt_and_resolve_creds()?;
    let cf = CfClient::from_creds(&creds)?;

    print!("Verifying API token... ");
    cf.verify_token()?;
    println!("ok");
    print!("Verifying zone... ");
    let zone = cf.verify_zone()?;
    println!("ok ({zone})");

    let state = cf.provision_tunnel(&creds)?;
    let path = save_state(&state)?;
    println!();
    println!("Tunnel provisioned. State saved to {}", path.display());
    println!("Public URL: {}", state.public_url());
    println!(
        "Serve your app on port {} then: shinrai start",
        state.origin_port
    );
    Ok(())
}

fn cmd_start() -> Result<()> {
    let state = load_state()?;
    let bin = cloudflared::ensure_installed()?;
    println!("Starting tunnel {} ({})", state.tunnel_name, state.tunnel_id);
    cloudflared::start(&bin, &state.tunnel_token)?;
    println!("Waiting for connector Healthy...");
    wait_healthy(&state, 60)?;
    println!("Try:  curl -sS {}", state.public_url());
    Ok(())
}

fn cmd_stop() -> Result<()> {
    let bin = cloudflared::find_cloudflared().context("cloudflared not found — shinrai install")?;
    cloudflared::stop(&bin)?;
    Ok(())
}

fn cmd_status() -> Result<()> {
    let state = load_state()?;
    let cf = CfClient::new(
        state.api_token.clone(),
        state.account_id.clone(),
        state.zone_id.clone(),
    )?;

    let st = cf.get_tunnel_status(&state.tunnel_id)?;
    print_tunnel_status(st);

    println!("Public URL: {}", state.public_url());
    println!("Origin:     {}", state.origin_service);

    match probe_public(&state.public_url()) {
        Ok((code, body, probe)) => {
            println!("curl HTTP {code}");
            println!("{}", probe.guidance());
            if !body.is_empty() {
                let preview: String = body.chars().take(200).collect();
                println!("body preview: {preview}");
            }
            if st.is_success() && probe == PublicProbe::Ok {
                println!("SUCCESS: public domain is live on the internet.");
            } else {
                println!("NOT fully live yet — see guidance above.");
            }
        }
        Err(err) => {
            println!("curl failed: {err}");
            println!("{}", PublicProbe::OtherFailure.guidance());
        }
    }
    Ok(())
}

fn cmd_up() -> Result<()> {
    print_welcome();
    cmd_init()?;
    cmd_start()?;

    let state = load_state()?;
    println!();
    println!("Probing public URL...");
    // DNS may need a moment
    for attempt in 1..=6 {
        match probe_public(&state.public_url()) {
            Ok((code, _body, probe)) if probe == PublicProbe::Ok => {
                println!("curl HTTP {code} — SUCCESS");
                println!("{}", state.public_url());
                println!("Public domain is live on the internet.");
                return Ok(());
            }
            Ok((code, _body, probe)) => {
                println!("attempt {attempt}: HTTP {code} — {}", probe.guidance());
            }
            Err(err) => println!("attempt {attempt}: {err}"),
        }
        thread::sleep(Duration::from_secs(5));
    }
    bail!(
        "tunnel may be Healthy but public URL not answering yet. \
         Check DNS propagation and origin ({}), then: shinrai status",
        state.origin_service
    );
}

fn print_tunnel_status(st: TunnelStatus) {
    let mark = if st.is_success() {
        "SUCCESS"
    } else {
        "FAILURE"
    };
    println!("Tunnel status: {} [{mark}]", st.label());
    println!("{}", st.guidance());
}

fn wait_healthy(state: &config::TunnelState, timeout_secs: u64) -> Result<()> {
    let cf = CfClient::new(
        state.api_token.clone(),
        state.account_id.clone(),
        state.zone_id.clone(),
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let st = cf.get_tunnel_status(&state.tunnel_id)?;
        println!("  status: {}", st.label());
        if st.is_success() {
            print_tunnel_status(st);
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            print_tunnel_status(st);
            bail!("timed out waiting for Healthy");
        }
        thread::sleep(Duration::from_secs(3));
    }
}

fn probe_public(url: &str) -> Result<(u16, String, PublicProbe)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("shinrai/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let res = client.get(url).send().context("curl public URL")?;
    let code = res.status().as_u16();
    let body = res.text().unwrap_or_default();
    let probe = PublicProbe::from_http(code, &body);
    Ok((code, body, probe))
}
