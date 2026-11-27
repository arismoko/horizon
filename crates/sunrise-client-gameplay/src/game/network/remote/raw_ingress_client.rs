use super::ingress_state::ListenerTurn;
pub(in crate::game::client_side) use super::ingress_state::PacketError;
use super::ingress_state::local_transfer_notice;
use super::ingress_state::packet_error_notice;
use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientCommonPacketListenerImpl.handlePostEffects`.
    pub(in crate::game::client_side) fn apply_post_effects_packet(
        &mut self,
        p: play::PostEffects,
    ) -> Result<()> {
        if !self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.logged_in && r.reconfiguring.is_none())
        {
            return Err(PacketError(
                "Cannot set player post effects before the player is created.",
            )
            .into());
        }
        self.client.post_effects = p.effects;
        Ok(())
    }
    pub fn apply_staged_attachment_sync(
        &mut self,
        staged: sunrise_protocol::attachment_sync::StagedMessage,
    ) {
        let (message, payload) = staged.into_message();
        self.observe_client_packet(&C::CustomPayload(payload));
        self.remote_attachment_sync(message);
    }
    pub(in crate::game::client_side) fn dispatch_packet_turn(
        &mut self,
        mut events: VecDeque<Received>,
        overdue: bool,
        before_tick: bool,
        chunks: &mut Duration,
    ) -> Result<()> {
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = sunrise_level::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        let mut turn = ListenerTurn::default();
        while let Some(item) = events.pop_front() {
            // PacketProcessor checks eligibility for the queued outer action,
            // before any handler, including worker-decoded columns.
            let needs_open = match &item {
                Received::Event(event, _) => event.requires_open_listener(),
                Received::Chunk(..) => true,
            };
            if needs_open
                && !self
                    .client
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.session.accepts_packets())
            {
                continue;
            }
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            sunrise_level::spans::time("pr_forget", || self.forget_remote_packets(&mut forgotten));
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk && !overdue && !before_tick && *chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.client.remote.as_ref().map(|r| {
                        if before_tick {
                            r.decoder.wait_next(TICK_DECODE_WAIT)
                        } else {
                            r.decoder.next()
                        }
                    }) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Sunrise: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    if !self
                        .client
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.session.accepts_packets())
                    {
                        continue;
                    }
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        sunrise_level::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    *chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            self.handle_remote_event(event, &mut turn)?;
            if turn.opening.is_some() {
                // The handoff already closed reader admission. Freeze any
                // published tail before dropping any older receipt, then keep
                // the old listener installed for its eligible common tasks.
                let tail = self
                    .client
                    .remote
                    .as_ref()
                    .map(|old| old.session.poll())
                    .unwrap_or_default();
                let admitted = events
                    .into_iter()
                    .filter_map(|item| match item {
                        Received::Event(event, credit) => Some((event, credit)),
                        Received::Chunk(..) => None,
                    })
                    .chain(tail.into_iter().map(|receipt| receipt.into_parts()));
                for (event, _credit) in admitted {
                    if matches!(event, Event::Cookie(_) | Event::Transfer(_)) {
                        self.handle_remote_event(event, &mut turn)?;
                    }
                }
                self.install_remote_transfer(
                    turn.opening.take().expect("handoff kept its first opening"),
                )?;
                return Ok(());
            }
            *chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.client.remote.is_none() {
                break;
            }
        }
        if let Some(end) = turn.end.take() {
            self.finish_remote_disconnect(end)?;
            return Ok(());
        }
        if self.client.remote.is_some() {
            sunrise_level::spans::time("pr_forget", || self.forget_remote_packets(&mut forgotten));
        }
        if let Some(r) = self.client.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        Ok(())
    }
    pub(in crate::game::client_side) fn finish_remote_disconnect(
        &mut self,
        event: Event,
    ) -> Result<()> {
        let Event::Disconnected {
            reason,
            key,
            from_server,
            after_login,
        } = event
        else {
            unreachable!("listener only retains the disconnect notice");
        };
        if let Some(remote) = &self.client.remote {
            remote.session.shutdown_and_wait();
        }
        self.reset_book_view();
        log::info!("Sunrise: disconnected: {reason}");
        let by = if from_server {
            sunrise_hooks::connection::By::Server
        } else {
            sunrise_hooks::connection::By::Connection
        };
        self.connection_ended(&reason, key.as_deref(), by);
        // Only the common listener calls Minecraft.disconnect; a failed
        // ConnectScreen or login handshake just changes screens.
        let screen_reason = match key.as_deref() {
            Some(k) if sunrise_game_data::lang::get(k) == reason => Text::translate(k, &[]),
            _ => Text::literal(reason),
        };
        if after_login {
            self.leave_server(Some(screen_reason))
        } else {
            self.finish_leave_server(Some(screen_reason))
        }
    }
    pub(in crate::game::client_side) fn handle_listener_transfer(
        &mut self,
        packet: sunrise_protocol::packets::configuration::Transfer,
        turn: &mut ListenerTurn,
    ) -> Result<()> {
        // ConnectScreen.startConnecting keeps the first opening, even if a
        // later eligible Transfer runs as a separate old-listener task.
        if turn.opening.is_none() {
            turn.opening = self.start_remote_transfer(packet)?;
            if turn.opening.is_none() {
                turn.end.get_or_insert_with(local_transfer_notice);
            }
        }
        Ok(())
    }
    /// `ClientPacketListener.handleBundlePacket`: use one main-thread turn, so
    /// the chunk decoder/frame quota cannot split a bundle at a column packet.
    pub fn handle_play_bundle(
        &mut self,
        bundle: sunrise_client_lib::net::CompletedBundle,
        turn: &mut ListenerTurn,
    ) -> Result<()> {
        use sunrise_client_lib::net::{BundleMember, BundleStep};
        use sunrise_protocol::{connection_groups, payload_negotiation::Phase};
        use sunrise_sim::raw_payloads::{Classified, Incoming, dispatch_bundle};
        let sunrise_client_lib::net::CompletedBundle { packets, io } = bundle;
        let Some(remote) = self.client.remote.as_ref() else {
            return Ok(());
        };
        if !io.accepts_bundle() {
            return Ok(());
        }
        let payloads = remote.session.payload_transport();
        let negotiated = payloads.epoch().is_some_and(|epoch| {
            epoch.phase == Phase::Play
                && epoch
                    .peer
                    .engine_receives(Phase::Play, connection_groups::CHANNEL)
        });
        let channel = |member: &BundleMember, channel: &str| matches!(member, BundleMember::Packet(C::CustomPayload(p)) if p.channel == channel);
        if packets
            .iter()
            .any(|member| channel(member, connection_groups::CHANNEL))
            || (packets.len() > 1
                && packets
                    .iter()
                    .all(|member| channel(member, sunrise_protocol::viewed_levels::CHANNEL)))
        {
            let Some(group_packets) = packets
                .into_iter()
                .map(|member| match member {
                    BundleMember::Packet(packet) => Some(packet),
                    BundleMember::AttachmentSync(staged) => {
                        Some(C::CustomPayload(staged.into_message().1))
                    }
                    BundleMember::OversizedDebugSample => None,
                })
                .collect::<Option<Vec<_>>>()
            else {
                return self.refuse_remote_input("connection groups: unexpected packet in bundle");
            };
            self.route_connection_group_bundle(&group_packets, negotiated)?;
            return Ok(());
        }
        // One PacketProcessor task: a handler failure leaves the prefix applied and skips the suffix.
        for packet in packets {
            match io.bundle_member(packet) {
                BundleStep::Event(event) => {
                    if let Err(error) = self.handle_remote_event(event, turn) {
                        log::warn!("Sunrise: bundle packet handler failed: {error}");
                        self.record_packet_error(turn, || io.packet_error());
                        break;
                    }
                    if self.client.remote.is_none() || turn.packet_error {
                        break;
                    }
                }
                BundleStep::HandlerFailed => {
                    log::warn!("Sunrise: oversized debug sample in play bundle");
                    self.record_packet_error(turn, || io.packet_error());
                    break;
                }
                BundleStep::Payload(payload) => {
                    let Some(endpoint) = self
                        .client
                        .remote
                        .as_ref()
                        .map(|remote| remote.session.payload_endpoint())
                    else {
                        break;
                    };
                    match Incoming::classify(endpoint.transport(), Phase::Play, payload) {
                        Ok(Classified::Application(incoming)) => {
                            for task in dispatch_bundle(incoming, &endpoint) {
                                self.run_payload(endpoint.clone(), task);
                            }
                        }
                        Ok(Classified::Other(payload)) => {
                            self.handle_remote_event(
                                Event::Packet(Box::new(C::CustomPayload(payload))),
                                turn,
                            )?;
                        }
                        Ok(Classified::Skipped | Classified::Raw(_)) => {}
                        Err(error) => {
                            log::warn!("Sunrise: bundle payload refused: {error}");
                            if let Some(remote) = &self.client.remote {
                                remote.session.close_after_pending();
                            }
                            turn.end
                                .get_or_insert_with(|| io.incompatible_payload(&error.to_string()));
                        }
                    }
                }
                BundleStep::Transfer(packet) => self.handle_listener_transfer(packet, turn)?,
                BundleStep::Close(notice) => {
                    if let Some(remote) = &self.client.remote {
                        remote.session.close_after_pending();
                    }
                    // Discard the old listener only after its remaining handlers run.
                    turn.end.get_or_insert(notice);
                }
                BundleStep::Continue => {}
            }
        }
        Ok(())
    }
    pub(in crate::game::client_side) fn handle_remote_event(
        &mut self,
        event: Event,
        turn: &mut ListenerTurn,
    ) -> Result<()> {
        match event {
            Event::Status(s) => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.status = s;
                }
            }
            Event::Joined {
                registries,
                datapacks,
                name,
                uuid,
            } => {
                if let Err(error) = self.set_registries(registries) {
                    return self.refuse_remote_input(error);
                }
                // Vanilla `ClientConfigurationPacketListenerImpl.handleConfigurationFinished` ->
                // `RegistryDataCollector.collectGameRegistries` throws on registry decode failure
                // and disconnects via `ClientCommonPacketListenerImpl.onPacketError`.
                if let Err(error) = self.install_client_datapack_receiver(datapacks) {
                    log::warn!("Sunrise: failed to bind datapack vanilla values: {error}");
                    self.record_packet_error(turn, packet_error_notice);
                    return Ok(());
                }
                self.connection_joined();
                self.close_dialogs();
                if let Some(r) = self.client.remote.as_mut() {
                    r.joined = true;
                    r.profile = uuid;
                    r.profile_name = name.clone();
                    r.status = sunrise_game_data::lang::get_or(
                        "multiplayer.downloadingTerrain",
                        "Loading terrain...",
                    );
                }
                self.begin_book_negotiation();
            }
            Event::Features(features) => {
                if let Some(remote) = &mut self.client.remote {
                    // ClientConfigurationPacketListenerImpl.handleEnabledFeatures resolves
                    // network identifiers through FeatureFlags.REGISTRY.fromNames.
                    let flags: sunrise_sim::save::FeatureFlagIds = features
                        .iter()
                        .filter_map(|id| {
                            let flag = mod_api::FeatureFlag::from_id(id);
                            if flag.is_none() {
                                log::warn!("Unknown feature flag: {id}");
                            }
                            flag
                        })
                        .collect();
                    remote.features = flags.as_arc().clone();
                }
            }
            Event::Brand(brand) => {
                if let Some(remote) = &mut self.client.remote {
                    remote.brand = brand;
                }
            }
            Event::QuietCommands => {
                if let Some(remote) = &mut self.client.remote {
                    remote.quiet_commands = true;
                }
            }
            Event::CustomBooks => {
                if let Some(remote) = &mut self.client.remote {
                    remote.custom_books = true;
                }
            }
            Event::FarTerrain(offer) => {
                if let Some(remote) = &mut self.client.remote {
                    remote.far_terrain = Some(offer);
                }
            }
            Event::Cookie(task) => task.handle(),
            Event::Configuration(task) => {
                if let Some(effect) = task.handle() {
                    self.handle_remote_event(effect, turn)?;
                }
            }
            Event::FinishConfiguration(change) => change.acknowledge(),
            Event::Reconfiguring(resume) => {
                self.reset_book_view();
                // handleConfigurationStart: the held-back chat shows, then
                // clearClientLevel opens ServerReconfigScreen over no level.
                self.flush_chat_queue();
                self.send_chat_acknowledgement();
                self.close_dialogs();
                // clearClientLevel posts Unload before clearing the level.
                self.unload_client_level();
                // Minecraft.clearClientLevel -> updateLevelInEngines(null) stops
                // sound immediately after the level-unload event fires.
                self.stop_sounds_for_new_level();
                // The server configuring us now (a proxy's next one)
                // offers its own far terrain, or none, and announces its
                // own features again.
                if let Some(remote) = &mut self.client.remote {
                    remote.datapacks = Default::default();
                    remote.datapack_vanilla_values = None;
                    remote.has_logical_player = false;
                    remote.viewed.clear();
                    remote.far_terrain = None;
                    remote.quiet_commands = false;
                    remote.custom_books = false;
                    remote.cauldron_entries = None;
                    remote.pending_cauldrons.get_mut().clear();
                }
                self.clear_lod();
                if let Some(remote) = &self.client.remote {
                    remote.session.reset_far_terrain();
                }
                self.remove_local_player_body();
                self.client.post_effects.clear();
                if let Some(r) = self.client.remote.as_mut() {
                    r.camera = None;
                }
                self.clear_spectated_entity_post_effect();
                self.set_screen(Screen::Playing);
                self.capture(false);
                self.forget_all_remote_columns();
                self.common.native_lifecycle.begin_close();
                self.native_lifecycle_world_closed();
                let ticks = self.common.ticks;
                if let Some(r) = self.client.remote.as_mut() {
                    self.client.packet_holds.borrow_mut().clear();
                    r.reconfiguring = Some(ticks);
                    r.server_chat_tree = None;
                    r.logged_in = false;
                }
                resume.acknowledge();
            }
            Event::Reconfigured {
                registries,
                datapacks,
            } => {
                if let Err(error) = self.set_registries(registries) {
                    return self.refuse_remote_input(error);
                }
                // handleConfigurationStart → clearClientLevel, and the next
                // ClientPacketListener starts with no players or scoreboard,
                // no waypoints (its own ClientWaypointManager: a proxy's next
                // server never untracks the last one's), and its new
                // ClientSuggestionProvider with no custom chat completions.
                // clearClientLevel clears the narrator.
                self.clear_narrator();
                self.hud_disconnected();
                self.common.score.board = Default::default();
                self.clear_waypoints();
                if let Some(r) = self.client.remote.as_mut() {
                    r.datapacks = Default::default();
                    r.datapack_vanilla_values = None;
                    r.clear_attachments();
                    r.players.clear();
                    r.tab_players.clear();
                    r.tab_game_modes.clear();
                    r.own_player_info = std::cell::OnceCell::new();
                    r.level_player_infos.clear();
                    r.has_logical_player = false;
                    r.listed.clear();
                    r.signatures = Default::default();
                    r.custom_completions.clear();
                }
                self.common.ui.registries.custom_completions.clear();
                // Vanilla `ClientConfigurationPacketListenerImpl.handleConfigurationFinished` ->
                // `RegistryDataCollector.collectGameRegistries` disconnects via `onPacketError`.
                if let Err(error) = self.install_client_datapack_receiver(datapacks) {
                    log::warn!("Sunrise: failed to bind datapack vanilla values: {error}");
                    self.record_packet_error(turn, packet_error_notice);
                    return Ok(());
                }
                self.begin_book_negotiation();
            }
            Event::Transfer(task) => {
                if let Some(packet) = task.into_eligible_packet() {
                    self.handle_listener_transfer(packet, turn)?;
                }
            }
            Event::PackPush(p) => self.handle_pack_push(p),
            Event::PackPop(id) => self.handle_pack_pop(id),
            Event::ServerLinks(links) => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.server_links = crate::game::server_links::trusted(links);
                }
            }
            Event::ShowDialog { dialog, registries } => {
                self.show_dialog_in_phase(&dialog, Some(registries));
            }
            Event::ClearDialog => self.clear_dialog(),
            Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
            Event::DecodeError(e) => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.decode_errors += 1;
                    if r.decode_errors <= 20 {
                        log::warn!("Sunrise: skipped undecodable packet {e}");
                    }
                }
            }
            event @ Event::Disconnected { .. } => {
                turn.end.get_or_insert(event);
            }
            Event::Bundle(bundle) => self.handle_play_bundle(*bundle, turn)?,
            Event::RawPayload(incoming) => {
                if let Some(remote) = self.client.remote.as_ref() {
                    self.run_payload(remote.session.payload_endpoint(), *incoming);
                }
            }
            Event::ObservePacket(packet) => self.observe_client_packet(&packet),
            Event::AttachmentSync(staged) => self.apply_staged_attachment_sync(*staged),
            Event::Packet(packet) => match *packet {
                C::CustomPayload(p) if p.channel == sunrise_sim::perf_net::CHANNEL => {
                    self.remote_perf_reply(&p.data)
                }
                packet => match self.handle_packet(packet) {
                    // ClientCommonPacketListenerImpl.onPacketError: close first and keep any earlier reason.
                    Err(e) if e.is::<PacketError>() => {
                        log::warn!("Sunrise: disconnected: {e}");
                        self.record_packet_error(turn, packet_error_notice);
                    }
                    other => other?,
                },
            },
        }
        Ok(())
    }
    /// `Minecraft.runTick`: exhaust the packet phase before starting client tasks. A
    /// worker-decoded column defers its position, every later packet and the task phase.
    pub fn poll_remote_events(&mut self, before_tick: bool) -> Result<()> {
        let mut receive = !before_tick;
        let mut chunks = Duration::ZERO;
        loop {
            // Capture tasks before checking the packet queue, so a task published after the
            // empty packet boundary cannot overtake a packet that arrived before that task.
            self.client
                .pending_client_tasks
                .0
                .append(&mut self.client.client_tasks.0.take_turn());
            let Some(remote) = self.client.remote.as_mut() else {
                break;
            };
            let mut events = std::mem::take(&mut remote.backlog);
            if receive {
                remote.receive_credited_backlog(&mut events);
            }
            if events.is_empty() {
                remote.backlog_since = None;
                break;
            }
            let overdue = remote
                .backlog_since
                .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
            self.dispatch_packet_turn(events, overdue, before_tick, &mut chunks)?;
            if self
                .client
                .remote
                .as_ref()
                .is_some_and(|remote| !remote.backlog.is_empty())
            {
                return Ok(());
            }
            // A handler may have published more packets or installed another listener.
            receive = true;
        }
        self.send_suggestion_request();
        while self.client.client_tasks.0.is_open()
            && let Some(task) = self.client.pending_client_tasks.0.pop_front()
        {
            task.run(self);
        }
        Ok(())
    }
    #[cfg(feature = "test-support")]
    pub fn test_dispatch_ready_packets(&mut self) -> Result<bool> {
        // Only dispatch ingress at a checkpoint, without running the separate client task phase.
        let remote = self.client.remote.as_mut().expect("owner client connected");
        let mut events = std::mem::take(&mut remote.backlog);
        remote.receive_credited_backlog(&mut events);
        if events.is_empty() {
            remote.backlog_since = None;
            return Ok(true);
        }
        let overdue = remote
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        self.dispatch_packet_turn(events, overdue, false, &mut chunks)?;
        let remote = self.client.remote.as_ref().expect("owner client connected");
        Ok(remote.backlog.is_empty())
    }

    /// `ClientCommonPacketListenerImpl.onPacketError`: close first and keep any earlier reason.
    pub(in crate::game::client_side) fn record_packet_error(
        &self,
        turn: &mut ListenerTurn,
        notice: impl FnOnce() -> Event,
    ) {
        if let Some(remote) = &self.client.remote {
            remote.session.close_after_pending();
        }
        turn.end.get_or_insert_with(notice);
        turn.packet_error = true;
    }
}

impl Remote {
    pub(in crate::game) fn receive_credited_backlog(&mut self, events: &mut VecDeque<Received>) {
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column. Packets inside a delimited bundle stay
        // inline so the entire bundle dispatches atomically in one turn.
        for receipt in self.session.poll() {
            let (event, credit) = receipt.into_parts();
            events.push_back(match event {
                Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                    match self.decoder.submit(self.chunk_format.clone(), p) {
                        None => Received::Chunk(self.chunk_format.clone(), credit),
                        Some(p) => Received::Event(Event::Packet(p), credit),
                    }
                }
                event => Received::Event(event, credit),
            });
        }
    }
}

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;
