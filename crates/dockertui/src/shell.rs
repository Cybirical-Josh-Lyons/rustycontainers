use anyhow::{Result};
use dockertui_core::engine::EngineKind;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::Write;

#[allow(dead_code)]
pub struct ShellSession {
    pub master: Box<dyn MasterPty>,
    pub writer: Box<dyn Write>,
    pub child: Box<dyn portable_pty::Child + Send>,
}

pub fn spawn_shell_pty(kind: EngineKind, id: &str, cols: u16, rows: u16) -> Result<ShellSession> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let cmd = build_shell_command(kind, id);
    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let writer = pair.master.take_writer()?;

    Ok(ShellSession {
        master: pair.master,
        writer,
        child,
    })
}

fn build_shell_command(kind: EngineKind, id: &str) -> CommandBuilder {
    let shell_bootstrap = "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi";
    match kind {
        EngineKind::DockerApi => {
            let mut cmd = CommandBuilder::new("docker");
            cmd.arg("exec");
            cmd.arg("-it");
            cmd.arg(id);
            cmd.arg("sh");
            cmd.arg("-lc");
            cmd.arg(shell_bootstrap);
            cmd
        }
        EngineKind::WslCli => {
            let mut cmd = CommandBuilder::new("wsl.exe");
            cmd.arg("--");
            cmd.arg("docker");
            cmd.arg("exec");
            cmd.arg("-it");
            cmd.arg(id);
            cmd.arg("sh");
            cmd.arg("-lc");
            cmd.arg(shell_bootstrap);
            cmd
        }
    }
}
