//! `profile` against the server this window is connected to (docs/perf-platform.md §4.2): the request over `sunrise:perf`, its answer, and the warnings a refusal leaves in the client's capture.
use crate::game::ClientGame;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use sunrise_client::client_perf::request::{
    Failed, GpuSwitch, ProfileSide, StartOptions, Started, Stopped,
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::profile_capture_common::{reply_text, reports_dir, shown};
use sunrise_perf_format::{FieldSender, PartMerger, RawSend, files, v1::WarningCode};
use sunrise_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};
use sunrise_sim::perf_net::{CHANNEL, Outcome, Refusal, Reply, Request, Unsent, Wanted};

const MARK_SERVER_STARTED: &str = "server_started";

fn refusal_text(why: Refusal) -> String {
    let (key, default) = match why {
        Refusal::Denied => (
            "sunrise.perf.denied",
            "a server profile needs operator level 4",
        ),
        Refusal::AlreadyRunning => (
            "sunrise.perf.already_running",
            "the server is already running a profile",
        ),
        Refusal::NotRunning => (
            "sunrise.perf.not_running",
            "no server profile of yours is running",
        ),
        Refusal::Failed => (
            "sunrise.perf.failed",
            "the server could not write the profile; see its log",
        ),
    };
    sunrise_game_data::lang::get_or(key, default)
}

fn not_sunrise_text() -> String {
    sunrise_game_data::lang::get_or(
        "sunrise.perf.not_sunrise",
        "this server isn't running Sunrise",
    )
}

fn unsent_text(why: Unsent) -> String {
    match why {
        Unsent::NotSunrise => not_sunrise_text(),
        Unsent::Busy => "a server profile is already running or still being saved".to_owned(),
        Unsent::NothingToStop => "no server profile is running".to_owned(),
    }
}

fn warning_of(why: Refusal) -> Option<WarningCode> {
    match why {
        Refusal::Denied => Some(WarningCode::ServerDenied),
        Refusal::AlreadyRunning => Some(WarningCode::ServerAlreadyRunning),
        Refusal::NotRunning | Refusal::Failed => None,
    }
}

/// The window capture's file, which the server's part is written into.
pub(in crate::game) struct Sink {
    pub(in crate::game) sender: FieldSender,
    pub(in crate::game) origin: Instant,
}

impl Sink {
    /// Closes the file's server side as far as it got, and records why it is short.
    fn incomplete(&self, merger: &PartMerger) {
        for field in merger.cut_short(self.origin.elapsed().as_secs_f64()) {
            self.sender.send(field);
        }
    }
}

enum Dest {
    /// Appended to the window's file, whole fields at a time; `held` waits for room in the writer's channel.
    Window { sink: Sink, held: Vec<u8> },
    /// Saved as it came, as a file of its own.
    Own { file: File, path: PathBuf },
}

/// The server's file as it arrives.
pub(in crate::game) struct Incoming {
    merger: PartMerger,
    dest: Dest,
    reply: Option<Sender<String>>,
    /// The last chunk has arrived and was checked whole.
    whole: bool,
}

