use std::pin::Pin;
use anyhow::{anyhow, Result};
use crate::engine::{Engine, EngineKind, LogsOptions};
use crate::models::{ContainerRow, ImageRow, VolumeRow};
use futures::{stream, Stream};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct WslCliEngine {
    distro: String,
}

impl WslCliEngine {
    pub fn new(distro: String) -> Self {
        Self { distro }
    }

    fn wsl_base(&self) -> Command {
        let mut cmd = Command::new("wsl.exe");
        cmd.arg("-d").arg(&self.distro).arg("--");
        cmd
    }

    async fn run_lines(&self, args: &[&str]) -> Result<Vec<String>> {
        let mut cmd = self.wsl_base();
        for a in args {
            cmd.arg(a);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let mut reader = BufReader::new(stdout).lines();

        let mut out = vec![];
        while let Some(line) = reader.next_line().await? {
            out.push(line);
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(anyhow!("wsl command failed: {:?}", status.code()));
        }
        Ok(out)
    }
}

impl Engine for WslCliEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::WslCli
    }

    fn name(&self) -> String {
        format!("WSL CLI ({})", self.distro)
    }

    fn list_containers(&self) -> tokio::task::JoinHandle<Result<Vec<ContainerRow>>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            // docker ps -a --format '{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}'
            let lines = eng.run_lines(&[
                "docker","ps","-a","--format",
                "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}"
            ]).await?;
            let mut rows = vec![];
            for l in lines {
                let parts: Vec<&str> = l.split('\t').collect();
                if parts.len() >= 5 {
                    rows.push(ContainerRow {
                        id: parts[0].to_string(),
                        name: parts[1].to_string(),
                        image: parts[2].to_string(),
                        state: parts[3].to_string(),
                        status: parts[4].to_string(),
                    });
                }
            }
            Ok(rows)
        })
    }

    fn start_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            eng.run_lines(&["docker","start",&id]).await?;
            Ok(())
        })
    }

    fn stop_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            eng.run_lines(&["docker","stop",&id]).await?;
            Ok(())
        })
    }

    fn restart_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            eng.run_lines(&["docker","restart",&id]).await?;
            Ok(())
        })
    }

    fn container_logs(
        &self,
        id: String,
        opts: LogsOptions,
    ) -> tokio::task::JoinHandle<Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>>> {

        let distro = self.distro.clone();
        tokio::spawn(async move {
            // For MVP: we won’t do true streaming follow here (it’s doable, just longer).
            // We’ll fetch last N lines.
            let eng = WslCliEngine::new(distro);
            let tail = opts.tail.unwrap_or_else(|| "200".to_string());
            let lines = eng.run_lines(&["docker","logs","--tail",&tail,&id]).await?;
            let joined = lines.join("\n");
            let s = stream::once(async move { Ok(joined) });
            Ok(Box::pin(s) as Pin<Box<dyn Stream<Item = Result<String>> + Send>>)
        })
    }

    fn list_images(&self) -> tokio::task::JoinHandle<Result<Vec<ImageRow>>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            // docker images --format '{{.ID}}\t{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}'
            let lines = eng.run_lines(&[
                "docker","images","--format",
                "{{.ID}}\t{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}"
            ]).await?;
            let mut rows = vec![];
            for l in lines {
                let parts: Vec<&str> = l.split('\t').collect();
                if parts.len() >= 4 {
                    rows.push(ImageRow {
                        id: parts[0].to_string(),
                        tags: parts[1].to_string(),
                        size: parts[2].to_string(),
                        created: parts[3].to_string(),
                    });
                }
            }
            Ok(rows)
        })
    }

    fn remove_image(&self, id: String, force: bool) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            if force {
                eng.run_lines(&["docker","rmi","-f",&id]).await?;
            } else {
                eng.run_lines(&["docker","rmi",&id]).await?;
            }
            Ok(())
        })
    }

    fn list_volumes(&self) -> tokio::task::JoinHandle<Result<Vec<VolumeRow>>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            // docker volume ls --format '{{.Name}}\t{{.Driver}}'
            let lines = eng.run_lines(&["docker","volume","ls","--format","{{.Name}}\t{{.Driver}}"]).await?;
            let mut rows = vec![];
            for l in lines {
                let parts: Vec<&str> = l.split('\t').collect();
                if parts.len() >= 2 {
                    rows.push(VolumeRow {
                        name: parts[0].to_string(),
                        driver: parts[1].to_string(),
                        mountpoint: "".to_string(),
                    });
                }
            }
            Ok(rows)
        })
    }

    fn remove_volume(&self, name: String, force: bool) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            let eng = WslCliEngine::new(distro);
            if force {
                eng.run_lines(&["docker","volume","rm","-f",&name]).await?;
            } else {
                eng.run_lines(&["docker","volume","rm",&name]).await?;
            }
            Ok(())
        })
    }

    fn start_engine(&self) -> tokio::task::JoinHandle<Result<()>> {
        let distro = self.distro.clone();
        tokio::spawn(async move {
            // systemd must be enabled in that distro for this to work.
            let eng = WslCliEngine::new(distro);
            // try: sudo systemctl start docker
            eng.run_lines(&["sudo","systemctl","start","docker"]).await?;
            Ok(())
        })
    }
}
