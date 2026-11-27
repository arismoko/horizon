//! A server's resource packs bound to the game (`ClientCommonPacketListenerImpl.
//! handleResourcePackPush` and `handleResourcePackPop`, its `PackConfirmScreen`,
//! and `DownloadedPackSource` as `Minecraft` drives it): the prompt, the
//! server list's setting, downloads into `downloads/`, and a resource reload
//! with the server's packs on top of the player's own.
use super::{
    packs::ReloadKind,
    toast_manager::{SystemToastId, Toast},
};
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};
use sunrise_client::{
    confirm_screen::Confirm,
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer, packs,
    server_packs::{Prompt, ServerPacks, action, download, parse_sha1, valid_url},
    toasts::SystemToast,
    ui,
};
use sunrise_game_data::{
    Result,
    scoreboard::{comp_text, tag_to_json},
    text::{Text, named_color},
};
use sunrise_protocol::{packets::configuration::ResourcePackPush, servers};

/// `ClientCommonPacketListenerImpl.PackConfirmScreen`: the packs it asks
/// about, whether the last one is required, and the server's message.
pub struct PackPrompt {
    requests: Vec<(u128, String, String)>,
    required: bool,
    prompt: Option<Text>,
}
impl PackPrompt {
    fn texts(&self) -> (Text, Text, [Text; 2]) {
        let t = |key: &str| Text::translate(key, &[]);
        let (title, line2) = if self.required {
            let yellow = named_color("yellow").unwrap_or(0xFFFF55);
            (
                t("multiplayer.requiredTexturePrompt.line1"),
                t("multiplayer.requiredTexturePrompt.line2")
                    .with_color(yellow)
                    .bold(),
            )
        } else {
            (
                t("multiplayer.texturePrompt.line1"),
                t("multiplayer.texturePrompt.line2"),
            )
        };
        // preparePackPrompt.
        let message = match &self.prompt {
            Some(prompt) => Text::translate(
                "multiplayer.texturePrompt.serverPrompt",
                &[line2, prompt.clone()],
            ),
            None => line2,
        };
        let buttons = if self.required {
            [t("gui.proceed"), t("menu.disconnect")]
        } else {
            [t("gui.yes"), t("gui.no")]
        };
        (title, message, buttons)
    }
    fn layout(&self, atlas: &HudAtlas, size: (f32, f32)) -> Confirm {
        let (title, message, _) = self.texts();
        Confirm::new(&atlas.font, size, &title, &message)
    }
}