fn part_text() -> String {
    sunrise_game_data::lang::get_or(
        "sunrise.perf.part_incomplete",
        "the server's part of the profile did not arrive whole",
    )
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Whether this window is a client of a server it does not run itself.
    pub(in crate::game::client_side) fn profiles_remote_server(&self) -> bool {
        self.client.integrated.is_none() && self.client.remote.is_some()
    }

    fn server_brand(&self) -> &str {
        self.client.remote.as_ref().map_or("", |r| r.brand.as_str())
    }

    fn send_perf(&self, request: &Request) {
        self.send(S::CustomPayload(CustomPayload {
            channel: CHANNEL.into(),
            data: request.encode(),
        }));
    }

    /// `profile server start` on a server: the answer comes back through `reply`.
    pub(in crate::game::client_side) fn start_remote_only(
        &mut self,
        options: &StartOptions,
        reply: &Sender<String>,
    ) -> Result<Option<serde_json::Value>> {
        if !self.profiles_remote_server() {
            return Err("profile server: no server to reach; open a world or join a server".into());
        }
        if self.client.profile.perf.capturing() || self.client.profile.perf.saving() {
            return Err("a capture is already running or still being saved".into());
        }
        let brand = self.server_brand().to_owned();
        self.client
            .profile
            .server_link
            .ask(&brand, Instant::now())
            .map_err(unsent_text)?;
        self.send_perf(&Request::Start(Wanted::from(options)));
        self.client.profile.server_reply = Some(reply.clone());
        self.client.profile.server_joined = false;
        Ok(None)
    }

    /// A window capture on a server also asks the server for its side; a refusal is a warning in the file.
    pub(in crate::game::client_side) fn join_remote_server(&mut self, options: &StartOptions) {
        if !self.profiles_remote_server() {
            return;
        }
        let brand = self.server_brand().to_owned();
        match self.client.profile.server_link.ask(&brand, Instant::now()) {
            Ok(()) => {
                self.send_perf(&Request::Start(Wanted::from(options)));
                self.client.profile.server_joined = true;
            }
            Err(Unsent::NotSunrise) => self.warn_server(WarningCode::ServerNotSunrise),
            Err(_) => {}
        }
    }

    fn warn_server(&mut self, code: WarningCode) {
        let t = self.client.profile.perf.now();
        self.client.profile.perf.warn(t, code);
    }

    /// `profile server stop` on a server: the file's name comes back through `reply`.
    pub(in crate::game::client_side) fn stop_remote_only(
        &mut self,
        reply: &Sender<String>,
    ) -> Result<()> {
        self.client
            .profile
            .server_link
            .stop(Instant::now())
            .map_err(unsent_text)?;
        self.send_perf(&Request::Stop);
        self.client.profile.server_reply = Some(reply.clone());
        Ok(())
    }

    /// The window capture ends: so does the server's side that asked with it.
    pub(in crate::game::client_side) fn stop_joined_server(&mut self) {
        if self.client.profile.server_joined
            && self.client.profile.server_link.stop(Instant::now()).is_ok()
        {
            self.send_perf(&Request::Stop);
        }
    }

    /// A `sunrise:perf` payload from the server.
    pub(in crate::game::client_side) fn remote_perf_reply(&mut self, data: &[u8]) {
        let Ok(reply) = Reply::decode(data) else {
            log::debug!("Sunrise: a bad server profile reply");
            return;
        };
        if let Reply::Chunk(chunk) = reply {
            self.part_chunk(&chunk);
            return;
        }
        let waiting = self.client.profile.server_reply.take();
        let joined = self.client.profile.server_joined;
        match self
            .client
            .profile
            .server_link
            .answer(reply, Instant::now())
        {
            Outcome::Started {
                path,
                seconds,
                samples,
            } => {
                if joined {
                    let t = self.client.profile.perf.now();
                    self.client.profile.perf.mark(t, MARK_SERVER_STARTED);
                    self.client.profile.server_sink =
                        self.client.profile.perf.join().map(|j| Sink {
                            sender: j.sender,
                            origin: j.origin,
                        });
                }
                if let Some(waiting) = waiting {
                    let _ = waiting.send(reply_text(&Started {
                        ok: true,
                        sides: vec![ProfileSide::Server],
                        path,
                        seconds,
                        samples,
                        gpu: GpuSwitch::Off,
                    }));
                }
            }
            Outcome::Finished { path, bytes, .. } => {
                log::info!("Sunrise: the server saved its profile to {path}");
                self.client.profile.server_joined = false;
                if bytes == 0 {
                    self.client.profile.server_reply = waiting;
                    self.abandon_part();
                } else {
                    self.begin_part(waiting);
                }
            }
            Outcome::TooLarge { bytes } => {
                log::warn!("Sunrise: the server announced a {bytes}-byte profile; it is not taken");
                self.client.profile.server_joined = false;
                self.client.profile.server_reply = waiting;
                self.abandon_part();
            }
            Outcome::Refused(why) => {
                if joined {
                    if let Some(code) = warning_of(why) {
                        self.warn_server(code);
                    }
                    self.client.profile.server_joined = false;
                    self.profiler_client_only(&refusal_text(why));
                }
                if self.client.profile.incoming.is_some()
                    || self.client.profile.server_sink.is_some()
                {
                    self.client.profile.server_reply = waiting;
                    self.abandon_part();
                    return;
                }
                log::warn!(
                    "Sunrise: the server refused the profile: {}",
                    refusal_text(why)
                );
                if let Some(waiting) = waiting {
                    let _ = waiting.send(reply_text(&Failed {
                        error: refusal_text(why),
                    }));
                }
            }
            Outcome::Stray => self.client.profile.server_reply = waiting,
        }
    }

    /// The server's file is about to arrive: into the window's capture if it still holds the file open, else into one of its own.
    fn begin_part(&mut self, waiting: Option<Sender<String>>) {
        let dest = match self.client.profile.server_sink.take() {
            Some(sink) => Ok(Dest::Window {
                sink,
                held: Vec::new(),
            }),
            None => files::create(&reports_dir(), unix_secs())
                .map(|(file, path)| Dest::Own { file, path }),
        };
        match dest {
            Ok(dest) => {
                self.client.profile.incoming = Some(Incoming {
                    merger: PartMerger::new(),
                    dest,
                    reply: waiting,
                    whole: false,
                });
            }
            Err(e) => {
                log::warn!("Sunrise: the server's profile cannot be saved: {e}");
                self.client.profile.server_link.reset();
                if let Some(waiting) = waiting {
                    let _ = waiting.send(reply_text(&Failed {
                        error: e.to_string(),
                    }));
                }
            }
        }
    }

    /// The next bytes of the server's file.
    fn part_chunk(&mut self, chunk: &[u8]) {
        use sunrise_sim::perf_net::Received;
        if self.client.profile.incoming.is_none() {
            return;
        }
        let received = self
            .client
            .profile
            .server_link
            .chunk(chunk.len(), Instant::now());
        let step = match received {
            Received::Stray => return,
            Received::TooMuch => Err("the server sent more than it announced".to_owned()),
            Received::More | Received::Last => self.take_part(chunk, received == Received::Last),
        };
        if let Err(why) = step {
            log::warn!("Sunrise: the server's profile part is unusable: {why}");
            self.abandon_part();
        }
    }

    fn take_part(&mut self, chunk: &[u8], last: bool) -> std::result::Result<(), String> {
        let Some(incoming) = &mut self.client.profile.incoming else {
            return Ok(());
        };
        let fields = incoming.merger.push(chunk).map_err(|e| e.to_string())?;
        match &mut incoming.dest {
            Dest::Window { held, .. } => held.extend_from_slice(&fields),
            Dest::Own { file, .. } => file.write_all(chunk).map_err(|e| e.to_string())?,
        }
        if last {
            incoming.merger.finish().map_err(|e| e.to_string())?;
            incoming.whole = true;
        }
        self.settle_part()
    }

    /// Hands the window's writer what the server's file has yielded, and completes the transfer once all of it is in.
    fn settle_part(&mut self) -> std::result::Result<(), String> {
        let Some(incoming) = &mut self.client.profile.incoming else {
            return Ok(());
        };
        if let Dest::Window { sink, held } = &mut incoming.dest
            && !held.is_empty()
        {
            match sink.sender.try_send_raw(std::mem::take(held)) {
                RawSend::Sent => {}
                RawSend::Full(back) => *held = back,
                RawSend::Closed => return Err("the capture's file is closed".to_owned()),
            }
        }
        let drained = match &incoming.dest {
            Dest::Window { held, .. } => held.is_empty(),
            Dest::Own { .. } => true,
        };
        if incoming.whole && drained {
            self.part_done();
        }
        Ok(())
    }

    /// The server's file is whole: the window's capture can finish, or the file of its own is kept.
    fn part_done(&mut self) {
        let Some(Incoming { dest, reply, .. }) = self.client.profile.incoming.take() else {
            return;
        };
        let Dest::Own { file, path } = dest else {
            log::info!("Sunrise: the server's profile is part of this capture");
            return;
        };
        let kept = file.sync_all().and_then(|()| {
            let dir = path.parent().map(PathBuf::from).unwrap_or_default();
            files::retain(&dir, &path, files::KEEP_FILES, files::KEEP_BYTES)
        });
        let text = match kept {
            Ok(deleted) => {
                log::info!("Sunrise: {}", files::saved_line(&path));
                reply_text(&Stopped {
                    ok: true,
                    sides: vec![ProfileSide::Server],
                    path: shown(&path),
                    open_at: sunrise_client::client_perf::request::viewer(),
                    removed: deleted.iter().map(|p| shown(p)).collect(),
                })
            }
            Err(e) => reply_text(&Failed {
                error: e.to_string(),
            }),
        };
        if let Some(reply) = reply {
            let _ = reply.send(text);
        }
    }

    /// The server's part will not arrive whole: what did arrive stays in the window's capture, with the warning, or the file of its own is removed.
    pub(in crate::game::client_side) fn abandon_part(&mut self) {
        let waiting = self.client.profile.server_reply.take();
        let sink = self.client.profile.server_sink.take();
        if let Some(Incoming {
            dest,
            reply,
            merger,
            ..
        }) = self.client.profile.incoming.take()
        {
            match dest {
                Dest::Window { sink, held } => {
                    if !held.is_empty() {
                        let _ = sink.sender.try_send_raw(held);
                    }
                    sink.incomplete(&merger);
                }
                Dest::Own { file, path } => {
                    drop(file);
                    let _ = std::fs::remove_file(path);
                    if let Some(reply) = reply {
                        let _ = reply.send(reply_text(&Failed { error: part_text() }));
                    }
                }
            }
        }
        if let Some(sink) = sink {
            sink.incomplete(&PartMerger::new());
        }
        if let Some(waiting) = waiting {
            let _ = waiting.send(reply_text(&Failed { error: part_text() }));
        }
    }

    /// Once per loop turn: a request the server never answered, or a connection that ended under one.
    pub(in crate::game::client_side) fn poll_server_link(&mut self) {
        if self.client.profile.incoming.is_some() && self.settle_part().is_err() {
            self.abandon_part();
        }
        let now = Instant::now();
        let lost = self.client.remote.is_none() && !self.client.profile.server_link.is_idle();
        let silent = self.client.profile.server_link.timed_out(now);
        let stalled = self.client.profile.server_link.stalled(now);
        if !lost && !silent && !stalled {
            return;
        }
        if lost {
            self.client.profile.server_link.reset();
        }
        if lost || stalled {
            self.abandon_part();
        }
        self.client.profile.server_joined = false;
        if let Some(waiting) = self.client.profile.server_reply.take() {
            let error = if lost {
                "the connection to the server ended".to_owned()
            } else {
                "the server did not answer; it may not run a Sunrise with server profiles"
                    .to_owned()
            };
            let _ = waiting.send(reply_text(&Failed { error }));
        }
    }
}
