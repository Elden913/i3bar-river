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
    all_monitors: Vec<MangoMonitor>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct MangoTag {
    index: u32,
    is_active: bool,
    is_urgent: bool,
    client_count: u32,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct MangoMonitor {
    name: String,
    active: bool,
    tags: Vec<MangoTag>,
    layout_symbol: String,
}



#[derive(serde::Deserialize, Debug)]
struct MangoIpc {
    monitors: Vec<MangoMonitor>,
}

impl MangoInfoProvider {
    pub fn new() -> Option<Self> {
        let child = Command::new("/usr/bin/mmsg")
            .arg("watch")
            .arg("all-monitors")
            .stdout(Stdio::piped())
            .spawn().ok()?;
        let stdout = child.stdout.expect("couldn't get stdout");
        let sock = unsafe { UnixStream::from_raw_fd(stdout.into_raw_fd()) };
        sock.set_nonblocking(true).ok()?;
        Some(Self {
            mmsg_sock: sock,
            mmsg_buf: Vec::new(),
            all_monitors: Vec::new(),
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
                let mango = ctx.state.shared_state.get_mango().unwrap();
                let mut layout_updated = false;
                match mango.next_event() {
                    Ok(event) => {
                        if let Some(i) = event.monitors.iter().position(|m| m.active) {
                            if mango.all_monitors.get(i).map(|m| &m.layout_symbol) != event.monitors.get(i).map(|m| &m.layout_symbol) {
                                layout_updated = true;
                            }
                        }
                        mango.all_monitors = event.monitors;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        ctx.state.set_error(ctx.conn, "mango", e);
                        return Ok(event_loop::Action::Unregister);
                    }
                }
                ctx.state.tags_updated(ctx.conn, None);
                if layout_updated {
                    ctx.state.layout_name_updated(ctx.conn, None);
                }
            }
            Ok(event_loop::Action::Keep)
        });
    }
    fn get_tags(&self, output: &Output) -> Vec<Tag> {
        self.all_monitors
            .iter()
            .find(|c| c.name == output.name)
            .map_or_default(
                |monitor| {
                    monitor
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
    // TODO
    fn get_layout_name(&self, output: &Output) -> Option<String> {
        // self.layout_symbol.clone()
        self.all_monitors
            .iter()
            .find(|c| c.name == output.name)
            .map_or_else(
                || {
                    eprintln!("Couldn't find layout_name for monitor {}", output.name);
                    None
                },
                |monitor| {
                    Some(monitor.layout_symbol.clone())
                })
    }
    fn get_mode_name(&self, _: &Output) -> Option<String> {
        None
    }

    fn click_on_tag(
        &mut self,
        conn: &mut Connection<State>,
        _output: &Output,
        _seat: WlSeat,
        tag_id: Option<u32>,
        _btn: PointerBtn,
    ) {
        if let Some(id) = tag_id {
            let _ = Command::new("/usr/bin/mmsg")
                .arg("dispatch")
                .arg(format!("view,{}", id))
                .stdout(Stdio::piped())
                .spawn();
        }
    }
}
