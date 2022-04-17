use std::collections::HashMap;

use tokio::sync::mpsc::{Sender, UnboundedSender};

#[derive(Debug)]
pub struct Mount {
    pub content_type: String,
    pub subscribe_channel: Sender<UnboundedSender<Vec<u8>>>,
}

impl Mount {
    pub fn get_sub_channel(&self) -> &Sender<UnboundedSender<Vec<u8>>> {
        &self.subscribe_channel
    }
}

#[derive(Debug)]
pub struct State {
    streams: HashMap<String, Mount>,
}

impl State {
    pub fn new() -> Self {
        Self {
            streams: HashMap::default(),
        }
    }

    pub fn add_mount(&mut self, mount_name: String, stream: Mount) -> bool {
        if !self.streams.contains_key(&mount_name) {
            self.streams.insert(mount_name, stream);
            true
        } else {
            false
        }
    }

    pub fn remove_mount(&mut self, mount_name: &String) -> Option<Mount> {
        self.streams.remove(mount_name)
    }

    pub fn find_mount(&self, mount_name: &String) -> Option<&Mount> {
        self.streams.get(mount_name)
    }
}
