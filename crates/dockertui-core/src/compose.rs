use anyhow::{anyhow, Result};
use tokio::process::Command;
use std::process::Stdio;

#[derive(Debug, Clone)]
pub struct ComposeOptions {
    pub project_dir: String,          // directory containing compose file
    pub file: Option<String>,         // optional -f override
    pub project_name: Option<String>, // optional -p
}

fn base_args(opts: &ComposeOptions) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    if let Some(f) = &opts.file {
        args.push("-f".into());
        args.push(f.clone());
    }
    if let Some(p) = &opts.project_name {
        args.push("-p".into());
        args.push(p.clone());
    }
    args
}

async fn run_capture(opts: &ComposeOptions, extra: &[&str]) -> Result<String> {
    let mut cmd = Command::new("docker");
    cmd.current_dir(&opts.project_dir);

    let mut args = base_args(opts);
    args.extend(extra.iter().map(|s| s.to_string()));
    cmd.args(args);

    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow!(
            "docker compose failed (code={:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub async fn compose_up(opts: &ComposeOptions, detached: bool) -> Result<String> {
    if detached {
        run_capture(opts, &["up", "-d"]).await
    } else {
        run_capture(opts, &["up"]).await
    }
}

pub async fn compose_down(opts: &ComposeOptions) -> Result<String> {
    run_capture(opts, &["down"]).await
}

pub async fn compose_ps(opts: &ComposeOptions) -> Result<String> {
    run_capture(opts, &["ps"]).await
}

pub async fn compose_logs(opts: &ComposeOptions, follow: bool) -> Result<String> {
    if follow {
        run_capture(opts, &["logs", "-f", "--tail", "200"]).await
    } else {
        run_capture(opts, &["logs", "--tail", "200"]).await
    }
}

/// Interactive "compose up" (streams output to terminal).
pub async fn compose_up_interactive(opts: &ComposeOptions, detached: bool) -> Result<()> {
    let mut cmd = Command::new("docker");
    cmd.current_dir(&opts.project_dir);

    let mut args = base_args(opts);
    args.push("up".into());
    if detached {
        args.push("-d".into());
    }
    cmd.args(args);

    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd.status().await?;
    if !status.success() {
        return Err(anyhow!("docker compose up failed (code={:?})", status.code()));
    }
    Ok(())
}
