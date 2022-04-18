use std::{sync::Arc, time::Duration};

use clap::StructOpt;
use cli::CliArgs;
use config::Config;
use log::{error, info};
use net::SocketHandler;
use state::{IceMeta, Mount, State, Stats};
use tokio::{net::TcpListener, sync::RwLock};

mod api;
mod cli;
mod config;
mod net;
mod state;

#[tokio::main]
async fn main() {
    // Leak the config so we can access it globally
    let cfg: &mut Config = Box::leak(Box::new(CliArgs::parse().into()));

    pretty_env_logger::init();

    let tcp_listener = TcpListener::bind(("127.0.0.1", 8080));

    let mut state = State::new();

    for (mount_name, config) in &cfg.mounts {
        let mount = Mount::new(
            "".to_string(),
            tokio::sync::mpsc::unbounded_channel().0,
            tokio::sync::watch::channel(Stats::new()).1,
            config.source_auth.clone(),
            config.sub_auth.clone(),
            config.permanent,
            IceMeta::default(),
        );

        state.add_mount(mount_name.to_string(), mount);
    }

    let state = Arc::new(RwLock::new(state));

    let tcp_listener = match tcp_listener.await {
        Ok(value) => value,
        Err(e) => {
            error!("Socket error: {:?}", e);
            panic!()
        }
    };

    let state_clone = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            info!(
                "{}",
                serde_json::to_string(&state_clone.read().await.get_mount_stats()).unwrap()
            );
            state_clone.write().await.clean_disconnected_mounts();
        }
    });

    loop {
        match tcp_listener.accept().await {
            Ok((socket, addr)) => {
                let state = state.clone();
                let handler =
                    SocketHandler::new(cfg, socket.local_addr().unwrap(), addr, socket, state);
                tokio::spawn(handler.run());
            }
            Err(e) => error!("Socket error: {:?}", e),
        }
    }
}
