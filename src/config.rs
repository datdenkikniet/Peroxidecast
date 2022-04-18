use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::StreamUrl;

#[derive(Serialize, Deserialize, Clone)]
pub struct MountConfig {
    pub source_auth: Option<String>,
    pub sub_auth: Option<String>,
    #[serde(flatten)]
    pub stream_url: Option<StreamUrl>,
    pub permanent: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub static_source_dir: Option<PathBuf>,
    pub default_stream_url: Option<StreamUrl>,
    pub admin_authorization: Option<String>,
    pub allow_unauthenticated_mounts: bool,
    pub mounts: BTreeMap<String, MountConfig>,
}

impl Config {
    /// Merge this and another config
    ///
    /// The values provided by `other` will override the values
    /// provided by `self` if they are present.
    pub fn merge(self, other: Config) -> Self {
        // TODO log when settings are overwritten/ignored

        let static_source_dir = other.static_source_dir.or(self.static_source_dir);
        let default_stream_url = other.default_stream_url.or(self.default_stream_url);
        let admin_authorization = other.admin_authorization.or(self.admin_authorization);
        let allow_unauthenticated_mounts =
            other.allow_unauthenticated_mounts || self.allow_unauthenticated_mounts;
        let mut mounts = self.mounts;
        for (k, v) in other.mounts {
            mounts.insert(k, v);
        }

        Self {
            static_source_dir,
            default_stream_url,
            admin_authorization,
            allow_unauthenticated_mounts,
            mounts,
        }
    }
}
