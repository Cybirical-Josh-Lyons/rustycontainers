use std::env;

#[derive(Debug, Clone)]
pub enum DockerHost {
    Unix(String),   // unix:///var/run/docker.sock
    Npipe(String),  // npipe:////./pipe/docker_engine
    Tcp(String),    // tcp://127.0.0.1:2375
}

impl DockerHost {
    pub fn as_str(&self) -> &str {
        match self {
            DockerHost::Unix(s) => s,
            DockerHost::Npipe(s) => s,
            DockerHost::Tcp(s) => s,
        }
    }
}

/// Resolve Docker host in priority:
/// 1) config override (if provided by caller)
/// 2) DOCKER_HOST
/// 3) OS default
pub fn resolve_docker_host(override_host: Option<&str>) -> DockerHost {
    if let Some(h) = override_host {
        return parse_host(h).unwrap_or_else(|| default_host());
    }
    if let Ok(h) = env::var("DOCKER_HOST") {
        if !h.trim().is_empty() {
            return parse_host(&h).unwrap_or_else(|| default_host());
        }
    }
    default_host()
}

fn default_host() -> DockerHost {
    if cfg!(windows) {
        DockerHost::Npipe("npipe:////./pipe/docker_engine".to_string())
    } else {
        DockerHost::Unix("unix:///var/run/docker.sock".to_string())
    }
}

fn parse_host(h: &str) -> Option<DockerHost> {
    let s = h.trim();
    if s.starts_with("unix://") {
        Some(DockerHost::Unix(s.to_string()))
    } else if s.starts_with("npipe://") || s.starts_with("npipe:") {
        Some(DockerHost::Npipe(s.to_string()))
    } else if s.starts_with("tcp://") || s.starts_with("http://") || s.starts_with("https://") {
        Some(DockerHost::Tcp(s.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_accepts_known_schemes() {
        assert!(matches!(
            parse_host("unix:///var/run/docker.sock"),
            Some(DockerHost::Unix(_))
        ));
        assert!(matches!(
            parse_host("npipe:////./pipe/docker_engine"),
            Some(DockerHost::Npipe(_))
        ));
        assert!(matches!(
            parse_host("tcp://127.0.0.1:2375"),
            Some(DockerHost::Tcp(_))
        ));
        assert!(matches!(
            parse_host("https://example.test:2376"),
            Some(DockerHost::Tcp(_))
        ));
    }

    #[test]
    fn resolve_docker_host_prefers_override_then_env() {
        let prev = std::env::var("DOCKER_HOST").ok();
        unsafe { std::env::set_var("DOCKER_HOST", "unix:///tmp/docker.sock"); }

        let host = resolve_docker_host(Some("tcp://127.0.0.1:2375"));
        assert!(matches!(host, DockerHost::Tcp(_)));

        let host = resolve_docker_host(None);
        assert!(matches!(host, DockerHost::Unix(_)));

        if let Some(v) = prev {
            unsafe { std::env::set_var("DOCKER_HOST", v); }
        } else {
            unsafe { std::env::remove_var("DOCKER_HOST"); }
        }
    }
}
