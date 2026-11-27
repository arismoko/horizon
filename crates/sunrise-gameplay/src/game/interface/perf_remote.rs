//! A guest's `sunrise:perf` requests (docs/perf-platform.md §4.2): who may profile this server, the one capture at a time, and the log lines about it.
use crate::game::ServerGame;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use sunrise_gameplay_common::game::profile_capture_common::{Live, Target, reports_dir, shown};
use sunrise_perf_format::v1::{StopReason, Trigger};
use sunrise_protocol::packets::{configuration::CustomPayload, play};
use sunrise_sim::perf_net::{
    CHANNEL, Held, OPERATOR_LEVEL, Outgoing, Refusal, Reply, Request, Verdict, Wanted, judge,
};

/// The guest whose capture runs on this server.
pub(in crate::game) struct Requester {
    conn: u32,
    name: String,
    /// Their connection ended: the capture stops and its file stays on the server.
    left: bool,
}

/// A request that passed the checks and waits for the tick loop that owns the capture state.
pub(in crate::game) enum Asked {
    Start {
        conn: u32,
        name: String,
        wanted: Wanted,
    },
    Stop(StopReason),
}

/// What a guest has asked of this server, for the tick loop.
#[derive(Default)]
pub(in crate::game) struct Served {
    requester: Option<Requester>,
    asked: Vec<Asked>,
    sending: Vec<Sending>,
}

/// A finished capture's file on its way to the guest that asked for it.
struct Sending {
    conn: u32,
    file: File,
    pace: Outgoing,
}

/// How a server capture's file ended.
pub(in crate::game) enum Done {
    Saved {
        path: String,
        file: PathBuf,
        removed: Vec<String>,
    },
    Failed,
}

fn payload(reply: &Reply) -> play::Clientbound {
    play::Clientbound::CustomPayload(CustomPayload {
        channel: CHANNEL.into(),
        data: reply.encode(),
    })
}

fn window(wanted: &Wanted) -> String {
    wanted
        .seconds
        .map_or_else(|| "until stopped".to_owned(), |s| format!("{s} s"))
}

