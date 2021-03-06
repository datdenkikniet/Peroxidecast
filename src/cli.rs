use std::{collections::BTreeMap, path::PathBuf};

use clap::Parser;

use crate::config::Config;

#[derive(Parser)]
/// An IceShout2-compatible audio streaming server.
///
/// CLI options take precedence over configuration file options.
pub struct CliArgs {
    /// Path to the configuration file to load
    #[clap(short = 'f', long)]
    config_file: Option<PathBuf>,

    /// The authorization header to treat as admin
    /// authorization
    #[clap(short = 'a', long)]
    admin_authorization: Option<String>,

    /// Allow clients that connect with a SOURCE request to create
    /// new mountpoints without authentication
    #[clap(short = 'A', long)]
    allow_unauthenticated_mounts: bool,

    /// The directory from which to serve static files from.
    #[clap(short, long)]
    static_files_dir: Option<PathBuf>,
}

impl Into<Config> for CliArgs {
    fn into(self) -> Config {
        let file_config: Option<Config> = self.config_file.map(|config| {
            let res = match std::fs::read(&config) {
                Ok(contents) => contents,
                Err(e) => {
                    panic!(
                        "Failed to open/read config file {:?}. Error: {:?}",
                        config, e
                    )
                }
            };

            match std::str::from_utf8(&res) {
                Ok(file_contents) => match toml::from_str(file_contents) {
                    Ok(value) => value,
                    Err(_) => panic!("Failed to parse config file."),
                },
                Err(e) => panic!("Failed to read config file. Error: {:?}", e),
            }
        });

        let my_config = Config {
            static_source_dir: self.static_files_dir,
            admin_authorization: self.admin_authorization,
            allow_unauthenticated_mounts: self.allow_unauthenticated_mounts,
            default_stream_url: None,
            mounts: BTreeMap::new(),
        };

        if let Some(fcfg) = file_config {
            fcfg.merge(my_config)
        } else {
            my_config
        }
    }
}
