use std::{net::SocketAddr, sync::Arc, time::Instant};

use httparse::{Header, Request};
use log::{debug, error, info, trace, warn};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    sync::RwLock,
};

use crate::{
    api::MountInfo,
    config::Config,
    state::{State, StreamUrl},
};

use super::{Connector, CreateConnectorError};

pub struct BasicHttpResponse<'a> {
    code: u16,
    name: &'static str,
    headers: &'a [&'a str],
}

impl<'a> BasicHttpResponse<'a> {
    pub const OK: Self = Self::no_headers(200, "OK");
    pub const UNAUTHORIZED: Self = Self::no_headers(401, "Unauthorized");
    pub const NOT_FOUND: Self = Self::no_headers(404, "Not found");
    pub const BAD_REQUEST: Self = Self::no_headers(400, "Bad Request");
    pub const CONFLICT: Self = Self::no_headers(409, "Conflict");
    pub const INTERNAL_SERVER_ERROR: Self = Self::no_headers(500, "Internal server error");

    const fn no_headers(code: u16, name: &'static str) -> Self {
        Self::new(code, name, &[])
    }

    pub const fn new(code: u16, name: &'static str, headers: &'a [&'a str]) -> Self {
        Self {
            code,
            name,
            headers,
        }
    }

    pub const fn ok(headers: &'a [&'a str]) -> Self {
        let mut me = Self::OK;
        me.headers = headers;
        me
    }

    pub async fn send<T>(&self, write: &mut T)
    where
        T: AsyncWrite + Unpin,
    {
        let mut string = format!("HTTP/1.1 {} {}\r\n", self.code, self.name);

        for header in self.headers {
            string.push_str(header);
            string.push_str("\r\n");
        }

        string.push_str("\r\n");

        write.write_all(string.as_bytes()).await.ok();
    }
}

pub struct SocketHandler {
    config: Config,
    state: Arc<RwLock<State>>,
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    socket: (BufReader<OwnedReadHalf>, OwnedWriteHalf),
}

fn find_header<'a>(
    mut headers: impl Iterator<Item = &'a Header<'a>>,
    name: &str,
) -> Option<String> {
    headers
        .find(|h| h.name == name)
        .map(|h| {
            if let Ok(value) = std::str::from_utf8(h.value) {
                Some(value.to_string())
            } else {
                None
            }
        })
        .flatten()
}

impl SocketHandler {
    pub fn new(
        config: Config,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        socket: TcpStream,
        state: Arc<RwLock<State>>,
    ) -> Self {
        let (read_half, write_half) = socket.into_split();
        let reader = BufReader::new(read_half);

        Self {
            config,
            local_addr,
            remote_addr,
            socket: (reader, write_half),
            state,
        }
    }

    async fn mount_info(&mut self, request: Request<'_, '_>, method: &str) {
        let mut write_half = &mut self.socket.1;

        if method == "GET" {
            let json_data: Vec<MountInfo> = self
                .state
                .read()
                .await
                .mounts()
                .map(|(n, m)| {
                    let stream_url = m
                        .stream_url()
                        .clone()
                        .or(self.config.default_stream_url.clone())
                        .unwrap_or(StreamUrl::default());

                    let x_forwarded_host = find_header(request.headers.iter(), "X-Forwarded-Host");

                    let host = find_header(request.headers.iter(), "Host")
                        .unwrap_or(format!("{:?}", self.local_addr));

                    let stream_url = match stream_url {
                        StreamUrl::Hostname => format!("{}{}", host, n),
                        StreamUrl::XForwardedHostName => {
                            format!("{}{}", x_forwarded_host.unwrap_or(host), n)
                        }
                        StreamUrl::Static(value) => value,
                    };

                    MountInfo::from_named_mount(&n, m, stream_url)
                })
                .collect();

            if let Ok(string) = serde_json::to_string_pretty(&json_data) {
                let content_type = "Content-Type: application/json";
                let content_length = &format!("Content-Length: {}", string.as_bytes().len());

                BasicHttpResponse::ok(&[content_type, content_length])
                    .send(write_half)
                    .await;
                write_half.write_all(string.as_bytes()).await.ok();
            } else {
                BasicHttpResponse::INTERNAL_SERVER_ERROR
                    .send(&mut write_half)
                    .await;
            }
        } else {
            BasicHttpResponse::BAD_REQUEST.send(write_half).await;
        }
        return;
    }