impl ServerGame<'_> {
    /// A `sunrise:perf` payload from `conn`.
    pub(in crate::game::server_side) fn guest_perf(&mut self, conn: u32, data: &[u8]) {
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let name = guest.name.clone();
        let Ok(request) = Request::decode(data) else {
            log::debug!("Sunrise: a bad server profile request from {name}");
            return;
        };
        let served = &mut self.server.profile.served;
        let held = match &served.requester {
            Some(r) if r.conn == conn => Held::Asker,
            Some(_) => Held::Someone,
            None if served
                .asked
                .iter()
                .any(|a| matches!(a, Asked::Start { .. })) =>
            {
                Held::Someone
            }
            None => Held::Nothing,
        };
        match judge(&guest.brand, guest.permission, &request, held) {
            Verdict::Ignore => {}
            Verdict::Refuse(why) => {
                if why == Refusal::Denied {
                    log::warn!(
                        "Sunrise: {name} asked for a server profile and was refused: operator level {OPERATOR_LEVEL} is required"
                    );
                }
                guest.send(&payload(&Reply::Refused(why)));
            }
            Verdict::Accept => served.asked.push(match request {
                Request::Start(wanted) => Asked::Start { conn, name, wanted },
                Request::Stop => Asked::Stop(StopReason::Normal),
            }),
        }
    }

    /// Sends `reply` to the guest on `conn`, if it is still here.
    pub(in crate::game::server_side) fn perf_reply(&self, conn: u32, reply: &Reply) {
        if let Some(guest) = self.server.lan.as_ref().and_then(|h| h.hub.guest(conn)) {
            guest.send(&payload(reply));
        }
    }

    /// What guests asked since the last tick, and a stop when the guest whose capture runs has left.
    pub(in crate::game::server_side) fn take_perf_asked(&mut self) -> Vec<Asked> {
        let served = &mut self.server.profile.served;
        let mut asked = std::mem::take(&mut served.asked);
        if let Some(r) = served.requester.as_mut()
            && !r.left
            && self
                .server
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.guest(r.conn).is_none())
        {
            r.left = true;
            log::info!("Sunrise: {} left; their server profile stops", r.name);
            asked.push(Asked::Stop(StopReason::Disconnected));
        }
        asked
    }

    /// The capture `name` asked for runs, in the file at `path`.
    pub(in crate::game::server_side) fn perf_started(
        &mut self,
        conn: u32,
        name: String,
        wanted: &Wanted,
        path: &str,
    ) {
        log::info!(
            "Sunrise: {name} started a server profile ({}, samples {})",
            window(wanted),
            if wanted.samples { "on" } else { "off" }
        );
        self.server.profile.served.requester = Some(Requester {
            conn,
            name,
            left: false,
        });
        self.perf_reply(
            conn,
            &Reply::Started {
                path: path.to_owned(),
                seconds: wanted.seconds,
                samples: wanted.samples,
            },
        );
    }

    /// The capture `conn` asked for could not begin.
    pub(in crate::game::server_side) fn perf_refused(&self, conn: u32, why: Refusal) {
        self.perf_reply(conn, &Reply::Refused(why));
    }

    /// Server captures whose files are done: the guest that asked is told, and the log names it.
    pub(in crate::game) fn perf_captures_done(&mut self, done: Vec<Done>) {
        for done in done {
            let Some(Requester { conn, name, left }) = self.server.profile.served.requester.take()
            else {
                continue;
            };
            if left {
                if let Done::Saved { path, .. } = &done {
                    log::info!("Sunrise: {name}'s server profile saved to {path}");
                }
                continue;
            }
            match done {
                Done::Saved {
                    path,
                    file,
                    removed,
                } => {
                    log::info!("Sunrise: {name}'s server profile saved to {path}");
                    let (bytes, sending) = match File::open(&file).and_then(|f| {
                        let len = f.metadata()?.len();
                        Ok((f, len))
                    }) {
                        Ok((file, len)) => (
                            len,
                            Some(Sending {
                                conn,
                                file,
                                pace: Outgoing::new(len, Instant::now()),
                            }),
                        ),
                        Err(e) => {
                            log::warn!("Sunrise: {name}'s server profile cannot be sent: {e}");
                            (0, None)
                        }
                    };
                    self.perf_reply(
                        conn,
                        &Reply::Finished {
                            path,
                            removed,
                            bytes,
                        },
                    );
                    self.server.profile.served.sending.extend(sending);
                }
                Done::Failed => {
                    log::error!("Sunrise: {name}'s server profile could not be written");
                    self.perf_refused(conn, Refusal::Failed);
                }
            }
        }
    }

    /// Sends the next chunk of each file that is due, at most [`sunrise_client::perf_net::RATE`] a second and not while the guest has a backlog.
    pub(in crate::game::server_side) fn pump_perf_files(&mut self) {
        let now = Instant::now();
        let mut sending = std::mem::take(&mut self.server.profile.served.sending);
        sending.retain_mut(|s| {
            let Some(guest) = self.server.lan.as_ref().and_then(|h| h.hub.guest(s.conn)) else {
                return false;
            };
            let queued = guest.sender.pending.load(Ordering::Relaxed);
            let Some(n) = s.pace.next(now, queued) else {
                return !s.pace.done();
            };
            let mut chunk = vec![0; n];
            match s.file.read_exact(&mut chunk) {
                Ok(()) => guest.send(&payload(&Reply::Chunk(chunk))),
                Err(e) => {
                    log::warn!("Sunrise: a server profile could not be read for sending: {e}");
                    guest.send(&payload(&Reply::Refused(Refusal::Failed)));
                    return false;
                }
            }
            !s.pace.done()
        });
        sending.append(&mut self.server.profile.served.sending);
        self.server.profile.served.sending = sending;
    }

    /// On the server thread, between two ticks: begins and ends the captures guests asked for.
    pub(in crate::game::server_side) fn serve_perf_asked(&mut self) {
        self.pump_perf_files();
        for asked in self.take_perf_asked() {
            match asked {
                Asked::Start { conn, name, wanted } => {
                    if self.server_capture_busy() {
                        self.perf_refused(conn, Refusal::AlreadyRunning);
                        continue;
                    }
                    let target = Target::Own {
                        options: wanted.options(),
                        trigger: Trigger::Remote,
                        dir: reports_dir(),
                        viewer: None,
                    };
                    match self.start_server_capture(target, Live::new()) {
                        Ok(path) => {
                            let path: std::path::PathBuf = path;
                            self.perf_started(conn, name, &wanted, &shown(&path));
                        }
                        Err(e) => {
                            log::error!("Sunrise: {name}'s server profile did not start: {e}");
                            self.perf_refused(conn, Refusal::Failed);
                        }
                    }
                }
                Asked::Stop(reason) => self.stop_server_capture(reason, None),
            }
        }
    }
}
