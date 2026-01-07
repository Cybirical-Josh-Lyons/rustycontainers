use anyhow::Result;
use clap::Parser;
use dockertui_core::config::AppConfig;
use dockertui_core::bootstrap;
use dockertui_core::docker_host::resolve_docker_host;
use dockertui_core::engine::Engine;
use dockertui_core::engine_bollard::BollardEngine;
use dockertui_core::engine_wsl_cli::WslCliEngine;
use std::sync::Arc;

mod app;
mod ui;
mod keymap;
mod view_containers;
mod view_logs;
mod view_images;
mod view_volumes;
mod shell;

#[derive(Parser, Debug)]
#[command(name = "dockertui", version, about = "Docker TUI (Ratatui)")]
struct Args {
    #[arg(long)]
    host: Option<String>,

    #[arg(long)]
    wsl: Option<String>,

    /// Run a docker compose action and exit (e.g. up, down, ps, logs)
    #[arg(long)]
    compose: Option<String>,

    /// Directory containing compose.yaml / docker-compose.yml
    #[arg(long, default_value = ".")]
    compose_dir: String,

    /// Optional compose file path (relative to compose_dir or absolute)
    #[arg(long)]
    compose_file: Option<String>,

    /// Optional compose project name (-p)
    #[arg(long)]
    compose_project: Option<String>,
}


#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Bootstrap BEFORE entering raw-mode TUI so sudo prompts behave normally.
    // (This is for Linux/WSL builds. We'll add Windows-host -> wsl.exe bootstrap next.)
    if args.wsl.is_none() {
        println!("dockertui: ensuring Docker engine is installed and running...");
        let report = bootstrap::ensure_docker_ready_native().await?;
        for n in &report.notes {
            println!("  - {n}");
        }
        if !report.started {
            println!("dockertui: Docker engine is not ready. Exiting.");
            // You can choose to continue into UI with error state instead.
            std::process::exit(1);
        }
    }

    if let Some(action) = args.compose.clone() {
        use dockertui_core::compose::*;
        let opts = ComposeOptions {
            project_dir: args.compose_dir.clone(),
            file: args.compose_file.clone(),
            project_name: args.compose_project.clone(),
        };

        match action.as_str() {
            "up" => {
                compose_up_interactive(&opts, true).await?;
            }
            "down" => {
                let out = compose_down(&opts).await?;
                println!("{out}");
            }
            "ps" => {
                let out = compose_ps(&opts).await?;
                println!("{out}");
            }
            "logs" => {
                let out = compose_logs(&opts, false).await?;
                println!("{out}");
            }
            "logs-follow" => {
                // This version captures; for a true follow you’d use interactive streaming.
                let out = compose_logs(&opts, true).await?;
                println!("{out}");
            }
            other => {
                anyhow::bail!("Unknown --compose action: {other}. Use up|down|ps|logs|logs-follow");
            }
        }

        return Ok(());
    }

    let cfg = AppConfig {
        docker_host: args.host.clone(),
        wsl_distro: args.wsl.clone(),
    };

    let engine: Arc<dyn Engine> = if let Some(distro) = cfg.wsl_distro.clone() {
        Arc::new(WslCliEngine::new(distro))
    } else {
        let host = resolve_docker_host(cfg.docker_host.as_deref());
        Arc::new(BollardEngine::connect(host.as_str())?)
    };

    app::run(engine).await
}