    async fn admin(&mut self, uri: &str, request: Request<'_, '_>) {
        let write_half = &mut self.socket.1;

        info!("Got admin request: {}", uri);

        let auth = if let Some(auth) = find_header(request.headers.iter(), "Authorization") {
            auth
        } else {
            BasicHttpResponse::UNAUTHORIZED.send(write_half).await;
            return;
        };

        let is_admin = self.config.admin_authorization.is_some()
            && Some(&auth) == self.config.admin_authorization.as_ref();

        info!("{} is using admin credentials.", self.remote_addr);

        let uri = &uri["/admin/".len()..];

        if uri.starts_with("metadata?") {
            let values = &uri["metadata?".len()..].split("&");
            trace!(
                "Admin metadata request: {}",
                values.clone().collect::<String>()
            );

            let find_key = |name: &str| {
                values.clone().find(|v| v.starts_with(name)).map(|v| {
                    let value = &v[name.len()..];
                    urlencoding::decode(value).expect("UTF-8").to_string()
                })
            };

            let (mount, mount_name) = if let Some(mount_name) = find_key("mount=") {
                let state = self.state.read().await;
                let mount = if let Some((_, mount)) = state.mounts().find(|m| m.0 == &mount_name) {
                    (mount.clone(), mount_name)
                } else {
                    BasicHttpResponse::NOT_FOUND.send(write_half).await;
                    return;
                };
                mount
            } else {
                error!("Could not find mount name for admin request.");
                BasicHttpResponse::BAD_REQUEST.send(write_half).await;
                return;
            };

            if !is_admin
                && mount.source_auth().is_some()
                && mount.source_auth() != &Some(auth.into())
            {
                BasicHttpResponse::UNAUTHORIZED.send(write_half).await;
                return;
            }

            if Some("updinfo".to_string()) != find_key("mode=") {
                BasicHttpResponse::BAD_REQUEST.send(write_half).await;
                return;
            }

            let song = if let Some(song) = find_key("song=") {
                song
            } else {
                BasicHttpResponse::BAD_REQUEST.send(write_half).await;
                return;
            };

            info!(
                "Updating mount {}. Setting song name to {}",
                mount_name, song
            );

            {
                let mut state = self.state.write().await;
                state
                    .mounts_mut()
                    .find(|m| m.0 == &mount_name)
                    .map(|m| m.1.set_song(song.to_string()));
            }
        } else {
            error!("Unknown admin request. {}", uri);
            BasicHttpResponse::BAD_REQUEST.send(write_half).await;
        }
    }

    pub async fn static_file(&mut self, uri: &str) {
        let stripped = &uri["/static/".len()..];
        let write_half = &mut self.socket.1;

        if let Some(static_sources) = &self.config.static_source_dir {
            let mut path = static_sources.clone();

            for part in stripped.split("/") {
                if part == "." || part == ".." || part == "~" {
                    BasicHttpResponse::NOT_FOUND.send(write_half).await;
                    return;
                }
            }

            path.push(stripped);
            debug!("Serving file {:?} to {:?}", path, self.remote_addr);

            if let Ok(file) = std::fs::metadata(path.clone()) {
                let file_length = file.len();
                if file.is_dir() {
                    BasicHttpResponse::NOT_FOUND.send(write_half).await;
                } else {
                    let mut file = if let Ok(file) = tokio::fs::File::open(path.clone()).await {
                        file
                    } else {
                        BasicHttpResponse::NOT_FOUND.send(write_half).await;
                        return;
                    };

                    let content_length = format!("Content-Length: {}", file_length);
                    let mime_type = format!(
                        "Content-Type: {}",
                        mime_guess::from_path(path.clone()).first_or(mime_guess::mime::TEXT_PLAIN)
                    );
                    BasicHttpResponse::ok(&[&content_length, &mime_type])
                        .send(write_half)
                        .await;

                    let mut buffer = [0u8; 32 * 1024];
                    while let Ok(data) = file.read(&mut buffer).await {
                        if data == 0 {
                            break;
                        }
                        if let Err(_) = write_half.write_all(&buffer[..data]).await {
                            break;
                        }
                    }
                }
            } else {
                BasicHttpResponse::NOT_FOUND.send(write_half).await;
            }
        } else {
            warn!("Got request for static files, but no static file path is configured!");
            BasicHttpResponse::NOT_FOUND.send(write_half).await;
        }
    }

