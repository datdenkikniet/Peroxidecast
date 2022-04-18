use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use httparse::Header;
use log::{debug, info, trace};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        RwLock,
    },
};

use crate::{
    config::Config,
    state::{IceMeta, Mount, StatSender, State, Stats, SubReceiver},
};

use super::BasicHttpResponse;

#[derive(Debug, Clone)]
pub enum CreateConnectorError {
    UnknownMethod(String),
    MountHasSource(String),
    MountDoesNotExist(String),
    SourceMissingContentType,
    Unauthorized,
    MountNotConnected(String),
}

impl<T> Into<Result<T, CreateConnectorError>> for CreateConnectorError {
    fn into(self) -> Result<T, CreateConnectorError> {
        Err(self)
    }
}

#[derive(Debug)]
enum ConnectorKind {
    Sink {
        mount_meta: IceMeta,
        data_rx: UnboundedReceiver<Vec<u8>>,
        content_type: String,
    },
    Source {
        subscriber_rx: SubReceiver,
        stats_sender: StatSender,
        start_stats: Stats,
    },
}

#[derive(Debug)]
pub struct Connector<T>
where
    T: std::fmt::Debug,
{
    remote: T,
    mount_path: String,
    kind: ConnectorKind,
    write_half: OwnedWriteHalf,
    read_half: BufReader<OwnedReadHalf>,
}

type Error = (
    CreateConnectorError,
    OwnedWriteHalf,
    BufReader<OwnedReadHalf>,
);

#[derive(Debug, Clone, Copy)]
enum SubDisconnectReason {
    SourceDisconnected,
    ClientDisconnected,
}

