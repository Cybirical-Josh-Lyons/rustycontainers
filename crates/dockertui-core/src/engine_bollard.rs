use std::pin::Pin;
use anyhow::{anyhow, Result};
use bollard::container::{
    ListContainersOptions, LogsOptions as BollardLogsOptions, RestartContainerOptions,
    StartContainerOptions, StatsOptions, StopContainerOptions,
};
use bollard::image::{ListImagesOptions, RemoveImageOptions};
use bollard::volume::ListVolumesOptions;
use bollard::{Docker, API_DEFAULT_VERSION};
use crate::engine::{Engine, EngineKind, LogsOptions};
use crate::models::{ContainerRow, ContainerStats, ImageRow, VolumeRow};
use futures::{Stream, StreamExt};

pub struct BollardEngine {
    docker: Docker,
    display_name: String,
}

impl BollardEngine {
    pub fn connect(host: &str) -> Result<Self> {
        let docker = if host.starts_with("unix://") {
            Docker::connect_with_unix(host, 120, API_DEFAULT_VERSION)
        } else if host.starts_with("npipe://") || host.starts_with("npipe:") {
            // This exists on Windows builds. On Linux it will not compile if referenced,
            // so we gate it.
            #[cfg(windows)]
            {
                Docker::connect_with_named_pipe(host, 120, API_DEFAULT_VERSION)
            }
            #[cfg(not(windows))]
            {
                return Err(anyhow!(
                    "npipe Docker host is only supported on Windows builds: {host}"
                ));
            }
        } else if host.starts_with("https://") {
            // TLS requires cert/key paths; for MVP we don't support implicit TLS here.
            return Err(anyhow!(
                "https:// DOCKER_HOST requires TLS config (not wired in this MVP). Use unix://, npipe://, or tcp://"
            ));
        } else if host.starts_with("tcp://") || host.starts_with("http://") {
            // bollard wants http(s) url for connect_with_http
            let http_addr = host.replace("tcp://", "http://");
            Docker::connect_with_http(&http_addr, 120, API_DEFAULT_VERSION)
        } else {
            return Err(anyhow!("Unsupported DOCKER_HOST: {host}"));
        }
        .map_err(|e| anyhow!("Failed to connect to Docker daemon at {host}: {e}"))?;

        Ok(Self {
            docker,
            display_name: format!("Docker API ({host})"),
        })
    }
}

