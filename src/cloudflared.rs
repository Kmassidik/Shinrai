use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Locate cloudflared on PATH or under ~/.local/bin.
pub fn find_cloudflared() -> Option<PathBuf> {
    if let Ok(path) = which("cloudflared") {
        return Some(path);
    }
    let local = dirs::home_dir()?.join(".local/bin/cloudflared");
    if local.is_file() {
        return Some(local);
    }
    None
}

fn which(bin: &str) -> Result<PathBuf, ()> {
    let output = Command::new("which")
        .arg(bin)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Err(())
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Install cloudflared if missing (brew on macOS, else GitHub release binary).
pub fn ensure_installed() -> Result<PathBuf> {
    if let Some(path) = find_cloudflared() {
        println!("cloudflared found: {}", path.display());
        return Ok(path);
    }

    println!("cloudflared not found — installing...");

    #[cfg(target_os = "macos")]
    {
        if which("brew").is_ok() {
            println!("Using Homebrew: brew install cloudflared");
            let status = Command::new("brew")
                .args(["install", "cloudflared"])
                .status()
                .context("brew install cloudflared")?;
            if status.success() {
                return find_cloudflared()
                    .context("cloudflared installed via brew but not on PATH");
            }
            println!("brew install failed — falling back to GitHub release binary");
        }
        return install_from_github();
    }

    #[cfg(target_os = "linux")]
    {
        return install_from_github();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        bail!(
            "automatic cloudflared install is only supported on macOS/Linux. \
             Install from https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/"
        );
    }
}

fn install_from_github() -> Result<PathBuf> {
    let (os, arch) = platform_asset()?;
    let asset = format!("cloudflared-{os}-{arch}.tgz");
    // For linux, releases use bare binary names without .tgz for some arches
    let url = if os == "linux" {
        format!(
            "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-{os}-{arch}"
        )
    } else {
        format!(
            "https://github.com/cloudflare/cloudflared/releases/latest/download/{asset}"
        )
    };

    let dest_dir = dirs::home_dir()
        .context("home dir")?
        .join(".local/bin");
    fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join("cloudflared");

    println!("Downloading {url}");
    let tmp = dest_dir.join("cloudflared.download");

    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&tmp)
        .arg(&url)
        .status()
        .context("curl download cloudflared")?;
    if !status.success() {
        bail!("failed to download cloudflared from GitHub releases");
    }

    if os == "linux" {
        fs::rename(&tmp, &dest).context("move cloudflared binary")?;
    } else {
        // darwin .tgz
        let extract = Command::new("tar")
            .args(["-xzf"])
            .arg(&tmp)
            .arg("-C")
            .arg(&dest_dir)
            .status()
            .context("tar extract cloudflared")?;
        if !extract.success() {
            bail!("failed to extract cloudflared archive");
        }
        let _ = fs::remove_file(&tmp);
        // tarball usually contains ./cloudflared
        let extracted = dest_dir.join("cloudflared");
        if extracted != dest && extracted.is_file() {
            // already in place
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }

    println!("Installed: {}", dest.display());
    println!("Ensure ~/.local/bin is on your PATH");
    Ok(dest)
}

fn platform_asset() -> Result<(&'static str, &'static str)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        other => bail!("unsupported OS for auto-install: {other}"),
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => bail!("unsupported arch for auto-install: {other}"),
    };
    Ok((os, arch))
}

pub fn version(bin: &Path) -> Result<String> {
    let out = Command::new(bin)
        .arg("--version")
        .output()
        .context("cloudflared --version")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Start connector with tunnel token.
/// Prefers `cloudflared service install` (needs sudo); falls back to background `tunnel run`.
pub fn start(bin: &Path, tunnel_token: &str) -> Result<()> {
    // Try service install first (docs: sudo cloudflared service install <TOKEN>)
    let service = Command::new("sudo")
        .arg(bin)
        .args(["service", "install"])
        .arg(tunnel_token)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match service {
        Ok(status) if status.success() => {
            println!("cloudflared service installed and started.");
            return Ok(());
        }
        Ok(status) => {
            println!(
                "service install exited {} — falling back to background process",
                status
            );
        }
        Err(err) => {
            println!("sudo/service unavailable ({err}) — falling back to background process");
        }
    }

    start_background(bin, tunnel_token)
}

fn start_background(bin: &Path, tunnel_token: &str) -> Result<()> {
    let pid_path = pid_file()?;
    if pid_path.is_file() {
        let old = fs::read_to_string(&pid_path).unwrap_or_default();
        println!("Existing pid file {} ({}) — stopping first", pid_path.display(), old.trim());
        let _ = stop(bin);
    }

    let log_path = crate::config::config_dir()?.join("cloudflared.log");
    let log = fs::File::create(&log_path).context("create cloudflared.log")?;
    let log_err = log.try_clone()?;

    let child = Command::new(bin)
        .args(["tunnel", "--no-autoupdate", "run", "--token"])
        .arg(tunnel_token)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .context("spawn cloudflared tunnel run")?;

    let pid = child.id();
    fs::write(&pid_path, pid.to_string())?;
    // detach: don't wait
    std::mem::forget(child);

    println!("cloudflared started in background (pid {pid})");
    println!("logs: {}", log_path.display());
    // brief wait for first connections
    thread::sleep(Duration::from_secs(2));
    Ok(())
}

pub fn stop(bin: &Path) -> Result<()> {
    // Try service uninstall
    let _ = Command::new("sudo")
        .arg(bin)
        .args(["service", "uninstall"])
        .status();

    let pid_path = pid_file()?;
    if pid_path.is_file() {
        let pid = fs::read_to_string(&pid_path)?.trim().to_string();
        if !pid.is_empty() {
            let _ = Command::new("kill").arg(&pid).status();
            println!("killed cloudflared pid {pid}");
        }
        let _ = fs::remove_file(&pid_path);
    }

    // Best-effort: pkill by name+token is too aggressive; leave service/pid paths.
    println!("stop requested.");
    Ok(())
}

fn pid_file() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("cloudflared.pid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_asset_known() {
        // Just ensure function doesn't panic on this host
        let r = platform_asset();
        assert!(r.is_ok(), "{r:?}");
    }
}
