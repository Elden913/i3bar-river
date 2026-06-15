#![allow(clippy::collapsible_else_if)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use super::*;
use crate::event_loop;
use crate::utils::read_to_vec;

pub struct MangoInfoProvider {
    mmsg_sock: UnixStream,
    mmsg_buf: Vec<u8>,
    all_tags: Vec<MangoTagsContainer>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct MangoTag {
    index: u32,
    is_active: bool,
    is_urgent: bool,
    client_count: u32,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct MangoTagsContainer {
    monitor: String,
    tags: Vec<MangoTag>,
}

#[derive(serde::Deserialize, Debug)]
struct MangoIpc {
    all_tags: Vec<MangoTagsContainer>,
}

impl MangoInfoProvider {
    pub fn new() -> Option<Self> {
        let child = Command::new("/usr/bin/mmsg")
            .arg("watch")
            .arg("all-tags")
            .stdout(Stdio::piped())
            .spawn().ok()?;
        let stdout = child.stdout.expect("couldn't get stdout");
        let sock = unsafe { UnixStream::from_raw_fd(stdout.into_raw_fd()) };
        sock.set_nonblocking(true).ok()?;
        Some(Self {
            mmsg_sock: sock,
            mmsg_buf: Vec::new(),
            all_tags: Vec::new(),
        })
    }
    fn next_event(&mut self) -> io::Result<MangoIpc> {
        loop {
            if let Some(i) = memchr::memchr(b'\n', &self.mmsg_buf) {
                let event = String::from_utf8_lossy(&self.mmsg_buf[..i]).into_owned();
                self.mmsg_buf.drain(..=i);
                return Ok(serde_json::from_str(&event)?);
            }
            if read_to_vec(&self.mmsg_sock, &mut self.mmsg_buf)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "mmsg child process disconnected",
                ));
            }
        }
    }
}

impl WmInfoProvider for MangoInfoProvider {
    fn register(&self, ev: &mut EventLoop) {
        ev.register_with_fd(self.mmsg_sock.as_raw_fd(), |ctx| {
            loop {
                match ctx.state.shared_state.get_mango().unwrap().next_event() {
                    Ok(event) => {
                        ctx.state.shared_state.get_mango().unwrap().all_tags = event.all_tags;
                        ctx.state.tags_updated(ctx.conn, None);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        ctx.state.set_error(ctx.conn, "mango", e);
                        return Ok(event_loop::Action::Unregister);
                    }
                }
            }
            Ok(event_loop::Action::Keep)
        });
    }
    fn get_tags(&self, output: &Output) -> Vec<Tag> {
        self.all_tags
            .iter()
            .find(|c| c.monitor == output.name)
            .map_or_else(
                || {
                    eprintln!("Couldn't find tags for monitor {}", output.name);
                    Vec::new()
                },
                |container| {
                    container
                        .tags
                        .iter()
                        .enumerate()
                        .map(|(i, mango_tag)| Tag {
                            id: mango_tag.index,
                            name: (i + 1).to_string(),
                            is_focused: mango_tag.is_active,
                            is_active: mango_tag.client_count > 0,
                            is_urgent: mango_tag.is_urgent,
                        })
                        .collect()
                },
            )
    }
    fn get_mode_name(&self, _: &Output) -> Option<String> {
        None
    }

    fn click_on_tag(
        &mut self,
        _conn: &mut Connection<State>,
        _output: &Output,
        _seat: WlSeat,
        _tag_id: Option<u32>,
        _btn: PointerBtn,
    ) {
    }
}
