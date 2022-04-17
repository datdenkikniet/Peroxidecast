use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use b64::ToBase64;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    sync::RwLock,
};

use crate::state::{Mount, State};

type BufLines = tokio::io::Lines<BufReader<OwnedReadHalf>>;

#[derive(Debug, Clone)]
pub struct Headers(Vec<Header>);

impl Headers {
    pub fn new(headers: Vec<Header>) -> Self {
        Self(headers)
    }

    pub fn value_for_key(&self, key: &str) -> Option<&str> {
        self.0.iter().find_map(|header| match header.key_value() {
            Some((k, v)) => {
                if key == k {
                    Some(v)
                } else {
                    None
                }
            }
            _ => None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    inner: String,
}

#[allow(unused)]
impl Header {
    fn new(input: String) -> Self {
        Self { inner: input }
    }

    pub fn key_value(&self) -> Option<(&str, &str)> {
        let mut parts = self.inner.splitn(2, ": ");
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            Some((key, value))
        } else {
            None
        }
    }

    pub fn key(&self) -> Option<&str> {
        self.key_value().map(|(k, _)| k)
    }

    pub fn value(&self) -> Option<&str> {
        self.key_value().map(|(_, v)| v)
    }
}

pub struct SocketHandler {
    state: Arc<RwLock<State>>,
    socket: TcpStream,
}

#[derive(Debug)]
struct Connector {
    stream_name: String,
    _http_version: String,
    headers: Headers,
    kind: ConnectorKind,
    write_half: OwnedWriteHalf,
    read_half: BufReader<OwnedReadHalf>,
}

impl Connector {
    pub async fn from_tcp_stream(stream: TcpStream) -> Option<Self> {
        let (read_half, write_half) = stream.into_split();

        let reader = BufReader::new(read_half);
        let mut lines = reader.lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let mut parts = line.split(" ");

            let name = parts.next();

            let kind = if name == Some("SOURCE") {
                ConnectorKind::Source
            } else if name == Some("GET") {
                ConnectorKind::Sink
            } else {
                return None;
            };

            let (stream_path, http_version) = (parts.next(), parts.next());

            if let (Some(stream_path), Some(http_version)) = (stream_path, http_version) {
                let headers = Self::extract_other_headers(&mut lines)
                    .await
                    .into_iter()
                    .map(|v| Header::new(v))
                    .collect();

                let buf_reader = lines.into_inner();

                let sub = Self {
                    stream_name: stream_path.to_string(),
                    _http_version: http_version.to_string(),
                    kind,
                    headers: Headers::new(headers),
                    write_half,
                    read_half: buf_reader,
                };
                Some(sub)
            } else {
                None
            }
        } else {
            None
        }
    }

    async fn extract_other_headers(lines: &mut BufLines) -> Vec<String> {
        let mut headers = Vec::new();
        loop {
            if let Ok(Some(line)) = lines.next_line().await {
                if line != "" {
                    headers.push(line);
                } else {
                    break;
                }
            } else {
                return headers;
            }
        }
        headers
    }
}

#[derive(Clone, Debug)]
enum ConnectorKind {
    Sink,
    Source,
}

impl SocketHandler {
    pub fn new(socket: TcpStream, state: Arc<RwLock<State>>) -> Self {
        Self { socket, state }
    }

    async fn run_source(state: Arc<RwLock<State>>, connector: Connector) {
        let mut write_half = connector.write_half;
        let mut buf_reader = connector.read_half;

        let (subs_tx, mut subs_rx) = tokio::sync::mpsc::channel(64);

        let content_type = connector
            .headers
            .value_for_key("Content-Type")
            .unwrap_or("audio/mpeg")
            .to_string();

        let username_password = b"username:password".to_base64(b64::STANDARD);
        let correct = format!("Basic {}", username_password);
        if Some(correct.as_str()) != connector.headers.value_for_key("Authorization") {
            write_half
                .write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n")
                .await
                .ok();
            return;
        };

        let mount = Mount {
            content_type,
            subscribe_channel: subs_tx,
        };

        {
            let mut state = state.write().await;
            state.add_mount(connector.stream_name.clone(), mount);
        }

        write_half.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.ok();

        let subs = Arc::new(RwLock::new(Vec::new()));

        let add_subs = async {
            while let Some(sub) = subs_rx.recv().await {
                let mut subs = subs.write().await;
                subs.push(sub);
            }
        };

        let send_data = async {
            let mut buffer = Vec::with_capacity(16384);

            let last_sub_clean = Instant::now();

            loop {
                buffer.clear();
                let buf = buf_reader.read_buf(&mut buffer).await;
                if let Ok(bytes) = buf {
                    if bytes == 0 {
                        break;
                    }

                    let mut subs_to_remove = false;
                    for sub in subs.read().await.iter() {
                        if let Err(_) = sub.send(buffer.clone()) {
                            subs_to_remove = true;
                        }
                    }

                    // Remove disconnected subs from the list
                    if subs_to_remove
                        && Instant::now().duration_since(last_sub_clean) > Duration::from_secs(10)
                    {
                        let mut write_lock = subs.write().await;
                        let filtered = write_lock
                            .clone()
                            .into_iter()
                            .filter(|sub| {
                                if let Err(_) = sub.send(Vec::new()) {
                                    false
                                } else {
                                    true
                                }
                            })
                            .collect();
                        *write_lock = filtered;
                    }
                } else {
                    break;
                }
            }
        };

        tokio::select! {
            _ = add_subs => {},
            _ = send_data => {},
        };

        {
            let mut state = state.write().await;
            state.remove_mount(&connector.stream_name);
        }
    }

    async fn run_sink(state: Arc<RwLock<State>>, mut connector: Connector) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let content_type = {
            let state = state.read().await;
            if let Some(stream) = state.find_mount(&connector.stream_name) {
                stream.get_sub_channel().send(tx).await.ok();
                stream.content_type.clone()
            } else {
                return;
            }
        };

        connector
            .write_half
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\n\r\n", content_type).as_bytes(),
            )
            .await
            .ok();

        while let Some(bytes) = rx.recv().await {
            connector.write_half.write_all(&bytes).await.ok();
        }
    }

    pub async fn run(self) {
        let connector = if let Some(connector) = Connector::from_tcp_stream(self.socket).await {
            connector
        } else {
            return;
        };

        match connector.kind {
            ConnectorKind::Sink => Self::run_sink(self.state, connector).await,
            ConnectorKind::Source => {
                Self::run_source(self.state, connector).await;
            }
        }
    }
}
