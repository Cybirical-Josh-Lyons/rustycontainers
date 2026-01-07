#[derive(Debug, Clone)]
pub struct ContainerRow {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
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
