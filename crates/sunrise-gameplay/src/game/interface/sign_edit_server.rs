use super::commands::{Target, execute::Source};
use sunrise_protocol::packets::play::{self, Clientbound as C};
use {
    sunrise_items::items::ItemStack, sunrise_level::containers::BlockEntity,
    sunrise_level::signs_data::ClickAction, sunrise_level::signs_data::DYE_TEXT,
    sunrise_level::signs_data::click_action,
};

impl crate::game::ServerGame<'_> {
    /// `WallHangingSignBlock.shouldTryToChainAnotherHangingSign` and
    /// `CeilingHangingSignBlock.shouldTryToChainAnotherHangingSign`: a hanging
    /// sign item clicked on a hanging sign's free side chains another sign onto
    /// it, unless the side the player faces carries click commands the click
    /// runs instead (`canExecuteClickCommands`).
    pub(in crate::game) fn chains_hanging_sign(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        let hanging_sign = held.is_some_and(|s| {
            self.common
                .res
                .behaviours()
                .item_binding(&s.item)
                .is_some_and(|i| i.kind == sunrise_world::level::ItemKind::HangingSign)
        });
        let key = (pos[0], pos[1], pos[2]);
        if !hanging_sign {
            return false;
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let Some(form) = self
            .common
            .res
            .facts()
            .get(state)
            .sign_form()
            .filter(|f| f.hanging)
        else {
            return false;
        };
        let free_side = if form.wall {
            let Some(facing) = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing"))
            else {
                return false;
            };
            let axis = match &**facing {
                "east" | "west" => 0,
                "up" | "down" => 1,
                _ => 2,
            };
            face.iter().position(|c| *c != 0) != Some(axis)
        } else {
            face == [0, -1, 0]
        };
        free_side && !self.sign_can_execute_click_commands(key)
    }
    /// `SignBlockEntity.getSlotPlayerIsFacing`: whether the player faces the sign's front side.
    pub(in crate::game::server_side) fn facing_sign_front(&self, pos: (i32, i32, i32)) -> bool {
        let Some(player) = self.common.player.as_ref() else {
            return true;
        };
        let id = self.common.world.block(pos.0, pos.1, pos.2);
        let Some(state) = self.common.pack.assets.registry.state(id) else {
            return true;
        };
        let form = self.common.res.facts().get(id).sign_form();
        let prop = |key: &str| state.properties.get(key).map(String::as_str);
        // getYRotationDegrees: wall sign facing or standing/ceiling sign rotation segment.
        let y_rot = if form.is_some_and(|f| f.wall) {
            match prop("facing") {
                Some("west") => 90.0,
                Some("north") => 180.0,
                Some("east") => 270.0,
                _ => 0.0,
            }
        } else {
            prop("rotation")
                .and_then(|r| r.parse::<f32>().ok())
                .unwrap_or(0.0)
                * 22.5
        };
        // getSignHitboxCenterPosition: only a wall sign's board is off-centre against its wall.
        let centre = if form.is_some_and(|f| f.wall && !f.hanging) {
            match y_rot as i32 {
                90 => (0.9375, 0.5),
                180 => (0.5, 0.9375),
                270 => (0.0625, 0.5),
                _ => (0.5, 0.0625),
            }
        } else {
            (0.5, 0.5)
        };
        let xd = player.position.x - (f64::from(pos.0) + centre.0);
        let zd = player.position.z - (f64::from(pos.2) + centre.1);
        let player_y_rot = (zd.atan2(xd).to_degrees() - 90.0) as f32;
        // Mth.degreesDifferenceAbs.
        let diff = (player_y_rot - y_rot).rem_euclid(360.0);
        let diff = if diff >= 180.0 { 360.0 - diff } else { diff };
        diff <= 90.0
    }
    /// `SignBlockEntity.canExecuteClickCommands` for the side the acting player faces.
    pub(in crate::game) fn sign_can_execute_click_commands(&self, pos: (i32, i32, i32)) -> bool {
        // Only a waxed sign can; skip the facing maths for everything else.
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&pos) else {
            return false;
        };
        sign.waxed && sign.can_execute_click_commands(self.facing_sign_front(pos))
    }
    /// `SignBlockEntity.createCommandSourceStack`: silent game-master source at sign centre.
    pub(in crate::game::server_side) fn sign_command_source(&self, pos: [i32; 3]) -> Source {
        Source {
            entity: self.common.player.is_some().then_some(Target::Player),
            pos: pos.map(|v| f64::from(v) + 0.5),
            rot: [0.0, 0.0],
            eyes: false,
            silent: true,
            store: None,
            dimension: self.common.dim,
            origin_dimension: self.common.dim,
            origin_player: self.common.player.is_some(),
            native_output: None,
            level: None,
            block: None,
            cart: None,
        }
    }
    /// `SignBlockEntity.executeClickCommandsIfPresent`: runs click events on the faced side.
    pub(in crate::game::server_side) fn execute_sign_click_commands(
        &mut self,
        pos: [i32; 3],
        front: bool,
    ) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&key) else {
            return false;
        };
        let allow_op_features = sign.allow_op_features;
        let side = if front { &sign.front } else { &sign.back };
        // The side as it was when the loop began: a command may change it.
        let events: Vec<_> = side.click.iter().flatten().cloned().collect();
        let mut any_click_command = false;
        for event in &events {
            match click_action(event) {
                Some(ClickAction::RunCommand(command)) => {
                    if allow_op_features {
                        self.run_sign_command(pos, &command);
                    }
                    any_click_command = true;
                }
                Some(ClickAction::ShowDialog(dialog)) => {
                    if allow_op_features {
                        self.show_click_dialog(dialog);
                    }
                    any_click_command = true;
                }
                // MinecraftServer.handleCustomClickAction only logs it.
                Some(ClickAction::Custom { id, payload }) => {
                    if allow_op_features {
                        log::debug!("Received custom click action {id} with payload {payload:?}");
                    }
                    any_click_command = true;
                }
                None => {}
            }
        }
        if !allow_op_features && any_click_command {
            self.sign_click_actions_disabled();
        }
        any_click_command
    }
    /// `SignBlock.useWithoutItem`: runs click commands, fails on waxed signs, or opens the editor.
    pub(in crate::game) fn use_sign_empty_hand(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        if !matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sign(_))
        ) {
            return;
        }
        let front = self.facing_sign_front(key);
        let executed = self.execute_sign_click_commands(pos, front);
        // The commands may have changed the sign, or removed it.
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&key) else {
            return;
        };
        if sign.waxed {
            // `getSignInteractionFailedSoundEvent`: a hanging sign has its own.
            let sound = if sign.hanging {
                "minecraft:block.hanging_sign.waxed_interact_fail"
            } else {
                "minecraft:block.sign.waxed_interact_fail"
            };
            self.server_play_sound(sound, pos.map(|v| f64::from(v) + 0.5), 1.0, 1.0);
        } else if !executed
            && !self.other_player_is_editing_sign(key)
            && self.may_build()
            && self.sign_side_has_editable_text(key, front)
        {
            self.server_request_sign_editor(key, front);
        }
    }
    /// Whether `SignBlock.useWithoutItem` on the sign at `pos` consumes the click.
    pub(in crate::game::server_side) fn sign_consumes_click(&self, pos: [i32; 3]) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&key) else {
            return false;
        };
        if sign.waxed || self.may_build() {
            return true;
        }
        let side = if self.facing_sign_front(key) {
            &sign.front
        } else {
            &sign.back
        };
        side.click
            .iter()
            .flatten()
            .any(|event| click_action(event).is_some())
    }
    /// `SignBlock.useItemOn` with a `SignApplicator` in hand (dyes, ink sacs, honeycomb).
    pub(in crate::game::server_side) fn use_sign_applicator(
        &mut self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        let Some(stack) = held else {
            return false;
        };
        let key = (pos[0], pos[1], pos[2]);
        let waxed = match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Sign(sign)) => sign.waxed,
            _ => return false,
        };
        if waxed || !self.may_build() {
            return false;
        }
        let item = &*stack.item;
        let dye = if item.ends_with("_dye") {
            self.common
                .res
                .items()
                .get(&stack.item)
                .and_then(|i| i.dye.clone())
        } else {
            None
        };
        let colour = dye.and_then(|d| DYE_TEXT.iter().position(|(n, _)| *n == d));
        if !matches!(
            item,
            "minecraft:honeycomb" | "minecraft:glow_ink_sac" | "minecraft:ink_sac"
        ) && colour.is_none()
        {
            return false;
        }
        // `SignBlock.useItemOn`: a sign another player is editing takes no applicator.
        if self.other_player_is_editing_sign(key) {
            return false;
        }
        let front = self.facing_sign_front(key);
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get_mut(&key) else {
            return false;
        };
        let centre = pos.map(|v| f64::from(v) + 0.5);
        let sound = if item == "minecraft:honeycomb" {
            // HoneycombItem.tryApplyToSign: waxes any sign, empty or not.
            sign.waxed = true;
            "minecraft:item.honeycomb.wax_on"
        } else {
            let side = if front {
                &mut sign.front
            } else {
                &mut sign.back
            };
            // SignApplicator.canApplyToSign: the side must have text.
            if side.is_empty() {
                return false;
            }
            match colour {
                Some(c) if side.color != c as u8 => {
                    side.color = c as u8;
                    "minecraft:item.dye.use"
                }
                None if item == "minecraft:glow_ink_sac" && !side.glowing => {
                    side.glowing = true;
                    "minecraft:item.glow_ink_sac.use"
                }
                None if item == "minecraft:ink_sac" && side.glowing => {
                    side.glowing = false;
                    "minecraft:item.ink_sac.use"
                }
                _ => return false,
            }
        };
        if item == "minecraft:honeycomb" {
            self.server_level_event_at(3003, pos, 0);
        }
        self.server_play_sound(sound, centre, 1.0, 1.0);
        self.mark_block_entity_updated(key);
        self.execute_sign_click_commands(pos, front);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        self.post_block_game_event(
            "minecraft:block_change",
            pos,
            self.sculk_actor(),
            Some(state),
        );
        self.award_stat("used", &stack.item, 1);
        self.consume_held();
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        true
    }
    /// `ServerPlayer.openTextEdit` for the guest whose action opened a sign:
    /// the block, then the editor, on its client, and the guest may write on it.
    pub(in crate::game::server_side) fn lan_open_sign_editor(
        &mut self,
        conn: u32,
        edit: crate::game::server_side::acting_state::PendingSignEditor,
    ) {
        assert!(
            self.is_logical_server(),
            "lan_open_sign_editor called on Client role"
        );
        let pos = [edit.pos.0, edit.pos.1, edit.pos.2];
        let Some(guest_uuid) = self.server.lan.as_ref().and_then(|host| {
            let g = host.hub.guest(conn)?;
            (g.placed && g.participates_online() && g.within_block_range(pos, 4.0))
                .then_some(g.uuid)
        }) else {
            return;
        };
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&edit.pos) else {
            return;
        };
        if sign.waxed
            || sign
                .allowed_editor
                .is_some_and(|holder| holder != guest_uuid)
        {
            return;
        }
        self.common
            .world
            .block_entities
            .set_sign_editor(edit.pos, Some(guest_uuid));
        if let Some(host) = self.server.lan.as_mut() {
            host.hub
                .send_authoritative_block(conn, &self.common.world, pos);
            host.hub.send_to(
                conn,
                &C::OpenSignEditor(play::OpenSignEditor {
                    pos,
                    front: edit.front,
                }),
            );
        }
    }
    /// `ServerGamePacketListenerImpl.handleSignUpdate`: the lines, formatting
    /// codes stripped, go on the sign the guest may write on unless it is
    /// waxed (`SignBlockEntity.updateSignText`); a guest gone out of reach
    /// may no longer write (`playerIsTooFarAwayToEdit`).
    pub(in crate::game::server_side) fn lan_sign_update(
        &mut self,
        conn: u32,
        mut p: play::SignUpdate,
    ) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (uuid, name) = (g.uuid, g.name.clone());
        let key = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&key) else {
            return;
        };
        if sign.waxed || sign.allowed_editor != Some(uuid) {
            log::warn!("{name} just tried to change non-editable sign");
            return;
        }
        // Everything vanilla checks has passed: the mods hear what is about to
        // go on the sign, with the player's state swapped in, and may rewrite
        // or refuse it.
        if let Some((uuid, name, dimension)) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone(), g.dimension.clone()))
        {
            let mut lines = p.lines.to_vec();
            let mut cancel = false;
            self.as_guest_messaged(conn, |game| {
                game.native_sign_changed(mod_api::__private::SignChangedCall {
                    player: uuid,
                    name: &name,
                    dimension: &dimension,
                    pos: p.pos,
                    front: p.front,
                    lines: &mut lines,
                    cancel: &mut cancel,
                });
            });
            if cancel {
                // The sign keeps its text: the editor closes, and the player's
                // client, which shows what it sent, gets the sign as it is.
                self.common.world.block_entities.set_sign_editor(key, None);
                self.mark_block_entity_updated(key);
                return;
            }
            lines.resize(4, String::new());
            for (line, new) in p.lines.iter_mut().zip(lines) {
                *line = new;
            }
        }
        if self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.play.get(&conn))
            .is_none()
        {
            return;
        }
        // A hook may have broken the sign.
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get_mut(&key) else {
            return;
        };
        // A hook may have broken the sign.
        if sign.waxed {
            self.common.world.block_entities.set_sign_editor(key, None);
            return;
        }
        let side = if p.front {
            &mut sign.front
        } else {
            &mut sign.back
        };
        for (line, new) in side.lines.iter_mut().zip(&p.lines) {
            *line = sunrise_game_data::text::strip_formatting(new);
        }
        self.common.world.block_entities.set_sign_editor(key, None);
        self.mark_block_entity_updated(key);
    }
    #[inline(always)]
    pub(in crate::game) fn open_sign_editor_server(&mut self, pos: (i32, i32, i32), front: bool) {
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_open_sign(player, &name, [pos.0, pos.1, pos.2], front);
        }
    }
    /// `SignBlock.otherPlayerIsEditingSign`: the sign at `key` belongs to
    /// someone else's editor right now.
    pub(in crate::game::server_side) fn other_player_is_editing_sign(
        &mut self,
        key: (i32, i32, i32),
    ) -> bool {
        match self.common.world.block_entities.sign_editor(&key) {
            Some(holder) => holder != self.sign_editor_uuid(),
            None => false,
        }
    }
    /// Run a sign's `run_command` line as the prefixed command vanilla runs
    /// it as: a leading slash is allowed, errors are not reported (the NULL
    /// source).
    pub(in crate::game::server_side) fn run_sign_command(&mut self, pos: [i32; 3], command: &str) {
        let source = self.sign_command_source(pos);
        let line = command.strip_prefix('/').unwrap_or(command);
        self.server.exec.0 = Default::default();
        let permission = std::mem::replace(&mut self.common.ui.registries.permission, 2);
        let _ = self.execute_line(line, source);
        self.common.ui.registries.permission = permission;
        // InventoryChangeTrigger fires as slots change, not on the next tick.
        if self.common.player.is_some() {
            self.check_inventory_trigger();
        }
    }
    /// `Player.sendOverlayMessage(CLICK_ACTIONS_DISABLED)`: red text on the
    /// action bar. A guest gets a packet, so its own client translates it.
    pub(in crate::game::server_side) fn sign_click_actions_disabled(&mut self) {
        self.server_send_overlay_message(
            sunrise_game_data::text::Text::translate("sign.click_actions_disabled", &[])
                .with_color(sunrise_game_data::text::RED),
        );
    }
    /// `SignBlockEntity.playerIsTooFarAwayToEdit`: a player who left is no
    /// player at all (`Level.getPlayerByUUID` finds nobody).
    pub(in crate::game::server_side) fn sign_editor_in_range(
        &self,
        who: u128,
        pos: (i32, i32, i32),
    ) -> bool {
        let at = [pos.0, pos.1, pos.2];
        let me = self
            .common
            .level
            .player_uuid
            .map(sunrise_sim::lan::playerdata::ints_uuid);
        if me == Some(who) && self.common.player.is_some() {
            let within = self.common.eye_position().is_some_and(|eye| {
                sunrise_sim::lan::guest::within_block_range(eye, self.common.reach() + 4.0, at)
            });
            return self.host_within_interaction_range(at, within);
        }
        self.server.lan.as_ref().is_some_and(|host| {
            host.hub.guests.iter().any(|g| {
                g.uuid == who && g.within_interaction_range(&self.common.loop_context, at, 4.0)
            })
        })
    }
    /// `Player.getUUID` of the acting player: the one a sign remembers as its
    /// editor.
    pub(in crate::game::server_side) fn sign_editor_uuid(&mut self) -> u128 {
        sunrise_sim::lan::playerdata::ints_uuid(self.acting_player_uuid())
    }
    /// `SignText.hasEditableText` of the side the player faces: if every line is
    /// plain text, the editor rewrites nothing a player would miss.
    pub(in crate::game::server_side) fn sign_side_has_editable_text(
        &self,
        key: (i32, i32, i32),
        front: bool,
    ) -> bool {
        match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Sign(sign)) => {
                let side = if front { &sign.front } else { &sign.back };
                side.has_editable_text()
            }
            _ => false,
        }
    }
    /// `SignBlockEntity.tick` (`clearInvalidPlayerWhoMayEdit`): a sign keeps its
    /// editor only while that player is here and within the block interaction
    /// range plus four blocks of it.
    pub(in crate::game) fn tick_sign_editors(&mut self) {
        let held: Vec<(i32, i32, i32)> = self
            .common
            .world
            .block_entities
            .sign_editors()
            .copied()
            .collect();
        for pos in held {
            match self.common.world.block_entities.sign_editor(&pos) {
                Some(who) if self.sign_editor_in_range(who, pos) => {}
                Some(_) => self.common.world.block_entities.set_sign_editor(pos, None),
                // A sign that is gone, or one no editor holds.
                None => self.common.world.block_entities.forget_sign_editor(pos),
            }
        }
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
