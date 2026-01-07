use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Optional explicit Docker host override. If None, uses DOCKER_HOST if present,
    /// otherwise uses sensible default per OS.
    pub docker_host: Option<String>,

    /// If Some, enables WSL CLI mode for docker operations using this distro (Windows only).
    /// Example: "Ubuntu-22.04"
    pub wsl_distro: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            docker_host: None,
            wsl_distro: None,
        }
    }
}