impl Engine for BollardEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::DockerApi
    }

    fn name(&self) -> String {
        self.display_name.clone()
    }

    fn list_containers(&self) -> tokio::task::JoinHandle<Result<Vec<ContainerRow>>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            let opts = Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            });

            let list = docker.list_containers(opts).await?;
            let mut rows = Vec::with_capacity(list.len());

            for c in list {
                // In bollard 0.16 container summary fields are mostly Options.
                let id = c.id.unwrap_or_default();
                let name = c
                    .names
                    .unwrap_or_default()
                    .get(0)
                    .cloned()
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string();

                let image = c.image.unwrap_or_default();
                let state = c.state.unwrap_or_default();
                let status = c.status.unwrap_or_default();

                rows.push(ContainerRow {
                    id,
                    name,
                    image,
                    state,
                    status,
                });
            }

            Ok(rows)
        })
    }

    fn start_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            docker
                .start_container::<String>(&id, None::<StartContainerOptions<String>>)
                .await?;
            Ok(())
        })
    }

    fn stop_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            docker
                .stop_container(&id, Some(StopContainerOptions { t: 10 }))
                .await?;
            Ok(())
        })
    }

    fn restart_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            docker
                .restart_container(&id, Some(RestartContainerOptions { t: 10 }))
                .await?;
            Ok(())
        })
    }

    fn container_logs(
        &self,
        id: String,
        opts: LogsOptions,
    ) -> tokio::task::JoinHandle<Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            let bollard_opts = BollardLogsOptions::<String> {
                follow: opts.follow,
                stdout: true,
                stderr: true,
                tail: opts.tail.unwrap_or_else(|| "200".to_string()),
                timestamps: opts.timestamps,
                ..Default::default()
            };

            let stream = docker.logs(&id, Some(bollard_opts)).map(|item| {
                item.map_err(|e| anyhow!(e)).and_then(|chunk| Ok(chunk.to_string()))
            });

            Ok(Box::pin(stream) as Pin<Box<dyn Stream<Item = Result<String>> + Send>>)
        })
    }

    fn container_stats(&self, id: String) -> tokio::task::JoinHandle<Result<ContainerStats>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            let mut stream = docker.stats(
                &id,
                Some(StatsOptions {
                    stream: false,
                    one_shot: true,
                }),
            );

            let stats = match stream.next().await {
                Some(Ok(stats)) => stats,
                Some(Err(e)) => return Err(anyhow!(e)),
                None => return Err(anyhow!("No stats returned for container {id}")),
            };

            let cpu_delta = stats
                .cpu_stats
                .cpu_usage
                .total_usage
                .saturating_sub(stats.precpu_stats.cpu_usage.total_usage);
            let system_delta = stats
                .cpu_stats
                .system_cpu_usage
                .unwrap_or(0)
                .saturating_sub(stats.precpu_stats.system_cpu_usage.unwrap_or(0));
            let online_cpus = stats
                .cpu_stats
                .online_cpus
                .or_else(|| {
                    stats
                        .cpu_stats
                        .cpu_usage
                        .percpu_usage
                        .as_ref()
                        .map(|v| v.len() as u64)
                })
                .unwrap_or(1);

            let cpu_percent = if system_delta > 0 && cpu_delta > 0 {
                (cpu_delta as f64 / system_delta as f64) * online_cpus as f64 * 100.0
            } else {
                0.0
            };

            let mem_usage = stats.memory_stats.usage.unwrap_or(0);
            let mem_limit = stats.memory_stats.limit.unwrap_or(0);
            let mem_percent = if mem_limit > 0 {
                (mem_usage as f64 / mem_limit as f64) * 100.0
            } else {
                0.0
            };

            let (mut rx_bytes, mut tx_bytes) = (0u64, 0u64);
            if let Some(networks) = stats.networks {
                for net in networks.values() {
                    rx_bytes = rx_bytes.saturating_add(net.rx_bytes);
                    tx_bytes = tx_bytes.saturating_add(net.tx_bytes);
                }
            } else if let Some(network) = stats.network {
                rx_bytes = network.rx_bytes;
                tx_bytes = network.tx_bytes;
            }

            let (mut read_bytes, mut write_bytes) = (0u64, 0u64);
            if let Some(entries) = stats.blkio_stats.io_service_bytes_recursive {
                for entry in entries {
                    match entry.op.as_str() {
                        "Read" => read_bytes = read_bytes.saturating_add(entry.value),
                        "Write" => write_bytes = write_bytes.saturating_add(entry.value),
                        _ => {}
                    }
                }
            }

            let pids = stats.pids_stats.current.unwrap_or(0);

            Ok(ContainerStats {
                cpu_percent: format!("{cpu_percent:.2}%"),
                mem_usage: format!(
                    "{} / {}",
                    format_bytes(mem_usage),
                    format_bytes(mem_limit)
                ),
                mem_percent: format!("{mem_percent:.2}%"),
                net_io: format!(
                    "{} / {}",
                    format_bytes(rx_bytes),
                    format_bytes(tx_bytes)
                ),
                block_io: format!(
                    "{} / {}",
                    format_bytes(read_bytes),
                    format_bytes(write_bytes)
                ),
                pids: pids.to_string(),
                gpu: "n/a".to_string(),
            })
        })
    }

    fn list_images(&self) -> tokio::task::JoinHandle<Result<Vec<ImageRow>>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            let imgs = docker
                .list_images(Some(ListImagesOptions::<String> {
                    all: true,
                    ..Default::default()
                }))
                .await?;

            let mut rows = Vec::with_capacity(imgs.len());
            for i in imgs {
                // In bollard 0.16 these are *not* Option types.
                let id = i.id; // String
                let tags = i.repo_tags.join(", "); // Vec<String>
                let size = format!("{:.2} MB", (i.size as f64) / 1_048_576.0); // i64
                let created = i.created.to_string(); // i64 unix timestamp (seconds)

                rows.push(ImageRow {
                    id,
                    tags,
                    size,
                    created,
                });
            }
            Ok(rows)
        })
    }

    fn remove_image(&self, id: String, force: bool) -> tokio::task::JoinHandle<Result<()>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            docker
                .remove_image(
                    &id,
                    Some(RemoveImageOptions {
                        force,
                        noprune: false,
                    }),
                    None,
                )
                .await?;
            Ok(())
        })
    }

    fn list_volumes(&self) -> tokio::task::JoinHandle<Result<Vec<VolumeRow>>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            let vols = docker
                .list_volumes(Some(ListVolumesOptions::<String> {
                    ..Default::default()
                }))
                .await?;

            let mut rows = vec![];
            if let Some(v) = vols.volumes {
                rows.reserve(v.len());
                for vol in v {
                    // In bollard 0.16 these are Strings (not Options).
                    rows.push(VolumeRow {
                        name: vol.name,
                        driver: vol.driver,
                        mountpoint: vol.mountpoint,
                    });
                }
            }
            Ok(rows)
        })
    }

    fn remove_volume(&self, name: String, force: bool) -> tokio::task::JoinHandle<Result<()>> {
        let docker = self.docker.clone();
        tokio::spawn(async move {
            docker
                .remove_volume(&name, Some(bollard::volume::RemoveVolumeOptions { force }))
                .await?;
            Ok(())
        })
    }

    fn start_engine(&self) -> tokio::task::JoinHandle<Result<()>> {
        // No-op for now; service-manager start is platform-specific.
        tokio::spawn(async move { Ok(()) })
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
