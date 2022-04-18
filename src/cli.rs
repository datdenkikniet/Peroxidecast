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

    /// The username for the administrator user
    #[clap(short = 'u', long)]
    admin_username: Option<String>,

    /// The password for the administrator user
    #[clap(short = 'p', long)]
    admin_password: Option<String>,

    /// Allow clients that connect with a SOURCE request to create
    /// new mountpoints without authentication
    #[clap(short = 'A', long)]
    allow_unauthenticated_mounts: bool,
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
            admin_username: self.admin_username,
            admin_password: self.admin_password,
            allow_unauthenticated_mounts: self.allow_unauthenticated_mounts,
            mounts: BTreeMap::new(),
        };

        if let Some(fcfg) = file_config {
            fcfg.merge(my_config)
        } else {
            my_config
        }
    }
}
