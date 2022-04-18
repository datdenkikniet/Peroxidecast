use std::collections::HashMap;

use httparse::Header;
use serde::{Deserialize, Serialize};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    watch::{Receiver as WatchReceiver, Sender as WatchSender},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Stats {
    pub sub_count: usize,
    pub bytes_in: usize,
    pub bytes_out: usize,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    pub fn new() -> Self {
        Self {
            bytes_in: 0,
            bytes_out: 0,
            sub_count: 0,
        }
    }
}

pub type StatReceiver = WatchReceiver<Stats>;
pub type StatSender = WatchSender<Stats>;

pub type SubSender = UnboundedSender<UnboundedSender<Vec<u8>>>;
pub type SubReceiver = UnboundedReceiver<UnboundedSender<Vec<u8>>>;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IceMeta {
    public: Option<i32>,
    name: Option<String>,
    description: Option<String>,
    genre: Option<String>,
    url: Option<String>,
    irc: Option<String>,
    aim: Option<String>,
    icq: Option<String>,
    audio_info: Option<String>,
}

impl<'a> From<&'a [Header<'a>]> for IceMeta {
    fn from(v: &'a [Header]) -> Self {
        use std::str::FromStr;

        let mut me = IceMeta::default();
        macro_rules! extract_val {
            ($field: ident, $name: literal) => {
                extract_val!($field, String, $name);
            };
            ($field: ident, $ty: ty, $name: literal) => {
                if let Some(value) = v.iter().find(|h| h.name == $name).map(|v| v.value) {
                    if let Ok(string) = std::str::from_utf8(value) {
                        if string != "" {
                            if let Ok(value) = <$ty>::from_str(string) {
                                me.$field = Some(value.clone())
                            }
                        }
                    }
                }
            };
        }

        extract_val!(public, i32, "ice-public");
        extract_val!(name, "ice-name");
        extract_val!(description, "ice-description");
        extract_val!(genre, "ice-genre");
        extract_val!(url, "ice-url");
        extract_val!(irc, "ice-irc");
        extract_val!(aim, "ice-aim");
        extract_val!(icq, "ice-icq");
        extract_val!(audio_info, "ice-audio-info");

        me
    }
}

impl IceMeta {
    pub fn as_headers(&self) -> Vec<String> {
        let mut vec = Vec::new();

        macro_rules! append {
            ($field: ident, $name: literal) => {
                if let Some(value) = &self.$field {
                    vec.push(format!("{}:{}", $name, value));
                }
            };
        }

        append!(public, "icy-pub");
        append!(name, "icy-name");
        append!(description, "icy-description");
        append!(genre, "icy-genre");
        append!(url, "icy-url");
        append!(irc, "icy-irc");
        append!(aim, "icy-aim");
        append!(icq, "icy-icq");
        append!(audio_info, "ice-audio-info");

        vec
    }
}

#[derive(Debug, Clone)]
pub struct Mount {
    content_type: String,
    sub_sender: SubSender,
    stat_receiver: StatReceiver,
    permanent: bool,
    source_auth: Option<String>,
    sub_auth: Option<String>,
    song: Option<String>,
    meta: IceMeta,
}

impl Mount {
    pub fn new(
        content_type: String,
        sub_sender: SubSender,
        stat_receiver: StatReceiver,
        source_auth: Option<String>,
        sub_auth: Option<String>,
        permanent: bool,
        meta: IceMeta,
    ) -> Self {
        Self {
            content_type,
            sub_sender,
            stat_receiver,
            source_auth,
            sub_auth,
            permanent,
            meta,
            song: None,
        }
    }

    pub fn source_auth(&self) -> &Option<String> {
        &self.source_auth
    }

    pub fn sub_auth(&self) -> &Option<String> {
        &self.sub_auth
    }

    pub fn sub_sender(&self) -> &SubSender {
        &self.sub_sender
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn stats(&self) -> Stats {
        self.stat_receiver.borrow().clone()
    }

    pub fn set_source(
        &mut self,
        sub_sender: SubSender,
        stat_receiver: StatReceiver,
        content_type: String,
        meta: IceMeta,
    ) {
        self.sub_sender = sub_sender;
        self.stat_receiver = stat_receiver;
        self.content_type = content_type;
        self.meta = meta;
    }

    pub fn is_connected(&self) -> bool {
        !self.sub_sender.is_closed()
    }

    pub fn metadata(&self) -> IceMeta {
        self.meta.clone()
    }

    pub fn set_song(&mut self, song: String) {
        self.song = Some(song);
    }

    pub fn song(&self) -> &Option<String> {
        &self.song
    }
}

pub struct State {
    mounts: HashMap<String, Mount>,
}

impl State {
    pub fn new() -> Self {
        Self {
            mounts: HashMap::default(),
        }
    }

    pub fn add_mount(&mut self, mount_name: String, stream: Mount) -> bool {
        if !self.mounts.contains_key(&mount_name) {
            self.mounts.insert(mount_name, stream);
            true
        } else {
            false
        }
    }

    pub fn find_mount(&self, mount_name: &str) -> Option<&Mount> {
        self.mounts.get(mount_name)
    }

    pub fn find_mount_mut(&mut self, mount_name: &str) -> Option<&mut Mount> {
        self.mounts.get_mut(mount_name)
    }

    pub fn clean_disconnected_mounts(&mut self) -> usize {
        let mut to_remove = Vec::new();
        for (mount_name, mount) in self.mounts.iter() {
            if !(mount.is_connected() || mount.permanent) {
                to_remove.push(mount_name.clone());
            }
        }
        for to_remove in &to_remove {
            self.mounts.remove(to_remove);
        }
        to_remove.len()
    }

    pub fn get_mount_stats(&self) -> HashMap<String, Stats> {
        let mut map = HashMap::new();

        for (mount_name, mount) in self.mounts.iter() {
            let stats = mount.stats();
            map.insert(mount_name.to_string(), stats.to_owned());
        }

        map
    }

    pub fn mounts(&self) -> impl Iterator<Item = (&String, &Mount)> {
        self.mounts.iter()
    }

    pub fn mounts_mut(&mut self) -> impl Iterator<Item = (&String, &mut Mount)> {
        self.mounts.iter_mut()
    }
}
