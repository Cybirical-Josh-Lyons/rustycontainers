use anyhow::{anyhow, Result};
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unknown,
}

async fn sh(cmd: &str) -> Result<(i32, String, String)> {
    let out = Command::new("sh").arg("-lc").arg(cmd).output().await?;
    let code = out.status.code().unwrap_or(-1);
    Ok((
        code,
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

async fn has_systemd() -> bool {
    sh("command -v systemctl >/dev/null 2>&1 && pidof systemd >/dev/null 2>&1")
        .await
        .map(|(c,_,_)| c==0)
        .unwrap_or(false)
}

async fn is_wsl() -> bool {
    sh("grep -qi microsoft /proc/sys/kernel/osrelease /proc/version >/dev/null 2>&1")
        .await
        .map(|(c,_,_)| c==0)
        .unwrap_or(false)
}

async fn block_docker_socket() -> Result<()> {
    // WSL + Docker Desktop exposes a proxy socket; block it so CLI behaves like engine is stopped.
    let _ = sh("if mount | grep -q ' /var/run/docker.sock '; then sudo umount /var/run/docker.sock; fi").await?;
    let _ = sh("sudo sh -lc 'if [ -L /var/run/docker.sock ] || [ -S /var/run/docker.sock ]; then mv /var/run/docker.sock /var/run/docker.sock.disabled 2>/dev/null || true; fi'").await?;
    let _ = sh("sudo sh -lc 'if [ -e /var/run/docker.sock ]; then rm -rf /var/run/docker.sock; fi'").await?;
    let _ = sh("sudo sh -lc 'mkdir -p /var/run/docker.sock'").await?;
    Ok(())
}

async fn unblock_docker_socket() -> Result<()> {
    let _ = sh("sudo sh -lc 'if [ -d /var/run/docker.sock ]; then rmdir /var/run/docker.sock 2>/dev/null || true; fi'").await?;
    let _ = sh("sudo sh -lc 'if [ -e /var/run/docker.sock.disabled ]; then mv /var/run/docker.sock.disabled /var/run/docker.sock 2>/dev/null || true; fi'").await?;
    Ok(())
}

pub async fn status() -> Result<DaemonState> {
    if has_systemd().await {
        let (code, out, _) = sh("systemctl is-active docker").await?;
        if code == 0 && out.trim() == "active" {
            return Ok(DaemonState::Running);
        }
        // inactive/failed/activating/deactivating etc.
        return Ok(match out.trim() {
            "inactive" | "failed" => DaemonState::Stopped,
            "activating" => DaemonState::Starting,
            "deactivating" => DaemonState::Stopping,
            _ => DaemonState::Unknown,
        });
    }

    // no systemd: check dockerd process
    let (code, _, _) = sh("pidof dockerd >/dev/null 2>&1").await?;
    Ok(if code == 0 { DaemonState::Running } else { DaemonState::Stopped })
}

pub async fn start() -> Result<()> {
    if has_systemd().await {
        let (code, _o, e) = sh("sudo systemctl start docker").await?;
        if code != 0 { return Err(anyhow!("start docker failed: {e}")); }
        return Ok(());
    }
    let (code, _o, e) = sh("sudo nohup dockerd >/tmp/dockerd.log 2>&1 &").await?;
    if code != 0 { return Err(anyhow!("start dockerd failed: {e}")); }
    Ok(())
}

pub async fn stop() -> Result<()> {
    if has_systemd().await {
        let (code, _o, e) = sh("sudo systemctl stop docker").await?;
        if code != 0 { return Err(anyhow!("stop docker failed: {e}")); }
        return Ok(());
    }
    let (code, _o, e) = sh("sudo pkill -f 'dockerd'").await?;
    if code != 0 { return Err(anyhow!("stop dockerd failed: {e}")); }
    Ok(())
}

pub async fn restart() -> Result<()> {
    if has_systemd().await {
        let (code, _o, e) = sh("sudo systemctl restart docker").await?;
        if code != 0 { return Err(anyhow!("restart docker failed: {e}")); }
        return Ok(());
    }
    stop().await?;
    start().await?;
    Ok(())
}

pub async fn stop_hard() -> Result<()> {
    if has_systemd().await {
        // Best-effort stop/mask for rootless user service if present.
        let _ = sh("systemctl --user stop docker.service docker.socket >/dev/null 2>&1").await;
        let _ = sh("systemctl --user mask docker.service docker.socket >/dev/null 2>&1").await;

        // stop both and prevent socket activation from restarting it
        let _ = sh("sudo systemctl stop docker.service docker.socket").await?;
        let _ = sh("sudo systemctl mask docker.service docker.socket").await?;

        if is_wsl().await {
            let _ = block_docker_socket().await;
        }
        return Ok(());
    }

    // no systemd fallback
    stop().await?;
    if is_wsl().await {
        let _ = block_docker_socket().await;
    }
    Ok(())
}

pub async fn start_hard() -> Result<()> {
    if has_systemd().await {
        // Best-effort unmask/start for rootless user service if present.
        let _ = sh("systemctl --user unmask docker.service docker.socket >/dev/null 2>&1").await;
        let _ = sh("systemctl --user start docker.service docker.socket >/dev/null 2>&1").await;

        if is_wsl().await {
            let _ = unblock_docker_socket().await;
        }
        // allow start again
        let _ = sh("sudo systemctl unmask docker.service docker.socket").await?;
        let _ = sh("sudo systemctl start docker.service").await?;
        return Ok(());
    }

    if is_wsl().await {
        let _ = unblock_docker_socket().await;
    }
    start().await
}