/// The game's `DownloadedPackSource`.
#[derive(Default)]
pub struct ServerPackState {
    pub packs: ServerPacks,
    /// `downloads/` in the game folder, opened at the first download.
    queue: Option<download::Queue>,
    pub prompt: Option<PackPrompt>,
    /// The server list entry the setting is saved to (none: a direct join).
    pub(in crate::game) address: Option<String>,
    /// That entry's name, with `address`, so a pack answer updates only that row.
    pub(in crate::game) name: Option<String>,
    /// The account the downloads are made for (the download headers).
    user: (String, u128),
    /// The packs loaded on top of the player's own, bottom first.
    pub(in crate::game) active: Vec<packs::Pack>,
    /// The download batches' notifiers, oldest first.
    notifiers: Vec<PackNotifier>,
    /// Distinguish batches while their download notifications are still queued or visible.
    next_download_toast: usize,
}
impl ServerPackState {
    /// Submit every request to the pack manager and save the server list. A missing
    /// policy change leaves the current entry alone but still saves the list.
    fn complete_prompt(&mut self, prompt: PackPrompt, policy_change: Option<bool>) {
        for (id, url, hash) in prompt.requests {
            self.packs.push(id, &url, parse_sha1(&hash));
        }
        // ServerList.saveSingleServer: one row, matched by name and address.
        let (Some(address), Some(name)) = (&self.address, &self.name) else {
            return;
        };
        let file = multiplayer::servers_file(&saves());
        let _guard = servers::lock();
        let (mut shown, mut hidden) = servers::load_all(&file);
        if let Some(setting) = policy_change {
            servers::save_single_accept_textures(
                &mut shown,
                &mut hidden,
                name,
                address,
                Some(setting),
            );
        }
        if let Err(e) = servers::save_all(&file, &shown, &hidden) {
            log::warn!("Sunrise: could not save {}: {e}", file.display());
        }
    }
}
/// `DownloadedPackSource.createDownloadNotifier`: one batch's progress in its
/// own `SystemToast`.
pub struct PackNotifier {
    progress: Receiver<download::Progress>,
    total: usize,
    /// `count`: the requests started.
    count: usize,
    /// `failCount`.
    failed: usize,
    /// `totalBytes`: the Content-Length of the request in flight.
    size: Option<u64>,
    /// The bytes it has written so far: none before the first response.
    bytes: Option<u64>,
    /// The last request has finished.
    done: bool,
    id: usize,
    toast_cancelled: bool,
}
impl PackNotifier {
    pub fn new(progress: Receiver<download::Progress>, total: usize, id: usize) -> Self {
        Self {
            progress,
            total,
            count: 0,
            failed: 0,
            size: None,
            bytes: None,
            done: false,
            id,
            toast_cancelled: false,
        }
    }
    /// Update once for the accumulated events. Returns whether to keep polling
    /// the channel and whether a successful download should hide its toast.
    pub fn poll(&mut self, update: impl FnOnce(SystemToast) -> bool) -> (bool, bool) {
        let (mut changed, mut hide) = (false, false);
        let ended = loop {
            let event = match self.progress.try_recv() {
                Ok(event) => event,
                Err(e) => break e == TryRecvError::Disconnected,
            };
            changed |= match event {
                download::Progress::RequestStart => {
                    self.count += 1;
                    true
                }
                download::Progress::DownloadStart(size) => {
                    self.size = size;
                    self.bytes = Some(0);
                    true
                }
                download::Progress::Bytes(bytes) => {
                    self.bytes = Some(bytes);
                    true
                }
                download::Progress::RequestFinished(ok) => {
                    self.failed += usize::from(!ok);
                    self.done = self.count == self.total;
                    // Every pack came: forceHide; else the failure title.
                    hide |= self.done && self.failed == 0;
                    self.done && self.failed > 0
                }
            };
        };
        if changed && !self.toast_cancelled {
            let (title, message) = self.text();
            self.toast_cancelled = !update(SystemToast::new(title, message));
        }
        (!ended, hide)
    }
    /// The toast's title and message.
    pub fn text(&self) -> (Text, Option<Text>) {
        if self.done && self.failed > 0 {
            let failed = Text::tr("download.pack.failed", &[&self.failed, &self.total]);
            return (failed, None);
        }
        let title = Text::tr("download.pack.title", &[&self.count, &self.total]);
        let message = self.bytes.map(|bytes| match self.size {
            Some(size) if size > 0 => {
                Text::tr("download.pack.progress.percent", &[&(bytes * 100 / size)])
            }
            _ => Text::tr(
                "download.pack.progress.bytes",
                &[&sunrise_client::realms::human_readable(bytes)],
            ),
        });
        (title, message)
    }
}
pub(in crate::game::client_side) fn saves() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("saves")
}

/// `WorldOpenFlows.WORLD_PACK_ID`: the id of a world's own resource pack.
const WORLD_PACK_ID: u128 = 0x640a6a92_b6cb_48a0_b391_831586500359;

