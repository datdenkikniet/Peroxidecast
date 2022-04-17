use std::sync::Arc;

use socket::SocketHandler;
use state::State;
use tokio::{net::TcpListener, sync::RwLock};

mod socket;
mod state;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let tcp_listener = TcpListener::bind(("127.0.0.1", 8080));

    let state = Arc::new(RwLock::new(State::new()));

    let mut workers = Vec::new();

    for socket in tcp_listener.await {
        let state = state.clone();
        let worker = tokio::spawn(async move {
            while let Ok((socket, _addr)) = socket.accept().await {
                let state = state.clone();
                let handler = SocketHandler::new(socket, state);
                tokio::spawn(handler.run());
            }
        });
        workers.push(worker);
    }

    for worker in workers {
        tokio::join!(worker).0.ok();
    }
}
