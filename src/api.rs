use serde::{Deserialize, Serialize};

use crate::state::{IceMeta, Mount};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    name: String,
    subscribers: usize,
    stream_url: String,
    bytes_out: usize,
    bytes_in: usize,
    on_air: bool,
    song: Option<String>,
    #[serde(flatten)]
    metadata: IceMeta,
}

impl MountInfo {
    pub fn from_named_mount(name: &str, mount: &Mount, stream_url: String) -> Self {
        let stats = mount.stats();
        MountInfo {
            name: name.to_string(),
            subscribers: stats.sub_count,
            stream_url,
            bytes_in: stats.bytes_in,
            bytes_out: stats.bytes_out,
            metadata: mount.metadata(),
            on_air: mount.is_connected(),
            song: mount.song().clone(),
        }
    }
}
