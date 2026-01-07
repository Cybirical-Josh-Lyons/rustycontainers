use crate::models::{ContainerRow, ImageRow, VolumeRow};
use anyhow::Result;
use futures::Stream;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    DockerApi,
    WslCli,
}

#[derive(Debug, Clone)]
pub struct LogsOptions {
    pub follow: bool,
    pub tail: Option<String>, // e.g. "100"
    pub timestamps: bool,
}

pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;

    fn name(&self) -> String;

    fn list_containers(&self) -> tokio::task::JoinHandle<Result<Vec<ContainerRow>>>;
    fn start_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>>;
    fn stop_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>>;
    fn restart_container(&self, id: String) -> tokio::task::JoinHandle<Result<()>>;

    fn container_logs(
        &self,
        id: String,
        opts: LogsOptions,
    ) -> tokio::task::JoinHandle<Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>>>;


    fn list_images(&self) -> tokio::task::JoinHandle<Result<Vec<ImageRow>>>;
    fn remove_image(&self, id: String, force: bool) -> tokio::task::JoinHandle<Result<()>>;

    fn list_volumes(&self) -> tokio::task::JoinHandle<Result<Vec<VolumeRow>>>;
    fn remove_volume(&self, name: String, force: bool) -> tokio::task::JoinHandle<Result<()>>;

    /// Optional: attempt to start engine (best-effort).
    fn start_engine(&self) -> tokio::task::JoinHandle<Result<()>>;
}
