use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "url_type", content = "url_value")]
#[serde(rename_all = "lowercase")]
pub enum StreamUrl {
    Hostname,
    XForwardedHostName,
    Static(String),
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MountConfig {
    pub source_auth: Option<String>,
    pub sub_auth: Option<String>,
    #[serde(flatten)]
    pub stream_url: StreamUrl,
    pub permanent: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
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

        let admin_authorization = other.admin_authorization.or(self.admin_authorization);
        let allow_unauthenticated_mounts =
            other.allow_unauthenticated_mounts || self.allow_unauthenticated_mounts;
        let mut mounts = self.mounts;
        for (k, v) in other.mounts {
            mounts.insert(k, v);
        }

        Self {
            admin_authorization,
            allow_unauthenticated_mounts,
            mounts,
        }
    }
}
