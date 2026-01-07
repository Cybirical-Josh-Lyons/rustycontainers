use anyhow::{anyhow, Context, Result};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct BootstrapReport {
    pub docker_installed: bool,
    pub dockerd_installed: bool,
    pub started: bool,
    pub used_systemd: bool,
    pub notes: Vec<String>,
}

fn shell_cmd(cmd: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-lc").arg(cmd);
    c
}

async fn run(cmd: &str) -> Result<(i32, String, String)> {
    let out = shell_cmd(cmd).output().await.context(cmd.to_string())?;
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok((code, stdout, stderr))
}

async fn ok(cmd: &str) -> Result<bool> {
    let (code, _, _) = run(cmd).await?;
    Ok(code == 0)
}

async fn ensure_sudo() -> Result<()> {
    // This will prompt for password if needed and cache credentials.
    let (code, _o, e) = run("sudo -v").await?;
    if code != 0 {
        return Err(anyhow!("sudo authentication failed: {e}"));
    }
    Ok(())
}

async fn is_systemd_running() -> Result<bool> {
    // WSL with systemd enabled will have PID 1 = systemd typically.
    // This check is pragmatic: systemctl exists + pidof systemd returns something.
    let has_systemctl = ok("command -v systemctl >/dev/null 2>&1").await?;
    if !has_systemctl {
        return Ok(false);
    }
    ok("pidof systemd >/dev/null 2>&1").await
}

async fn docker_ready() -> Result<bool> {
    // Prefer docker CLI check, because it confirms client+daemon handshake.
    // If docker CLI isn't available, this will fail (handled by caller).
    Ok(ok("docker info >/dev/null 2>&1").await?)
}

async fn wait_for_docker(timeout: Duration) -> Result<bool> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if docker_ready().await.unwrap_or(false) {
            return Ok(true);
        }
        sleep(Duration::from_millis(500)).await;
    }
    Ok(false)
}

async fn install_docker_convenience_script(notes: &mut Vec<String>) -> Result<()> {
    ensure_sudo().await?;

    notes.push("Installing Docker using get.docker.com convenience script...".into());

    // This installs docker engine + cli on common distros (Ubuntu/Debian/etc).
    let cmd = "curl -fsSL https://get.docker.com | sudo sh";
    let (code, _o, e) = run(cmd).await?;
    if code != 0 {
        return Err(anyhow!("Docker install failed: {e}"));
    }

    Ok(())
}

async fn start_with_systemd(notes: &mut Vec<String>) -> Result<()> {
    ensure_sudo().await?;
    notes.push("Starting Docker via systemd...".into());

    // If the socket was blocked (e.g. WSL), unblock before starting.
    let _ = run("sudo sh -lc 'if [ -d /var/run/docker.sock ]; then rmdir /var/run/docker.sock 2>/dev/null || true; fi'").await?;
    let _ = run("sudo sh -lc 'if [ -e /var/run/docker.sock.disabled ]; then mv /var/run/docker.sock.disabled /var/run/docker.sock 2>/dev/null || true; fi'").await?;

    // Docker may have been masked by a previous stop_hard; unmask before enable/start.
    let _ = run("sudo systemctl unmask docker.service docker.socket").await?;

    let (code, _o, e) = run("sudo systemctl enable --now docker").await?;
    if code != 0 {
        // show journal hint
        return Err(anyhow!(
            "Failed to start docker via systemd: {e}\nTry: sudo systemctl status docker --no-pager"
        ));
    }
    Ok(())
}

