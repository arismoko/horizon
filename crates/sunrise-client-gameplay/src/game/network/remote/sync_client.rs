use super::*;
use sunrise_level::profile_lookup::TabPlayer;

/// The live entity a bound sound names; absence means that object was removed.
pub fn entity_sound_state(
    id: i32,
    player: Option<(i32, [f64; 3], bool)>,
    mob: Option<sunrise_game_data::sound_source::SoundEntityState>,
    tracked: &HashMap<i32, crate::game::client_side::remote_client::Tracked>,
    dragon: Option<(i32, [f64; 3])>,
) -> Option<sunrise_game_data::sound_source::SoundEntityState> {
    use sunrise_game_data::sound_source::SoundEntityState;
    if let Some((player_id, position, silent)) = player
        && player_id == id
    {
        return Some(SoundEntityState {
            position,
            silent,
            removed: false,
        });
    }
    let mut state = mob.or_else(|| {
        tracked.get(&id).map(|t| SoundEntityState {
            position: t.position,
            silent: t.silent,
            removed: false,
        })
    })?;
    // The dragon's Mob is metadata storage; only Dragon follows its movement.
    if let Some((dragon_id, position)) = dragon
        && dragon_id == id
    {
        state.position = position;
    }
    Some(state)
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn apply_remote_health(&mut self) {
        let Some((health, food, saturation)) = self.client.remote.as_ref().map(|r| r.health) else {
            return;
        };
        if let Some(p) = self.common.player.as_mut() {
            if (p.health.health() - health).abs() > f32::EPSILON {
                let _ = p.set_health(health.max(0.0), false);
            }
            p.health.food.food = food;
            p.health.food.saturation = saturation;
        }
    }
    /// `Entity.push`'s share of the other entities' ticks: added to the local
    /// player's deltaMovement, unless it lies dead, sleeps or rides.
    pub(in crate::game::client_side) fn apply_remote_push(&mut self, pushed: Vec3) {
        let still = self.client.screen == Screen::Dead
            || self.common.sleeping.is_some()
            || self.common.riding.is_some();
        if let Some(player) = self.common.player.as_mut()
            && !still
        {
            player.velocity = player.velocity.add_vector(pushed);
        }
    }
    pub(in crate::game) fn creative_stacks(&self) -> Vec<Option<ItemStack>> {
        creative_sync_stacks(
            &self.common.inventory,
            &self.common.offhand,
            &self.common.armor,
        )
    }
    /// `LocalPlayer.tickDeath`: the level stops ticking the player at death tick 20,
    /// but the client keeps the player for death-screen rendering until respawn.
    pub(in crate::game::client_side) fn local_player_removed_by_death(&self) -> bool {
        self.client.screen == Screen::Dead && self.client.chat_ui.death_time >= 20
    }
    pub(in crate::game::client_side) fn remote_effect_def(
        &self,
        effect: i32,
    ) -> Option<&'static sunrise_level::brewing::EffectDef> {
        let r = self.client.remote.as_ref()?;
        let name = r.builtin.get("minecraft:mob_effect")?.name(effect)?;
        sunrise_level::brewing::effect(name)
    }
    /// A mob effect's network id as the effect (`brewing::EFFECTS` id).
    pub(in crate::game::client_side) fn remote_effect_id(
        &self,
        effect: i32,
    ) -> Option<&'static str> {
        self.remote_effect_def(effect).map(|def| def.id)
    }
    /// The server's network id of a mob effect (`brewing::EFFECTS` id).
    pub fn remote_effect_network_id(&self, effect: &str) -> Option<i32> {
        let r = self.client.remote.as_ref()?;
        r.builtin
            .get("minecraft:mob_effect")?
            .id(&format!("minecraft:{effect}"))
    }
    /// `handleAddObjective`: add, remove or change an objective.
    pub(in crate::game) fn remote_objective(&mut self, p: play::SetObjective) {
        use sunrise_game_data::scoreboard::{RenderType, tag_to_json};
        let board = &mut self.common.score.board;
        match (p.method, p.display) {
            (1, _) => board.remove_objective(&p.name),
            (method, Some((name, render, format))) => {
                let render = if render == 1 {
                    RenderType::Hearts
                } else {
                    RenderType::Integer
                };
                let format = format.as_ref().map(number_format);
                if method == 0 {
                    board.add_objective(
                        &p.name,
                        "dummy",
                        tag_to_json(&name),
                        render,
                        false,
                        format,
                    );
                } else if let Some(o) = board.objective_mut(&p.name) {
                    o.display_name = tag_to_json(&name);
                    o.render_type = render;
                    o.number_format = format;
                }
            }
            _ => {}
        }
    }
    /// The local player's share of every touching entity's
    /// `LivingEntity.pushEntities`, and of the boats' pushes: on a client
    /// `EntitySelector.pushableBy` lets entities push only the local player,
    /// so the server never sends these pushes. `doPush(player)` is
    /// `player.push(entity)`. `before_player`: the mobs and boats that tick
    /// before the player (`ticked_before_player`), else those that tick after
    /// it.
    pub fn remote_pushes(&self, before_player: bool) -> Vec3 {
        let (Some(p), Some(r)) = (self.common.player.as_ref(), self.client.remote.as_ref()) else {
            return Vec3::ZERO;
        };
        // LivingEntity.isPushable: alive, no spectator, off ladders.
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if self.client.screen == Screen::Dead
            || p.spectator
            || self.named_block_in_tag("minecraft:climbable", &self.block_name(feet))
        {
            return Vec3::ZERO;
        }
        let bounds = p.bounds();
        // `Entity.getTeam` is `Scoreboard.getPlayersTeam(getScoreboardName())`:
        // a player's score holder is its profile name, every other entity's
        // its UUID text.
        let board = &self.common.score.board;
        let player_team = board.players_team(&r.profile_name);
        let mut push = Vec3::ZERO;
        let order = |id: u32| r.ticked_before_player.contains(&id) == before_player;
        for m in self.common.mobs.iter().filter(|m| order(m.id)) {
            // Bats never push, nor passengers of one vehicle, nor a spectator; the
            // box test comes before the lookups that far mobs never need (#14053).
            if !m.alive()
                || m.species() == sunrise_sim::mobs::Species::Bat
                || self
                    .common
                    .riding
                    .is_some_and(|v| v == m.id || m.extra.mount == Some(v))
                || !m.bounds().intersects(bounds)
            {
                continue;
            }
            let listed = r
                .level_players
                .get(&(m.id as i32))
                .and_then(|uuid| r.tab_players.get(uuid));
            let mob_team = listed.map_or_else(
                || self.client_mob_team_of(m),
                |t| board.players_team(&t.name),
            );
            if r.remote_player_info_game_mode(m.id as i32) == Some(SPECTATOR)
                || !sunrise_game_data::scoreboard::pushable_by(mob_team, player_team, true)
            {
                continue;
            }
            // Entity.push(Entity), `this` the player.
            let (mut xa, mut za) = (m.position.x - p.position.x, m.position.z - p.position.z);
            let mut dd = xa.abs().max(za.abs());
            if dd < f64::from(0.01_f32) {
                continue;
            }
            dd = dd.sqrt();
            xa /= dd;
            za /= dd;
            let pow = (1.0 / dd).min(1.0);
            xa *= pow;
            za *= pow;
            xa *= f64::from(0.05_f32);
            za *= f64::from(0.05_f32);
            push = Vec3::new(push.x - xa, push.y, push.z - za);
        }
        // AbstractBoat.push(player) from the boat's own tick (its box grown
        // by 0.2 sideways) and from the player's pushEntities (its box).
        let area = Vec3::new(f64::from(0.2_f32), f64::from(-0.01_f32), f64::from(0.2_f32));
        // A boat pushes from its own tick, in the same order as the mobs.
        for b in self
            .common
            .boats
            .iter()
            .filter(|b| order(b.id) && self.common.riding != Some(b.id))
        {
            let bb = b.bounds();
            let times = usize::from(
                bounds.intersects(bb.inflate(area))
                    && sunrise_game_data::scoreboard::pushable_by(None, player_team, true),
            ) + usize::from(
                bounds.intersects(bb)
                    && sunrise_game_data::scoreboard::pushable_by(player_team, None, true),
            );
            if times > 0
                && sunrise_gameplay_common::game::boats_common::boat_pushes(bb, bounds, false)
                && let Some(v) = sunrise_gameplay_common::game::boats_common::push_impulse(
                    b.position, p.position,
                )
            {
                push = push.add_vector(v.scale(times as f64));
            }
        }
        push
    }
    /// `MultiPlayerGameMode.ensureHasSentCarriedItem`: the server hears of a
    /// new selected slot before anything the slot acts with.
    pub fn remote_send_carried(&self) {
        if let Some(r) = &self.client.remote
            && r.carried.get() != Some(self.common.selected)
        {
            r.carried.set(Some(self.common.selected));
            self.send_play(S::SetCarriedItem(play::SetCarriedItem {
                slot: self.common.selected as i16,
            }));
        }
    }
    /// The end of the player's tick: `AbstractBoat.tick`'s paddles (the vehicle ticks before its
    /// passenger's `sendChanges`), then `LocalPlayer.sendChanges`: the input, and a passenger's look,
    /// vehicle and sprint, or `sendPosition`'s sprint and move. The held slot is not sent here:
    /// `MultiPlayerGameMode.tick` sends it at the start of the next tick (`remote_handle_input`).
    pub fn remote_send_state(&mut self, input: u8) {
        // Minecraft.tick's sendChanges also skips the tick that removed the player.
        if self.local_player_removed_by_death() {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let (pos, on_ground, collision, sprinting) = (
            [p.position.x, p.position.y, p.position.z],
            p.ground.on_ground,
            p.collision.horizontal,
            p.sprinting,
        );
        let (yaw, pitch) = (self.common.yaw, self.common.pitch);
        let riding = self.common.riding.is_some();
        let controls_camera = self.controls_camera();
        let (paddles, vehicle): (Vec<S>, Vec<S>) = self
            .remote_vehicle_packets()
            .into_iter()
            .partition(|p| matches!(p, S::PaddleBoat(_)));
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let mut out = paddles;
        if input != r.last_input {
            r.last_input = input;
            out.push(S::PlayerInput(play::PlayerInput { flags: input }));
        }
        // LocalPlayer.sendIsSprintingIfNeeded.
        let sprint = |r: &mut Remote, out: &mut Vec<S>| {
            if sprinting != r.was_sprinting {
                r.was_sprinting = sprinting;
                out.push(S::PlayerCommand(play::PlayerCommand {
                    id: r.entity_id,
                    action: if sprinting { 1 } else { 2 },
                    data: 0,
                }));
            }
        };
        let mut moved_or_rotated = false;
        if riding {
            // LocalPlayer.sendChanges: a passenger sends its look, then the
            // vehicle it controls and, only then, its sprint.
            out.push(S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch,
                flags: play::pack_move_player_flags(on_ground, collision),
            }));
            if !vehicle.is_empty() {
                out.extend(vehicle);
                sprint(r, &mut out);
            }
            moved_or_rotated = true;
        } else {
            sprint(r, &mut out);
            // LocalPlayer.sendPosition: no move while the view is another
            // entity's (the server keeps the player on it).
            if controls_camera
                && let Some(packet) =
                    r.position_sender
                        .update(pos, (yaw, pitch), on_ground, collision)
            {
                // A server that sent colliders reads how old the copy carrying the player is.
                if !r.colliders.is_empty()
                    && let Some(hint) = r.carry_age.hint()
                {
                    out.push(S::CustomPayload(
                        sunrise_protocol::packets::configuration::CustomPayload {
                            channel: sunrise_protocol::carry_age::CHANNEL.into(),
                            data: sunrise_protocol::carry_age::encode(hint),
                        },
                    ));
                }
                out.push(packet);
                moved_or_rotated = true;
            }
        }
        if moved_or_rotated && r.session.is_recording() {
            let pkt = sunrise_protocol::packets::play::Clientbound::PlayerPosition(
                sunrise_protocol::packets::play::PlayerPosition {
                    teleport: 0,
                    position: pos,
                    delta: [0.0, 0.0, 0.0],
                    yaw,
                    pitch,
                    relatives: 0,
                },
            );
            r.session.record_clientbound(&pkt);
        }
        if r.session.is_recording() {
            let is_using = self.common.using.is_some();
            let living_flags: u8 = if is_using {
                1 | if self.common.hand == Hand::Off { 2 } else { 0 }
            } else {
                0
            };
            let crouching =
                p.local_crouching || matches!(p.pose, sunrise_sim::pose::Pose::Crouching);
            let mut shared: u8 = 0;
            if crouching {
                shared |= 0x02;
            }
            if sprinting {
                shared |= 0x08;
            }
            if p.swimming {
                shared |= 0x10;
            }
            if p.fall_flying {
                shared |= 0x80;
            }
            if shared != r.was_shared_flags
                || living_flags != r.was_living_flags
                || r.last_recorded_pose != Some(p.pose)
            {
                r.was_shared_flags = shared;
                r.was_living_flags = living_flags;
                r.last_recorded_pose = Some(p.pose);
                use sunrise_protocol::entity_data::{Value, serializer};
                let pkt = sunrise_protocol::packets::play::Clientbound::SetEntityData(
                    sunrise_protocol::packets::play::SetEntityData {
                        id: r.entity_id,
                        values: vec![
                            (0, serializer::BYTE, Value::Byte(shared as i8)),
                            (6, serializer::POSE, Value::Id(p.pose.id())),
                            (8, serializer::BYTE, Value::Byte(living_flags as i8)),
                        ],
                    },
                );
                r.session.record_clientbound(&pkt);
            }
        }
        // Creative inventory edits are sent slot by slot (SetCreativeModeSlot).
        let mut creative_slots = Vec::new();
        if self.common.creative {
            // A server's open menu takes clicks, which it applies itself: what
            // this client predicts for them isn't an edit to send (it would
            // overwrite the server's stacks whenever the prediction is wrong).
            let in_menu = r.menu.as_ref().is_some();
            let stacks = creative_sync_stacks(
                &self.common.inventory,
                &self.common.offhand,
                &self.common.armor,
            );
            for (i, stack) in stacks.into_iter().enumerate() {
                if stack != r.synced[i] {
                    r.synced[i] = stack.clone();
                    if !in_menu {
                        creative_slots.push((creative_sync_slot(i), stack));
                    }
                }
            }
        }
        for packet in out {
            self.send(packet);
        }
        for (slot, stack) in creative_slots {
            self.send_creative_item_add(slot, stack);
        }
    }
    pub(in crate::game::client_side) fn remote_sound_entity_state(
        &mut self,
        id: i32,
    ) -> Option<sunrise_game_data::sound_source::SoundEntityState> {
        let r = self.client.remote.as_ref()?;
        let player = self.common.player.as_ref().map(|p| {
            (
                r.entity_id,
                [p.position.x, p.position.y, p.position.z],
                r.local_entity_silent,
            )
        });
        // The local player is not in mobs, so avoid a guaranteed index miss.
        if player.is_some_and(|p| p.0 == id) {
            return entity_sound_state(id, player, None, &r.entities, None);
        }
        let mob = self
            .common
            .mob_index
            .get(&self.common.mobs, id as u32)
            .map(|m| sunrise_game_data::sound_source::SoundEntityState {
                position: [m.position.x, m.position.y, m.position.z],
                silent: m.silent,
                removed: m.removed,
            });
        let dragon = self
            .common
            .dragon
            .as_ref()
            .map(|d| (d.id as i32, [d.position.x, d.position.y, d.position.z]));
        let mut state = entity_sound_state(id, player, mob, &r.entities, dragon)?;
        // Mob states already use the live position. In particular,
        // level_entity would find the dragon's stale Mob before its Dragon.
        if mob.is_some() || dragon.is_some_and(|d| d.0 == id) {
            return Some(state);
        }
        // Vehicles, projectiles and other simulated objects may have moved
        // since the last tracking packet.
        if let Some(position) = self.remote_simulated_entity_position(id) {
            state.position = position;
        }
        Some(state)
    }
    /// A `Holder<SoundEvent>` off the wire: an inline event's location, or
    /// the name of a registered one.
    pub(in crate::game) fn remote_sound_name(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        match &sound.1 {
            Some((location, _)) => Some(location.clone()),
            None => self.client.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:sound_event")?
                    .name(sound.0)
                    .map(str::to_owned)
            }),
        }
    }
    /// `handleSetPlayerTeamPacket`.
    pub(in crate::game) fn remote_team(&mut self, p: play::SetPlayerTeam) {
        use sunrise_game_data::scoreboard::{CollisionRule, Visibility, tag_to_json};
        let board = &mut self.common.score.board;
        let team = if p.method == 0 {
            board.add_team(&p.name)
        } else if let Some(team) = board.team_mut(&p.name) {
            team
        } else {
            log::warn!(
                "Received team packet with method {} for unknown team {:?}",
                p.method,
                p.name
            );
            return;
        };
        if p.method == 1 {
            board.remove_team(&p.name);
            return;
        }
        if let Some(params) = &p.parameters {
            team.display_name = tag_to_json(&params.display_name);
            team.prefix = tag_to_json(&params.prefix);
            team.suffix = tag_to_json(&params.suffix);
            team.color = params
                .color
                .and_then(|c| sunrise_game_data::text::COLORS.get(c as usize))
                .map(|c| c.1.to_owned());
            team.allow_friendly_fire = params.options & 1 != 0;
            team.see_friendly_invisibles = params.options & 2 != 0;
            team.nametag_visibility = Visibility::ALL
                .get(params.name_tag_visibility as usize)
                .copied()
                .unwrap_or(Visibility::Always);
            team.collision_rule = CollisionRule::ALL
                .get(params.collision_rule as usize)
                .copied()
                .unwrap_or(CollisionRule::Always);
        }
        match p.method {
            0 | 3 => {
                for player in &p.players {
                    board.add_to_team(player, &p.name);
                }
            }
            // Scoreboard.removePlayerFromTeam: only out of this team (vanilla
            // throws for a player in another one).
            4 => {
                for player in &p.players {
                    if board.players_team(player).is_some_and(|t| t.name == p.name) {
                        board.remove_from_team(player);
                    }
                }
            }
            _ => {}
        }
    }
    /// ClientPacketListener.handleMovePlayer.
    pub fn remote_teleport(&mut self, p: &play::PlayerPosition) -> Result<()> {
        use play::relative as rel;
        let (had_player, current) = self
            .common
            .player
            .as_ref()
            .map(|pl| (true, pl.position))
            .unwrap_or((false, Vec3::ZERO));
        let respawn_velocity = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.respawn_velocity.take());
        let velocity = self
            .common
            .player
            .as_ref()
            .map(|pl| pl.velocity)
            .or(respawn_velocity)
            .unwrap_or(Vec3::ZERO);
        let pick = |bit: i32, base: f64, v: f64| if p.relatives & bit != 0 { base + v } else { v };
        let position = Vec3::new(
            pick(rel::X, current.x, p.position[0]),
            pick(rel::Y, current.y, p.position[1]),
            pick(rel::Z, current.z, p.position[2]),
        );
        let yaw = if p.relatives & rel::Y_ROT != 0 {
            self.common.yaw + p.yaw
        } else {
            p.yaw
        };
        let relative_pitch = p.relatives & rel::X_ROT != 0;
        let pitch = position_packet_pitch(self.common.pitch, p.pitch, relative_pitch);
        let delta = sunrise_client::player_sync::teleport_velocity(
            velocity,
            (self.common.yaw, self.common.pitch),
            (yaw, pitch),
            p,
        );
        // BlockStatePredictionHandler.onTeleport: a prior prediction keeps its block, not its remembered player position.
        if let Some(r) = self.client.remote.as_mut() {
            r.last_teleport_sequence = r.sequence;
        }
        // The server's answer to a passage crossed (`cross_passage`), or any other.
        self.common.passages.crossing = None;
        // handleMovePlayer: a passenger keeps its own position and look (its vehicle
        // carries it), so a vehicle a datapack teleports every tick doesn't snap the
        // rider's view; the teleport is still accepted, with where the player is.
        if self.common.riding.is_some()
            && let Some(player) = self.common.player.as_ref()
        {
            let at = player.position;
            self.send(S::AcceptTeleportation(play::AcceptTeleportation {
                id: p.teleport,
                x: at.x,
                y: at.y,
                z: at.z,
                yaw: self.common.yaw,
                pitch: self.common.pitch,
            }));
            self.stop_remote_mining();
            self.set_client_mining(None);
            return Ok(());
        }
        match self.common.player.as_mut() {
            Some(player) => {
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.common.motion, position)?;
                // The id the server gave, for the hooks the motion fires.
                player.entity_id = self.own_entity_id();
                // Entity.noPhysics starts false until Player.tick() resolves PlayerInfo;
                // LocalPlayer.aiStep reads MultiPlayerGameMode.isSpectator().
                player.local_spectator = self.common.spectator();
                player.velocity = delta;
                self.take_carried_attributes(&mut player)?;
                self.common.hook.discard();
                self.common.player_state.attachments.retire();
                self.common.player = Some(player);
                self.bind_local_player_residency();
                self.input_player_replaced();
                self.client.sway = Default::default();
                self.common.eye = 1.62;
                self.client.previous_eye = 1.62;
                self.common.clock.reset();
                self.apply_remote_health();
                self.apply_remote_abilities();
            }
        }
        entity_section_steps::player_teleport(self, had_player, current, position);
        if !self.client.replay.playing() {
            self.common.previous_position = position;
        }
        self.common.yaw = yaw;
        self.common.pitch = pitch;
        // setValuesFromPositionPacket: setOldPosAndRot gives xRotO the same
        // change, so a relative pitch keeps the model's ease and an absolute
        // one snaps it.
        if let Some(m) = self.client.player_model.as_mut() {
            m.previous_head_pitch =
                position_packet_pitch(m.previous_head_pitch, p.pitch, relative_pitch);
            m.head_pitch = pitch;
        }
        self.ensure_player_model();
        if let Some(id) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.ghast_dismount.take())
        {
            self.place_ghast_under(id, position);
        }
        // handleMovePlayer does not change LocalPlayer.sendPosition's baseline:
        // the next tick must still report the corrected state.
        self.send(S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
        }));
        // Then MultiPlayerGameMode.stopDestroyBlock: a dig the teleport (an
        // anticheat's setback, a /tp) interrupts is aborted on the wire, and the
        // attack strength starts again.
        self.stop_remote_mining();
        self.set_client_mining(None);
        Ok(())
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemAdd`, shared by each creative slot sender.
    pub(in crate::game) fn send_creative_item_add(&self, slot: i16, stack: Option<ItemStack>) {
        if !self.common.creative || self.client.remote.is_none() {
            return;
        }
        let stack = stack.filter(|stack| !stack.is_empty());
        // Vanilla checks the item's required features, and an empty stack's item is air.
        let air;
        let enabled = match &stack {
            Some(stack) => self
                .client
                .creative_screen
                .0
                .catalog()
                .item_enabled(stack, self.enabled_feature_ids()),
            None => {
                air = ItemStack::new("minecraft:air", 0);
                self.client
                    .creative_screen
                    .0
                    .catalog()
                    .item_enabled(&air, self.enabled_feature_ids())
            }
        };
        if !enabled {
            return;
        }
        self.send_play(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot,
            item: self.to_network(&stack),
        }));
    }
    /// `CreativeInventoryListener.slotChanged`: `handleCreativeModeItemAdd` for every slot the
    /// player's inventory menu changed. `CreativeModeInventoryScreen.slotClicked` ends in
    /// `broadcastChanges`, so a click's slots go out with the click, right behind a thrown
    /// stack, and not at the end of the tick: a slot update the server sends meanwhile (the
    /// pickup of what a held Q threw) can then never land on a change still unsent and undo it.
    pub fn send_creative_slots(&mut self) {
        if !self.common.creative {
            return;
        }
        let stacks = self.creative_stacks();
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        for (i, stack) in stacks.into_iter().enumerate() {
            if stack != r.synced[i] {
                r.synced[i] = stack.clone();
                changed.push((creative_sync_slot(i), stack));
            }
        }
        for (slot, stack) in changed {
            self.send_creative_item_add(slot, stack);
        }
    }

    pub fn remote_tab_players(&mut self) -> Option<(Vec<TabPlayer>, bool, Vec<u128>)> {
        self.update_remote_skins();
        self.publish_tab_object_skins();
        let r = self.client.remote.as_ref()?;
        let players: Vec<_> = r
            .listed
            .iter()
            .filter_map(|u| {
                let mut player = r.tab_players.get(u)?.clone();
                player.skin = r.player_info_skin(*u).to_owned();
                Some(player)
            })
            .collect();
        let upside_down = players
            .iter()
            .filter(|p| {
                (p.name == "Dinnerbone" || p.name == "Grumm")
                    && (p.uuid == r.profile || r.level_players.values().any(|u| *u == p.uuid))
            })
            .map(|p| p.uuid)
            .collect();
        Some((players, r.online_mode, upside_down))
    }
}

impl Remote {
    /// `ClientPacketListener.getPlayerInfo(String)`: exact profile name, whether listed or not.
    pub(in crate::game) fn player_info_by_name(&self, name: &str) -> Option<&TabPlayer> {
        self.tab_players.values().find(|p| p.name == name)
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