impl crate::game::ClientGame<'_> {
    /// `WorldOpenFlows.loadBundledResourcePack`: a world's own
    /// `resourcepacks/resources.zip` is pushed as a local pack before the world
    /// connects, after `configureForLocalWorld` so the pack is accepted at once.
    pub(in crate::game::client_side) fn server_packs_world_pack(&mut self, folder: &str) {
        let path = saves()
            .join(folder)
            .join("resourcepacks")
            .join("resources.zip");
        if path.is_file() {
            self.client.server_packs.0.packs.local_world();
            self.client
                .server_packs
                .0
                .packs
                .push_local(WORLD_PACK_ID, path);
        }
    }
    /// `ConnectScreen`: the server's setting decides what its packs do
    /// (`configureForServerControl`); single player's own server takes them
    /// all, quietly (`configureForLocalWorld`).
    pub fn server_packs_connect(
        &mut self,
        address: Option<&str>,
        list_name: Option<&str>,
        user: (String, u128),
    ) {
        let state = &mut self.client.server_packs.0;
        state.user = user;
        state.prompt = None;
        state.notifiers.clear();
        let Some(address) = address else {
            state.address = None;
            state.name = None;
            state.packs.local_world();
            return;
        };
        let file = multiplayer::servers_file(&saves());
        // ServerList.get: the shown entries, then the hidden ones.
        let (shown, hidden) = servers::load_all(&file);
        let entry = servers::find(&shown, &hidden, address, list_name).cloned();
        let setting = entry.as_ref().and_then(|e| e.accept_textures);
        state.address = entry.as_ref().map(|e| e.ip.clone());
        state.name = entry.map(|e| e.name);
        state.packs.connect(Prompt::for_server(setting));
        // GetServerDetailsTask.scheduleResourcePackDownload: the Realm's own pack, which the
        // player agreed to before connecting (`allowServerPacks`, then `pushPack`).
        if let Some(pack) = sunrise_client::realms::take_pending_pack(address) {
            state.packs.allow();
            state.packs.push(pack.id, &pack.url, parse_sha1(&pack.hash));
        }
    }
    /// `Minecraft.clearDownloadedResourcePacks`: every server pack goes at once.
    pub(in crate::game::client_side) fn server_packs_disconnect(&mut self) {
        self.client.server_packs.0.prompt = None;
        self.client.server_packs.0.notifiers.clear();
        self.client.server_packs.0.packs.disconnect();
        self.tick_server_packs();
    }

    /// `handleResourcePackPush`.
    pub fn handle_pack_push(&mut self, p: ResourcePackPush) {
        if !valid_url(&p.url) {
            self.send_pack_status(p.id, action::INVALID_URL);
            return;
        }
        let setting = self.server_setting();
        let prompt = Prompt::for_server(setting);
        if prompt != Prompt::Pending && (!p.required || prompt != Prompt::Declined) {
            self.client
                .server_packs
                .0
                .packs
                .push(p.id, &p.url, parse_sha1(&p.hash));
            // `acceptPack`/`pushPack` report ACCEPTED or DECLINED from the packet handler, so
            // it reaches the server in this phase: before a FinishConfiguration behind it.
            self.send_pack_replies();
            return;
        }
        // addOrUpdatePackPrompt: one prompt for every pack pushed before it is answered.
        let text = p.prompt.as_ref().map(|t| comp_text(&tag_to_json(t)));
        // A new prompt opens as a screen (a mod may keep it shut, and then
        // nothing answers the pack); an open one only takes the pack.
        if self.client.server_packs.0.prompt.is_none()
            && !self.open_screen_overlay("server_pack_prompt", |game| {
                game.client.server_packs.0.prompt = Some(PackPrompt {
                    requests: Vec::new(),
                    required: p.required,
                    prompt: None,
                });
            })
        {
            return;
        }
        let state = &mut self.client.server_packs.0;
        let open = state.prompt.get_or_insert(PackPrompt {
            requests: Vec::new(),
            required: p.required,
            prompt: None,
        });
        open.requests.push((p.id, p.url, p.hash));
        open.required = p.required;
        open.prompt = text;
        // Gui.setScreen: KeyMapping.releaseAll, so no walking behind the prompt.
        self.clear_device_keys();
        self.capture(false);
    }
    /// `handleResourcePackPop`: one pack, or all of them.
    pub(in crate::game::client_side) fn handle_pack_pop(&mut self, id: Option<u128>) {
        match id {
            Some(id) => self.client.server_packs.0.packs.pop(id),
            None => self.client.server_packs.0.packs.pop_all(),
        }
    }
    /// The joined server's entry in the server list: its `acceptTextures`.
    fn server_setting(&self) -> Option<bool> {
        let address = self.client.server_packs.0.address.as_deref()?;
        let (shown, hidden) = servers::load_all(&multiplayer::servers_file(&saves()));
        servers::find(
            &shown,
            &hidden,
            address,
            self.client.server_packs.0.name.as_deref(),
        )?
        .accept_textures
    }
    fn send_pack_status(&self, id: u128, status: i32) {
        if let Some(r) = &self.client.remote {
            r.session.send_pack_status(id, status);
        }
    }

    /// `DownloadedPackSource`'s feedback: `connection.send` as each status is reported.
    fn send_pack_replies(&mut self) {
        for (id, status) in self.client.server_packs.0.packs.take_replies() {
            self.send_pack_status(id, status);
        }
    }

    /// `PackConfirmScreen`'s callback: push every pending pack and save the server list.
    /// Declining a required pack closes the connection without changing its stored policy.
    pub fn answer_pack_prompt(&mut self, yes: bool) -> Result<()> {
        let Some(prompt) = self.client.server_packs.0.prompt.take() else {
            return Ok(());
        };
        if self.common.player.is_some() && self.client.screen == super::Screen::Playing {
            self.capture(true);
        }
        if yes {
            self.client.server_packs.0.packs.allow();
        } else {
            self.client.server_packs.0.packs.reject();
            if prompt.required {
                let reason = Text::translate("multiplayer.requiredTexturePrompt.disconnect", &[]);
                // The requests still get local DECLINED feedback. The transport is already
                // closed, so cleanup drops the replies without sending them.
                return self.close_server_session(Some(reason), |state| {
                    state.complete_prompt(prompt, None);
                });
            }
        }
        self.refresh_screen_widgets();
        self.client
            .server_packs
            .0
            .complete_prompt(prompt, Some(yes));
        self.send_pack_replies();
        Ok(())
    }

    /// Runs what the packs need (`createUpdateScheduler`): the downloads
    /// that finished, the ones to start, a resource reload when the active
    /// packs change, and the statuses for the server.
    pub fn tick_server_packs(&mut self) {
        let mut notifiers = std::mem::take(&mut self.client.server_packs.0.notifiers);
        notifiers.retain_mut(|n| {
            let id = SystemToastId::PackDownload(n.id);
            let (keep, hide) = n.poll(|new| self.add_or_update_system_toast(id, new));
            if hide && let Some(toast) = self.client.toast_manager.system_mut(id) {
                toast.force_hide();
            }
            keep
        });
        self.client.server_packs.0.notifiers = notifiers;
        // A reload waits for the world's resources (`reload_packs`).
        if self.common.rest_loader.is_some() {
            return;
        }
        self.drive_pack_reload();
        self.drive_pack_browser();
        // One reload at a time: the player's own waits under its overlay
        // before the server's packs can ask for theirs (`ServerPacks` keeps
        // its own reload from overlapping).
        let selection_reloading = self
            .client
            .pack_reload
            .0
            .as_ref()
            .is_some_and(|r| r.kind() == ReloadKind::Selection);
        let finished = self
            .client
            .server_packs
            .0
            .queue
            .as_ref()
            .map(download::Queue::finished)
            .unwrap_or_default();
        for batch in finished {
            self.client.server_packs.0.packs.downloaded(batch);
        }
        while !selection_reloading && self.client.server_packs.0.packs.wants_tick() {
            let tick = self.client.server_packs.0.packs.tick();
            if !tick.download.is_empty() {
                self.start_pack_downloads(tick.download);
            }
            if let Some(load) = tick.reload {
                self.start_pack_reload(&load);
            }
        }
        self.send_pack_replies();
    }
    fn start_pack_downloads(&mut self, requests: Vec<sunrise_client::server_packs::Request>) {
        let state = &mut self.client.server_packs.0;
        let toast_id = state.next_download_toast;
        state.next_download_toast += 1;
        if state.queue.is_none() {
            let dir = std::env::current_dir()
                .unwrap_or_default()
                .join("downloads");
            match download::Queue::open(dir) {
                Ok(queue) => state.queue = Some(queue),
                Err(e) => {
                    log::error!("Sunrise: failed to open the download queue: {e}");
                    // Every request failed, as the notifier would hear it.
                    let (progress, events) = std::sync::mpsc::channel();
                    for _ in &requests {
                        let _ = progress.send(download::Progress::RequestStart);
                        let _ = progress.send(download::Progress::RequestFinished(false));
                    }
                    state
                        .notifiers
                        .push(PackNotifier::new(events, requests.len(), toast_id));
                    let failed = requests.iter().map(|r| r.id).collect();
                    state.packs.downloaded(sunrise_client::server_packs::Batch {
                        downloaded: Vec::new(),
                        failed,
                    });
                    return;
                }
            }
        }
        let headers = download::headers(&state.user.0, state.user.1);
        let (progress, events) = std::sync::mpsc::channel();
        state
            .notifiers
            .push(PackNotifier::new(events, requests.len(), toast_id));
        if let Some(queue) = &state.queue {
            queue.download(requests, headers, progress);
        }
    }
    /// `DownloadedPackSource.startReload`: the packs on top of the player's
    /// own, staged on a worker (`Minecraft.reloadResourcePacks` under a
    /// `LoadingOverlay`); `drive_pack_reload` applies it. A pack with no
    /// readable `pack.mcmeta` fails the reload at once, as `onFailure`.
    fn start_pack_reload(&mut self, load: &[(u128, PathBuf)]) {
        let server: Option<Vec<_>> = load
            .iter()
            .map(|(id, path)| packs::server_pack(*id, path))
            .collect();
        let Some(server) = server else {
            log::warn!("Sunrise: invalid pack metadata in a server pack, ignoring all");
            self.finish_pack_reload(false);
            return;
        };
        self.client.server_packs.0.active = server;
        if let Err(e) = self.spawn_pack_reload(ReloadKind::Server) {
            log::error!("Sunrise: could not start the pack reload worker: {e}");
            self.finish_pack_reload(false);
        }
    }
    /// `ServerPacks::reloaded` and the recovery reload it can ask for.
    pub(in crate::game::client_side) fn finish_pack_reload(&mut self, ok: bool) {
        if let Some(recovery) = self.client.server_packs.0.packs.reloaded(ok) {
            self.start_pack_reload(&recovery);
        }
    }

    pub(in crate::game::client_side) fn draw_pack_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.client.server_packs.0.prompt else {
            return;
        };
        sunrise_client::widgets::cover_frame();
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        if self.common.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(
                atlas,
                0.0,
                0.0,
                w,
                h,
                sunrise_client::hud::rgba(20, 20, 20, 255),
            );
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.common.player.is_some());
        let (title, message, [yes, no]) = prompt.texts();
        // PackConfirmScreen is a ConfirmScreen.
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::confirm("pack_prompt", &title.plain(), &message.plain())
        });
        let mut layout = prompt.layout(atlas, (w, h));
        layout.declare_no();
        layout.draw(batch, atlas, self.client.cursor, [&yes, &no]);
        self.draw_overlay_mounted_widgets(batch, "server_pack_prompt");
    }

    /// Admit new server reports in the receiving window. `SystemToast.addOrUpdate`
    /// resets an existing chunk toast instead of admitting a second one.
    pub(in crate::game::client_side) fn add_server_report_toasts(&mut self) {
        let pending = self.client.client_reports.0.take();
        let toasts = [
            pending
                .low_disk_space
                .then(sunrise_client::toasts::low_disk_space_toast),
            pending
                .load_failure
                .map(|(x, z)| sunrise_client::toasts::load_failure_toast(x, z)),
            pending
                .save_failure
                .map(|(x, z)| sunrise_client::toasts::save_failure_toast(x, z)),
        ];
        for (index, new) in toasts.into_iter().enumerate() {
            let Some(new) = new else { continue };
            self.add_or_update_system_toast(SystemToastId::ChunkIo(index), new);
        }
        for mod_id in pending.scheduled_task_failures {
            let notice = SystemToast::new(
                Text::tr("sunrise.mods.scheduled_task_error.title", &[]),
                Some(Text::tr(
                    "sunrise.mods.scheduled_task_error.message",
                    &[&mod_id],
                )),
            )
            .with_title_color(rgba(255, 85, 85, 255));
            self.add_system_toast(None, notice);
        }
    }

    /// Queued hook-listener panics from any thread, each shown once per
    /// `(hook, mod)` as a red-titled system toast. Enqueued directly on the
    /// client toast queue rather than through `toast_add` so a mod cannot veto
    /// or rewrite its own failure notice; headless servers log the error and do
    /// not drain the client queue.
    pub fn add_hook_panic_toasts(&mut self) {
        if self.common.options.dedicated.is_some() {
            return;
        }
        for report in mod_api::__private::take_hook_panics() {
            let toast = SystemToast::new(
                Text::tr("sunrise.mods.hook_panic.title", &[]),
                Some(Text::tr(
                    "sunrise.mods.hook_panic.description",
                    &[&report.mod_id, &report.hook],
                )),
            )
            .with_title_color(sunrise_client::hud::rgba(255, 85, 85, 255));
            // Audio playback notices retain the player's existing F1 visibility rule.
            let hide_with_f1 = matches!(report.hook, "AudioStream::read" | "AudioStream::drop");
            let clock = sunrise_client::toasts::ToastClock::of(self.common.clock.still());
            self.client.toast_manager.post(
                clock,
                Toast::Diagnostic {
                    toast,
                    hide_with_f1,
                },
            );
        }
    }

    /// A left click on the prompt: its answer when it hit a button.
    pub(in crate::game::client_side) fn pack_prompt_click(&mut self) -> Result<bool> {
        let Some(prompt) = &self.client.server_packs.0.prompt else {
            return Ok(false);
        };
        let answer = prompt
            .layout(&self.client.gui.atlas, self.gui_size())
            .answer_at(self.client.cursor);
        match answer {
            Some(yes) => {
                self.play_audio_button_click();
                self.answer_pack_prompt(yes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}
