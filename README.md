# Rusty Containers

Retro Docker control deck in your terminal. Manage containers, images, volumes,
logs, and the engine itself without leaving the CRT glow.

## Features
- Fast TUI navigation for containers/images/volumes
- Tail logs in-place with smooth scroll + mouse wheel
- Exec shell into containers
- Start/stop/restart containers
- Start/stop/restart the Docker engine (best-effort per platform)
- WSL CLI mode for Windows workflows

## How It Works (Under the Hood)
- `dockertui` is the TUI shell (Ratatui + Crossterm) that renders the UI and
  dispatches actions.
- `dockertui-core` handles Docker engine integration and models.
- Engine access:
  - Linux/Unix socket mode uses `bollard` (async Docker API client).
  - WSL mode shells out to `docker` via `wsl.exe` for a specific distro.
- Async runtime: `tokio` powers UI tasks, log streaming, and engine commands.
- Logs: streamed from the engine, sanitized, and rendered into a scrollable pane.

## Project Layout
- `crates/dockertui/` TUI app entry point and UI (`src/main.rs`, `src/ui.rs`)
- `crates/dockertui-core/` engine integration and models (`src/engine*.rs`)
- `Cargo.toml` workspace metadata

## Run
Linux:
- Ensure docker daemon is running and your user can access `/var/run/docker.sock`
- `cargo run -p dockertui`

Windows (Docker API / npipe):
- Requires an engine exposing npipe (Docker Desktop or equivalent)
- `cargo run -p dockertui`

Windows (WSL mode):
- Requires Docker inside your WSL distro
- `cargo run -p dockertui -- --wsl Ubuntu-22.04`

## Keys
- `Tab` / `Shift+Tab` switch tabs
- `Up`/`Down` select row
- `S` start container
- `X` stop container
- `Shift+S` start all containers
- `Shift+X` stop all containers
- `R` restart container
- `L` logs
- `F` toggle log follow
- Mouse wheel scroll logs
- `E` shell (docker exec -it)
- `Del` remove (images/volumes)
- `F5` refresh
- `Q` quit
- `Ctrl+E` start engine (best-effort)
- `Ctrl+X` stop engine (hard stop)
- `Ctrl+R` restart engine

## Notes
- WSL + Docker Desktop: engine stop blocks the WSL socket to emulate Desktop
  shutdown behavior; start restores it.
- Engine commands are best-effort and vary by platform/systemd availability.

## Build / Test
- `cargo build -p dockertui`
- `cargo test`
