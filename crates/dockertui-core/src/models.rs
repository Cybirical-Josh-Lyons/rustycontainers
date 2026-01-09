#[derive(Debug, Clone)]
pub struct ContainerRow {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct ContainerStats {
    pub cpu_percent: String,
    pub mem_usage: String,
    pub mem_percent: String,
    pub net_io: String,
    pub block_io: String,
    pub pids: String,
    pub gpu: String,
    pub cpu_percent_value: f64,
    pub mem_usage_bytes: u64,
    pub mem_limit_bytes: u64,
    pub mem_percent_value: f64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
    pub pids_value: u64,
}

#[derive(Debug, Clone)]
pub struct ImageRow {
    pub id: String,
    pub tags: String,
    pub size: String,
    pub created: String,
}

#[derive(Debug, Clone)]
pub struct VolumeRow {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
}