impl<T> Connector<T>
where
    T: std::fmt::Debug,
{
    pub async fn parse(
        remote: T,
        config: &Config,
        state: Arc<RwLock<State>>,
        method: &str,
        mount_path: &str,
        content_type: Option<&str>,
        authorization: Option<&str>,
        write_half: OwnedWriteHalf,
        read_half: BufReader<OwnedReadHalf>,
        headers: &[Header<'_>],
    ) -> Result<Self, Error>
    where
        T: std::fmt::Debug,
    {
        let authorization = authorization.map(|s| s.to_string());

        let is_admin = authorization
            .as_ref()
            .map(|a| {
                config.admin_authorization.is_some()
                    && Some(a) == config.admin_authorization.as_ref()
            })
            .unwrap_or(false);

        trace!("Parsing TCP request from {:?}", remote);
        if is_admin {
            info!("{:?} is connecting with admin credentials.", remote);
        }

        macro_rules! error {
            ($variant: tt) => {
                return Err((CreateConnectorError::$variant, write_half, read_half));
            };
            ($variant: tt($value: expr)) => {
                return Err((
                    CreateConnectorError::$variant($value),
                    write_half,
                    read_half,
                ));
            };
        }

        let kind = if method == "SOURCE" {
            let content_type = if let Some(content_type) = content_type {
                content_type
            } else {
                error!(SourceMissingContentType);
            };

            let (subs_tx, subs_rx) = tokio::sync::mpsc::unbounded_channel();
            let (stats_tx, stats_rx) = tokio::sync::watch::channel(Stats::new());

            let meta = IceMeta::from(headers);

            let found_mount = { state.read().await.find_mount(mount_path).cloned() };
            let start_stats = if let Some(mount) = found_mount {
                trace!(
                    "{:?} is attempting to become source for existing mount {}",
                    remote,
                    mount_path
                );

                let auth = mount.source_auth();
                if !is_admin && !auth.is_none() && auth != &authorization.map(|v| v.to_string()) {
                    error!(Unauthorized);
                }

                if mount.is_connected() {
                    error!(MountHasSource(mount_path.to_string()));
                } else {
                    trace!("SOURCE: {:?} ICE metadata: {:?}", remote, meta);
                    let mut state = state.write().await;
                    let mount = state.find_mount_mut(mount_path).unwrap();
                    mount.set_source(subs_tx, stats_rx, content_type.to_string(), meta);
                }

                debug!(
                    "{:?} is now sending to existing mount {} with content type {}. Current stats: {:?}",
                    remote,
                    mount_path,
                    content_type,
                    mount.stats()
                );

                mount.stats()
            } else {
                trace!("SOURCE: {:?} ICE metadata : {:?}", remote, meta);

                if !is_admin && !config.allow_unauthenticated_mounts {
                    error!(Unauthorized);
                }

                let mount = Mount::new(
                    content_type.to_string(),
                    subs_tx,
                    stats_rx,
                    authorization,
                    None,
                    false,
                    meta,
                    None,
                );

                {
                    let mut state = state.write().await;
                    state.add_mount(mount_path.to_string(), mount);
                }

                debug!(
                    "Created mount {} with content type {}.",
                    mount_path, content_type
                );
                Stats::new()
            };

            ConnectorKind::Source {
                subscriber_rx: subs_rx,
                stats_sender: stats_tx,
                start_stats,
            }
        } else if method == "GET" {
            if let Some(mount) = state.read().await.find_mount(mount_path) {
                let auth = mount.sub_auth().clone();
                if auth.is_some() && auth != authorization {
                    error!(Unauthorized);
                }

                if !mount.is_connected() {
                    error!(MountNotConnected(mount_path.to_string()));
                }

                let (data_tx, data_rx) = tokio::sync::mpsc::unbounded_channel();
                mount.sub_sender().send(data_tx).ok();
                let meta = mount.metadata();

                ConnectorKind::Sink {
                    mount_meta: meta,
                    data_rx,
                    content_type: mount.content_type().to_string(),
                }
            } else {
                error!(MountDoesNotExist(mount_path.to_string()));
            }
        } else {
            error!(UnknownMethod(method.to_string()));
        };

        Ok(Self {
            remote,
            mount_path: mount_path.to_string(),
            kind,
            write_half,
            read_half,
        })
    }

    pub async fn run(mut self) {
        match &mut self.kind {
            ConnectorKind::Sink {
                mount_meta,
                ref mut data_rx,
                content_type,
            } => {
                debug!(
                    "SUB: {:?} connected to mount {}",
                    self.remote, self.mount_path
                );
                let disconnect_reason =
                    Self::run_sink(mount_meta, &mut self.write_half, data_rx, content_type).await;
                debug!(
                    "SUB: {:?} disconnected from mount {}. Reason: {:?}",
                    self.remote, self.mount_path, disconnect_reason
                )
            }
            ConnectorKind::Source {
                ref mut subscriber_rx,
                stats_sender,
                start_stats,
            } => {
                info!(
                    "SOURCE: {:?} connected to mount {}",
                    self.remote, self.mount_path
                );
                Self::run_source(
                    start_stats.clone(),
                    subscriber_rx,
                    &stats_sender,
                    &mut self.write_half,
                    &mut self.read_half,
                )
                .await;
                info!(
                    "SOURCE: {:?} disconnected from mount {}.",
                    self.remote, self.mount_path
                );
            }
        }
    }

    async fn run_sink(
        mount_meta: &mut IceMeta,
        write_half: &mut OwnedWriteHalf,
        data_rx: &mut UnboundedReceiver<Vec<u8>>,
        content_type: &String,
    ) -> SubDisconnectReason {
        let headers = mount_meta.as_headers();
        let mut transformed: Vec<&str> = headers.iter().map(|h| h.as_str()).collect();
        let content_type = format!("Content-Type: {}", content_type);
        transformed.push(&content_type);

        BasicHttpResponse::ok(&transformed).send(write_half).await;

        while let Some(bytes) = data_rx.recv().await {
            if write_half.write_all(&bytes).await.is_err() {
                return SubDisconnectReason::ClientDisconnected;
            }
        }

        SubDisconnectReason::SourceDisconnected
    }

    async fn do_data_mirroring(
        read_half: &mut BufReader<OwnedReadHalf>,
        mut stats: Stats,
        subs: Arc<RwLock<Vec<UnboundedSender<Vec<u8>>>>>,
        stats_tx: &StatSender,
    ) {
        let mut buffer = Vec::with_capacity(16384);

        let last_sub_clean = Instant::now();

        loop {
            buffer.clear();
            let buf = read_half.read_buf(&mut buffer).await;

            if let Ok(bytes) = buf {
                stats.bytes_in += bytes;
                if bytes == 0 {
                    break;
                }

                let mut subs_to_remove = false;
                {
                    let subs = subs.read().await;
                    let sub_count = subs.len();
                    stats.sub_count = sub_count;

                    for sub in subs.iter() {
                        if let Err(_) = sub.send(buffer.clone()) {
                            stats.sub_count -= 1;
                            subs_to_remove = true;
                        } else {
                            stats.bytes_out += bytes;
                        }
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
                        .filter(|sub| !sub.is_closed())
                        .collect();
                    *write_lock = filtered;
                }
                if stats_tx.send(stats).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    async fn run_source(
        stats: Stats,
        subs_rx: &mut SubReceiver,
        stats_tx: &StatSender,
        write_half: &mut OwnedWriteHalf,
        read_half: &mut BufReader<OwnedReadHalf>,
    ) {
        BasicHttpResponse::OK.send(write_half).await;

        let subs = Arc::new(RwLock::new(Vec::new()));

        let add_subs = async {
            while let Some(sub) = subs_rx.recv().await {
                let mut subs = subs.write().await;
                subs.push(sub);
            }
        };

        let send_data = Self::do_data_mirroring(read_half, stats, subs.clone(), stats_tx);

        tokio::select! {
            _ = add_subs => {},
            _ = send_data => {},
        };
    }
}