async fn start_without_systemd(notes: &mut Vec<String>) -> Result<()> {
    ensure_sudo().await?;
    notes.push("systemd not detected; starting dockerd in background (nohup)...".into());

    // If the socket was blocked (e.g. WSL), unblock before starting.
    let _ = run("sudo sh -lc 'if [ -d /var/run/docker.sock ]; then rmdir /var/run/docker.sock 2>/dev/null || true; fi'").await?;
    let _ = run("sudo sh -lc 'if [ -e /var/run/docker.sock.disabled ]; then mv /var/run/docker.sock.disabled /var/run/docker.sock 2>/dev/null || true; fi'").await?;

    // Kill any stale dockerd first (best-effort)
    let _ = run("sudo pkill -f 'dockerd' >/dev/null 2>&1 || true").await;

    // Start dockerd in background
    // Logs: /tmp/dockerd.log
    let cmd = "sudo nohup dockerd >/tmp/dockerd.log 2>&1 &";
    let (code, _o, e) = run(cmd).await?;
    if code != 0 {
        return Err(anyhow!("Failed to start dockerd (nohup): {e}"));
    }
    Ok(())
}

async fn compose_installed() -> Result<bool> {
    // Compose v2: `docker compose version` works if plugin installed
    Ok(ok("docker compose version >/dev/null 2>&1").await?)
}

async fn ensure_user_in_docker_group(notes: &mut Vec<String>) -> Result<()> {
    // Only meaningful on Linux/WSL where docker group exists.
    // This may require the user to restart their shell session to take effect.
    ensure_sudo().await?;

    let _ = run("sudo groupadd docker >/dev/null 2>&1 || true").await;

    // Check membership
    let in_group = ok("id -nG \"$USER\" | tr ' ' '\\n' | grep -qx docker").await?;
    if in_group {
        return Ok(());
    }

    notes.push("Adding current user to docker group (may require new shell / re-login)...".into());
    let (code, _o, e) = run("sudo usermod -aG docker \"$USER\"").await?;
    if code != 0 {
        return Err(anyhow!("Failed to add user to docker group: {e}"));
    }
    Ok(())
}

pub async fn ensure_docker_ready_native() -> Result<BootstrapReport> {
    let mut notes = vec![];

    let docker_installed = ok("command -v docker >/dev/null 2>&1").await?;
    let dockerd_installed = ok("command -v dockerd >/dev/null 2>&1").await?;

    if !docker_installed || !dockerd_installed {
        install_docker_convenience_script(&mut notes).await?;
    }

    // Re-check after install
    let docker_installed = ok("command -v docker >/dev/null 2>&1").await?;
    let dockerd_installed = ok("command -v dockerd >/dev/null 2>&1").await?;

    if !docker_installed || !dockerd_installed {
        return Err(anyhow!(
            "Docker install did not complete successfully (docker_installed={docker_installed}, dockerd_installed={dockerd_installed})"
        ));
    }

    // Start engine
    let used_systemd = is_systemd_running().await?;
    if used_systemd {
        // If already ready, skip start
        if !docker_ready().await.unwrap_or(false) {
            start_with_systemd(&mut notes).await?;
        }
    } else {
        if !docker_ready().await.unwrap_or(false) {
            start_without_systemd(&mut notes).await?;
        }
    }

    // Wait for daemon
    let started = wait_for_docker(Duration::from_secs(20)).await?;
    if !started {
        notes.push("Docker did not become ready within timeout.".into());
        notes.push("If systemd is disabled in WSL, consider enabling it in /etc/wsl.conf and restarting WSL.".into());
        notes.push("If using nohup mode, check: /tmp/dockerd.log".into());
    } else {
        // group membership improvement is optional once daemon is up
        let _ = ensure_user_in_docker_group(&mut notes).await;
    }

    // Ensure compose v2 plugin is available
    if !compose_installed().await.unwrap_or(false) {
        // On systems installed via get.docker.com, compose plugin is usually present,
        // but if it isn't, install explicitly (Ubuntu/Debian packages).
        let _ = ensure_sudo().await;
        let _ = run("sudo apt-get update >/dev/null 2>&1 || true").await;
        let _ = run("sudo apt-get install -y docker-compose-plugin >/dev/null 2>&1 || true").await;
        if compose_installed().await.unwrap_or(false) {
            notes.push("Installed docker-compose-plugin (Compose v2).".into());
        } else {
            notes.push("Compose v2 not detected; compose features may be unavailable.".into());
        }
    }

    Ok(BootstrapReport {
        docker_installed,
        dockerd_installed,
        started,
        used_systemd,
        notes,
    })
}
