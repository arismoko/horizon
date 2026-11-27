//! Item frames and glow item frames: `ItemFrameItem` placement on any face,
//! `ItemFrame.survives`, `interact` (put an item in, then rotate it),
//! `hurtServer` (take the item out, then break the frame) and
//! `ItemFrameRenderer` (the frame model and the item at a 45 degree step).
use super::{Hit, InteractTarget};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_gameplay_common::game::frames_common::center;
use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_gameplay_common::painting_variants::frame_coexists;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemFrame;
pub(in crate::game::server_side) fn facing_of(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    }
}
pub(in crate::game) fn sound(f: &ItemFrame, what: &str) -> String {
    let prefix = if f.glow {
        "glow_item_frame"
    } else {
        "item_frame"
    };
    format!("minecraft:entity.{prefix}.{what}")
}
/// `HangingEntity.spawnAtLocation` widens the signed horizontal offsets after multiplying in f32.
pub(in crate::game) fn hanging_drop_position(center: [f64; 3], step: [i32; 3]) -> [f64; 3] {
    [
        center[0] + f64::from(step[0] as f32 * 0.15_f32),
        center[1],
        center[2] + f64::from(step[2] as f32 * 0.15_f32),
    ]
}
impl crate::game::ServerGame<'_> {
    /// `ItemFrame.survives` (not fixed): no collision, a solid block behind
    /// (or a diode for wall frames), no overlapping hanging entity.
    pub(in crate::game) fn frame_survives(&self, f: &ItemFrame) -> bool {
        if f.fixed {
            return true;
        }
        let (lo, hi) = f.bounds();
        let bounds = Aabb::new(
            Vec3::new(lo[0], lo[1], lo[2]),
            Vec3::new(hi[0], hi[1], hi[2]),
        );
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            return false;
        }
        let s = f.step();
        let behind = self.common.res.facts().get(self.common.world.block(
            f.pos[0] - s[0],
            f.pos[1] - s[1],
            f.pos[2] - s[2],
        ));
        let supported = behind.legacy_solid || (s[1] == 0 && behind.has_class("DiodeBlock"));
        // canCoexist(true): only hanging entities facing the same way block it.
        supported && frame_coexists(f, &self.common.item_frames, &self.common.paintings)
    }
    /// Asks the mods whether the acting guest may place a hanging entity
    /// of type `kind` at `pos` (`native_hanging_place`): true when one
    /// cancelled it. A game's own player is not asked.
    pub(in crate::game) fn hanging_place_cancelled(&mut self, kind: &str, pos: [i32; 3]) -> bool {
        let (Some(uuid), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) else {
            return false;
        };
        let mut cancel = false;
        self.native_hanging_place(uuid, &name, kind, pos, &mut cancel);
        cancel
    }
    /// Asks the mods whether a hanging entity of type `kind` at `pos` may
    /// break (`native_hanging_break`): true when one cancelled it. A break by
    /// a player is asked only for the acting guest, with them as the player;
    /// a break by anything else is asked with none.
    pub(in crate::game) fn hanging_break_cancelled(
        &mut self,
        kind: &str,
        pos: [i32; 3],
        by_player: bool,
    ) -> bool {
        let player = if by_player {
            let Some(uuid) = self.server.lan_acting_uuid else {
                return false;
            };
            Some(uuid)
        } else {
            None
        };
        let mut cancel = false;
        self.native_hanging_break(player, kind, pos, &mut cancel);
        cancel
    }
    /// `ItemFrameItem.useOn`.
    pub(in crate::game::server_side) fn place_item_frame(
        &mut self,
        hit: &Hit,
        stack: &ItemStack,
    ) -> bool {
        let glow = match &*stack.item {
            "minecraft:item_frame" => false,
            "minecraft:glow_item_frame" => true,
            _ => return false,
        };
        let pos: [i32; 3] = std::array::from_fn(|i| hit.pos[i] + hit.face[i]);
        // `ItemFrameItem.mayPlace`: `Level.isInsideBuildHeight(blockPos)`, so a frame is not
        // placed on the top face of the highest block or under the lowest one.
        if !(sunrise_level::chunk::MIN_Y
            ..sunrise_level::chunk::MIN_Y + sunrise_level::chunk::HEIGHT)
            .contains(&pos[1])
        {
            // ItemFrameItem.mayPlace's FAIL.
            self.server.use_failed = true;
            return true;
        }
        let mut frame = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.server.next_entity + 1,
            pos,
            facing: facing_of(hit.face),
            glow,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            // `applyComponentsFromItemStack` sets `CUSTOM_NAME`.
            custom_name: stack.data().and_then(|d| d.custom_name.clone()),
            random: sunrise_level::random_source::entity(),
        };
        // `EntityType.createDefaultStackConfig`: the item's `custom_data`, then its
        // `entity_data` merged over the frame (`TypedEntityData.loadInto`), before
        // `survives` runs on the configured frame.
        let kind = frame.kind();
        let own = sunrise_sim::entity_nbt::item_frame_to_nbt(
            &frame,
            [0; 4],
            None,
            self.common.res.items(),
        );
        let configured = sunrise_sim::block_entity_item::stack_configured_entity(stack, kind, &own);
        if let Some(mut loaded) =
            sunrise_sim::entity_nbt::item_frame_from_nbt(&configured, frame.id)
        {
            loaded.glow = glow;
            frame = loaded;
        }
        // HangingEntity.survives false: CONSUME, which ItemStack.useOn still counts as a use.
        if !self.frame_survives(&frame) {
            self.award_stat("used", &stack.item, 1);
            return true;
        }
        if self.hanging_place_cancelled(kind, pos) {
            return true;
        }
        self.server.next_entity += 1;
        self.server_play_sound(&sound(&frame, "place"), center(&frame), 1.0, 1.0);
        // HangingEntityItem.useOn: ENTITY_PLACE at the frame, by the player, before it is
        // added; ItemStack.useOn then counts the use.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:entity_place", center(&frame), ctx);
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
        // The unmodelled rest of the loaded data (`data`, `Tags`...) stays with the frame.
        let mut raw = configured;
        if let sunrise_core::nbt::Tag::Compound(map) = &mut raw {
            map.remove(&"UUID".into());
        }
        self.common.persist.raw.insert(frame.id, raw);
        self.common.item_frames.push(frame);
        if !self.common.creative {
            self.consume_held();
        }
        self.award_stat("used", &stack.item, 1);
        true
    }
    /// `ItemFrame.interact` for the frame under the crosshair, or the one a
    /// guest's interact packet named.
    pub fn interact_frame(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => sunrise_gameplay_common::game::item_actions::named_entity(
                &self.common.item_frames,
                t.id,
                |f| f.id,
            )
            .map(|i| (i, 0.0)),
            None => self.frame_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if (target.is_none() && !self.before_block(d)) || self.common.item_frames[i].fixed {
            return false;
        }
        let at = center(&self.common.item_frames[i]);
        let entity_id = self.common.item_frames[i].id;
        let action;
        if self.common.item_frames[i].item.is_none() {
            let Some(held) = self.common.held().cloned() else {
                return false;
            };
            // A map tracking more than 256 counted markers stays in the hand
            // (InteractionResult.FAIL): nothing else is tried, no swing (#11383).
            if let Some(id) = super::maps::map_id(&held)
                && self.map(id).is_some_and(|m| m.tracked_over_limit(256))
            {
                return true;
            }
            self.common.item_frames[i].set_item(Some(held));
            action = "put";
            self.server_play_sound(
                &sound(&self.common.item_frames[i], "add_item"),
                at,
                1.0,
                1.0,
            );
        } else {
            let s = sound(&self.common.item_frames[i], "rotate_item");
            self.server_play_sound(&s, at, 1.0, 1.0);
            let f = &mut self.common.item_frames[i];
            f.rotation = (f.rotation + 1) % 8;
            action = "rotate";
        }
        let p = self.common.item_frames[i].pos;
        self.common.world.modified.insert((p[0] >> 4, p[2] >> 4));
        self.frame_output_changed(p);
        // ItemFrame.interact: BLOCK_CHANGE by the player after setItem or
        // setRotation, then the held item is consumed.
        let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
        self.post_game_event("minecraft:block_change", at, ctx);
        if action == "put" && !self.common.creative {
            self.consume_held();
        }
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            let frame = self.common.item_frames.iter().find(|f| f.id == entity_id);
            let item = frame
                .and_then(|f| f.item.as_ref())
                .map_or_else(String::new, |s| s.item.to_string());
            let rotation = frame.map_or(0, |f| f.rotation);
            self.native_player_item_frame_change(
                player,
                &name,
                entity_id,
                action,
                &item,
                rotation.rem_euclid(8) as u8,
            );
        }
        self.swing_arm();
        true
    }
    /// `ItemFrame.hurtServer` from the player: the item comes out first.
    pub fn attack_frame(&mut self) -> bool {
        let Some((i, d)) = self.frame_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        if self.common.item_frames[i].fixed {
            // A fixed frame refuses the hit of a player who isn't in creative
            // (canHurtWhenFixed), so skipAttackInteraction is false and
            // Player.attack goes on to onAttack.
            if !self.common.creative {
                self.server.attack_ticks = 0;
            }
            return true;
        }
        // `isInvulnerableToBase`: a refused hit goes on to onAttack, as a fixed frame's does.
        if self.native_player_hit_invulnerable(
            self.common.item_frames[i].id,
            self.common.item_frames[i].kind(),
        ) {
            self.server.attack_ticks = 0;
            return true;
        }
        self.hit_frame_by(
            i,
            true,
            self.common.creative,
            Some(super::commands::Target::Player),
            false,
        );
        true
    }
    /// `ItemFrame.hurtServer` from a projectile with full source attribution.
    pub(in crate::game::server_side) fn projectile_hurts_frame_by(
        &mut self,
        id: u32,
        by_player: bool,
        creative: bool,
        source: Option<super::commands::Target>,
        damage_type: &str,
    ) -> bool {
        let Some(i) = self.common.item_frames.iter().position(|f| f.id == id) else {
            return false;
        };
        let bypasses = self
            .common
            .health_resources
            .damage
            .id(damage_type)
            .is_some_and(|k| {
                self.common
                    .health_resources
                    .damage
                    .in_tag(k, "minecraft:bypasses_invulnerability")
            });
        let mob_blocked = !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
            && matches!(source, Some(super::commands::Target::Mob(_)));
        let fixed = self.common.item_frames[i].fixed;
        if fixed {
            if (!creative && !bypasses) || mob_blocked {
                return false;
            }
        } else if self.common.item_frames[i].item.is_none() && mob_blocked {
            return false;
        }
        self.hit_frame_by(i, by_player, creative, source, fixed);
        true
    }
    /// Non-fixed part of `ItemFrame.hurtServer`: filled drops item, empty or fixed breaks.
    pub(in crate::game::server_side) fn hit_frame_by(
        &mut self,
        mut i: usize,
        by_player: bool,
        creative: bool,
        source: Option<super::commands::Target>,
        force_break: bool,
    ) {
        let with_frame = force_break || self.common.item_frames[i].item.is_none();
        let entity_id = self.common.item_frames[i].id;
        if with_frame {
            let (kind, pos) = (
                self.common.item_frames[i].kind(),
                self.common.item_frames[i].pos,
            );
            if self.hanging_break_cancelled(kind, pos, by_player) {
                return;
            }
            // The listener had the game in its hands: find the frame again by id.
            match self
                .common
                .item_frames
                .iter()
                .position(|f| f.id == entity_id)
            {
                Some(now) if force_break || self.common.item_frames[now].item.is_none() => i = now,
                _ => return,
            }
        }
        let old_item = self.common.item_frames[i]
            .item
            .as_ref()
            .map_or("", |s| &*s.item)
            .to_owned();
        let rotation = self.common.item_frames[i].rotation;
        let at = center(&self.common.item_frames[i]);
        let mut f = if with_frame {
            self.common.item_frames.remove(i)
        } else {
            self.common.item_frames[i].clone()
        };
        if !with_frame {
            self.common.item_frames[i].item = None;
        } else {
            // BlockAttachedEntity.kill precedes ItemFrame.dropItem's break sound.
            self.entity_killed_event_by(at, source);
            self.server_play_sound(&sound(&f, "break"), at, 1.0, 1.0);
        }
        // ItemFrame.dropItem clears the item (updating comparators) before drops.
        if f.item.is_some() {
            self.frame_output_changed(f.pos);
        }
        self.drop_frame_items(&mut f, with_frame, by_player, creative);
        // Drops run listeners: the random state goes back to the frame with
        // this id, wherever it is now.
        if !with_frame
            && let Some(frame) = self
                .common
                .item_frames
                .iter_mut()
                .find(|frame| frame.id == entity_id)
        {
            frame.random = f.random.clone();
        }
        // BLOCK_CHANGE by the attacker after the drops: ItemFrame.hurtServer
        // for the item, ItemFrame.dropItem(level, causedBy) for the frame.
        let ctx = match source {
            Some(super::commands::Target::Player) => {
                self.who_context(Some(sunrise_sim::mobs::Who::Player), None)
            }
            Some(super::commands::Target::Guest(conn)) => {
                let who = self
                    .command_guest(conn)
                    .map(|g| sunrise_sim::mobs::Who::Mob(g.entity_id as u32))
                    .or(Some(sunrise_sim::mobs::Who::Player));
                self.who_context(who, None)
            }
            Some(super::commands::Target::Mob(id)) => {
                self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None)
            }
            _ => {
                let who = by_player.then_some(sunrise_sim::mobs::Who::Player);
                self.who_context(who, None)
            }
        };
        self.post_game_event("minecraft:block_change", at, ctx);
        if !with_frame {
            // ItemFrame.hurtServer plays this after the event.
            self.server_play_sound(&sound(&f, "remove_item"), at, 1.0, 1.0);
        }
        self.common
            .world
            .modified
            .insert((f.pos[0] >> 4, f.pos[2] >> 4));
        if by_player
            && let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            self.native_player_item_frame_change(
                player,
                &name,
                entity_id,
                if with_frame { "break" } else { "remove_item" },
                if with_frame { "" } else { &old_item },
                rotation.rem_euclid(8) as u8,
            );
        }
    }
    /// `ItemFrame.setItem` / `setRotation`: `updateNeighbourForOutputSignal`, so
    /// the comparators reading the frame take its new signal.
    pub(in crate::game) fn frame_output_changed(&mut self, pos: [i32; 3]) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.update_neighbour_for_output_signal(level, ticks, at));
    }
    /// `ItemFrame.dropItem`: the frame (when breaking) and the held item.
    pub(in crate::game) fn drop_frame_items(
        &mut self,
        f: &mut ItemFrame,
        with_frame: bool,
        by_player: bool,
        creative: bool,
    ) {
        // The framed map forgets the frame, except when the drops are off and
        // something caused the break (`removeFramedMap` runs only for a null
        // `causedBy` then).
        if self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
            || !by_player
        {
            self.remove_framed_map(f);
        }
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
            || (by_player && creative)
        {
            return;
        }
        let c = center(f);
        let at = hanging_drop_position(c, f.step());
        let mut drops = Vec::new();
        if with_frame {
            // `ItemFrame.getFrameItemStackWithData` (glow frames share it).
            drops.push(f.item_stack_with_data());
        }
        if let Some(item) = f.item.clone()
            && f.random.next_float() < f.drop_chance
        {
            drops.push(item);
        }
        if by_player && !drops.is_empty() {
            let tool = self.common.held().cloned();
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                "",
                c,
                None,
                Some((f.id, f.kind())),
                player,
                tool.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            self.spawn_item(stack, at, [0.0; 3], 10);
        }
    }
    /// `ServerExplosion.hurtEntities` for item frames: a frame within the blast
    /// is killed and dropped with its item (`BlockAttachedEntity.hurtServer`).
    /// A fixed frame is not hurt by an explosion, and `blast_spares_decorations`
    /// keeps every frame without `mobGriefing`.
    pub(in crate::game::server_side) fn explosion_breaks_frames(
        &mut self,
        blast: [f64; 3],
        diameter: f64,
        interaction: super::explosion::BlockInteraction,
    ) {
        if self.blast_spares_decorations(interaction) {
            return;
        }
        let mut i = 0;
        while i < self.common.item_frames.len() {
            let c = center(&self.common.item_frames[i]);
            let dist =
                ((c[0] - blast[0]).powi(2) + (c[1] - blast[1]).powi(2) + (c[2] - blast[2]).powi(2))
                    .sqrt()
                    / diameter;
            if dist > 1.0
                || self.common.item_frames[i].fixed
                || !self.server.combat.0.visits(self.common.item_frames[i].id)
                || !self.break_frame_by(i, self.explosion_kill_credit())
            {
                i += 1;
            }
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// `BlockAttachedEntity.kill`, then `ItemFrame.dropItem(level, causedBy)`
    /// for the frame at `index`, the kill credited to `source`: the break
    /// sound, the frame and its item, then `BLOCK_CHANGE`. False when a mod
    /// kept it (`native_hanging_break`): it stays at `index`.
    pub fn break_frame_by(
        &mut self,
        index: usize,
        source: Option<super::commands::Target>,
    ) -> bool {
        let (id, kind, pos, was_empty) = (
            self.common.item_frames[index].id,
            self.common.item_frames[index].kind(),
            self.common.item_frames[index].pos,
            self.common.item_frames[index].item.is_none(),
        );
        if self.hanging_break_cancelled(kind, pos, false) {
            return false;
        }
        // Found again by id after the listener: gone, or empty before and
        // filled now, nothing breaks.
        let Some(index) = self
            .common
            .item_frames
            .iter()
            .position(|f| f.id == id && !(was_empty && f.item.is_some()))
        else {
            return false;
        };
        let mut f = self.common.item_frames.remove(index);
        self.entity_killed_event_by(center(&f), source);
        self.server_play_sound(&sound(&f, "break"), center(&f), 1.0, 1.0);
        self.drop_frame_items(&mut f, true, false, false);
        // ItemFrame.dropItem(level, causedBy): BLOCK_CHANGE after the drops.
        let ctx = self.who_context(None, None);
        self.post_game_event("minecraft:block_change", center(&f), ctx);
        self.common
            .world
            .modified
            .insert((f.pos[0] >> 4, f.pos[2] >> 4));
        if f.item.is_some() {
            self.frame_output_changed(f.pos);
        }
        true
    }
    /// `ItemFrame.setItem(stack, true)`: keeps one item, plays the add sound
    /// for a non-empty stack and updates the comparators.
    pub(in crate::game::server_side) fn set_frame_item(
        &mut self,
        i: usize,
        stack: Option<ItemStack>,
    ) {
        let f = &mut self.common.item_frames[i];
        f.set_item(stack);
        let (pos, filled) = (f.pos, f.item.is_some());
        if filled {
            let (name, at) = (
                sound(&self.common.item_frames[i], "add_item"),
                center(&self.common.item_frames[i]),
            );
            self.server_play_sound(&name, at, 1.0, 1.0);
        }
        self.frame_output_changed(pos);
    }
    /// BlockAttachedEntity.tick for frames.
    pub fn tick_item_frames(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // The frame whose tick just ended: it hears its Post before the next one's Pre.
        let mut told = None;
        // EntityTickList copy-on-iterate: while a listener may remove frames, the round visits
        // the frames that were there when it began, each found again by its join.
        let joins: Option<Vec<_>> = round
            .told_listening()
            .then(|| self.common.item_frames.iter().map(|f| f.join).collect());
        let mut turns = joins.iter().flatten();
        let mut i = 0;
        loop {
            if let Some((id, kind)) = told.take() {
                self.native_tick_post_one(&round, id, kind);
            }
            if joins.is_some() {
                let Some(&join) = turns.next() else { break };
                let Some(at) = self.common.item_frames.locate(join) else {
                    continue;
                };
                i = at;
            } else if i >= self.common.item_frames.len() {
                break;
            }
            let at = center(&self.common.item_frames[i]);
            if !ticking(at[0].floor() as i32, at[2].floor() as i32) {
                i += 1;
                continue;
            }
            let (id, kind) = (
                self.common.item_frames[i].id,
                self.common.item_frames[i].kind(),
            );
            // ServerLevel.tick skips an entity that `isRemoved`, here one whose removal is queued.
            let target = super::commands::Target::Frame(id);
            if self.target_removed(&target) || !self.native_tick_pre_one(&mut round, id, kind) {
                i += 1;
                continue;
            }
            // The Pre's listener may have removed frames, this one included.
            if self.target_removed(&target) {
                i += 1;
                continue;
            }
            if self.common.item_frames.get(i).is_none_or(|f| f.id != id) {
                let Some(now) = self.common.item_frames.iter().position(|f| f.id == id) else {
                    continue;
                };
                i = now;
            }
            told = Some((id, kind));
            // `Entity.tickCount` is raised before the rest of the tick, and is not saved.
            self.common.item_frames[i].ticks = self.common.item_frames[i].ticks.saturating_add(1);
            self.common.item_frames[i].check_interval += 1;
            if self.common.item_frames[i].check_interval >= 100 {
                self.common.item_frames[i].check_interval = 0;
                let f = self.common.item_frames[i].clone();
                if !self.frame_survives(&f) && !self.hanging_break_cancelled(f.kind(), f.pos, false)
                {
                    // Found again by id after the listener: gone, nothing is
                    // removed; empty before and filled now, it stays.
                    let Some(now) = self.common.item_frames.iter().position(|g| g.id == f.id)
                    else {
                        continue;
                    };
                    if f.item.is_none() && self.common.item_frames[now].item.is_some() {
                        i = now + 1;
                        continue;
                    }
                    i = now;
                    let mut f = self.common.item_frames.remove(i);
                    // BlockAttachedEntity.tick discards without a death event.
                    self.server_play_sound(&sound(&f, "break"), center(&f), 1.0, 1.0);
                    if f.item.is_some() {
                        self.frame_output_changed(f.pos);
                    }
                    self.drop_frame_items(&mut f, true, false, false);
                    // ItemFrame.dropItem(level, null): sound, drops, then event.
                    let ctx = self.who_context(None, None);
                    self.post_game_event("minecraft:block_change", center(&f), ctx);
                    continue;
                }
            }
            i += 1;
        }
        if let Some((id, kind)) = told {
            self.native_tick_post_one(&round, id, kind);
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