    pub async fn run(mut self) {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut request_buffer = Vec::with_capacity(2048);

        let read_half = &mut self.socket.0;

        let bytes = read_half.read_buf(&mut request_buffer).await.unwrap();

        let mut request = httparse::Request::new(&mut headers);

        let result = request.parse(&request_buffer[..bytes]);

        if let Err(_) = result {
            // TODO handle parse error
            return;
        }

        let uri = if let Some(path) = request.path {
            path
        } else {
            // TODO handle parse error
            return;
        };

        let method = if let Some(method) = request.method {
            method
        } else {
            // TODO handle parse error
            return;
        };

        if uri == "/favicon.ico" || uri == "/" || uri.starts_with("/static/") {
            let uri = if uri == "/" {
                "/static/index.html"
            } else if uri == "/favicon.ico" {
                "/static/favicon.ico"
            } else {
                uri
            };
            self.static_file(uri).await;
        } else if uri == "/mount_info" {
            let start = Instant::now();
            self.mount_info(request, method).await;
            let duration = Instant::now().duration_since(start);
            trace!(
                "Computed and responded with mount info in {}",
                humantime::format_duration(duration)
            );
        } else if uri.starts_with("/admin/") {
            self.admin(uri, request).await;
            return;
        } else {
            let content_type = if let Some(value) = request
                .headers
                .iter()
                .find(|h| h.name == "Content-Type")
                .map(|h| h.value)
            {
                if let Ok(value) = std::str::from_utf8(value) {
                    Some(value)
                } else {
                    None
                }
            } else {
                None
            };

            let authorization = if let Some(value) = request
                .headers
                .iter()
                .find(|h| h.name == "Authorization")
                .map(|h| h.value)
            {
                if let Ok(value) = std::str::from_utf8(value) {
                    Some(value)
                } else {
                    None
                }
            } else {
                None
            };

            let (reader, write_half) = self.socket;

            let connector = Connector::parse(
                self.remote_addr,
                &self.config,
                self.state,
                method,
                uri,
                content_type,
                authorization,
                write_half,
                reader,
                request.headers,
            )
            .await;

            match connector {
                Ok(connector) => connector.run().await,
                Err((e, mut write_half, _)) => {
                    debug!(
                        "Connection to {:?} failed. Reason: {:?}",
                        self.remote_addr, e
                    );
                    let response = match e {
                        CreateConnectorError::UnknownMethod(_) => BasicHttpResponse::BAD_REQUEST,
                        CreateConnectorError::MountHasSource(_) => BasicHttpResponse::CONFLICT,
                        CreateConnectorError::MountDoesNotExist(_) => BasicHttpResponse::NOT_FOUND,
                        CreateConnectorError::SourceMissingContentType => {
                            BasicHttpResponse::BAD_REQUEST
                        }
                        CreateConnectorError::Unauthorized => BasicHttpResponse::UNAUTHORIZED,
                        CreateConnectorError::MountNotConnected(_) => BasicHttpResponse::NOT_FOUND,
                    };

                    response.send(&mut write_half).await;
                }
            }
        }
    }
}
