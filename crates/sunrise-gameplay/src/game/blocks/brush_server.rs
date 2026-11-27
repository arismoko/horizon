use super::{Hit, decorated_pot};
use sunrise_core::random::RandomSource;
use sunrise_gameplay::brushing::{USE_DURATION, completion_state};
use sunrise_level::containers::BlockEntity;
use sunrise_sim::save;

impl crate::game::ServerGame<'_> {
    /// `BrushableBlockEntity.brush`: whether the block changed (the brush wears).
    pub(in crate::game::server_side) fn brush_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        slot: usize,
    ) -> bool {
        let time = self.common.world.game_time;
        let mut brush = self.server.brushing.remove(&pos).unwrap_or_default();
        brush.hit_face.get_or_insert(face);
        let direction = match brush.hit_face.unwrap() {
            [0, -1, 0] => 0,
            [0, 1, 0] => 1,
            [0, 0, -1] => 2,
            [0, 0, 1] => 3,
            [-1, 0, 0] => 4,
            [1, 0, 0] => 5,
            _ => return false,
        };
        if let Some(BlockEntity::Brushable(b)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.hit_direction = Some(direction);
        }
        brush.resets_at = time + 40;
        if time < brush.cooldown_ends_at {
            self.server.brushing.insert(pos, brush);
            return false;
        }
        brush.cooldown_ends_at = time + 10;
        self.unpack_brush_loot(pos, slot);
        let before = completion_state(brush.count);
        brush.count += 1;
        if brush.count >= 10 {
            self.brushing_completed(pos, brush.hit_face, slot);
            return true;
        }
        self.schedule_brushable_tick(pos);
        let after = completion_state(brush.count);
        self.server.brushing.insert(pos, brush);
        if before != after {
            self.set_dusted(pos, after);
        }
        false
    }
    /// One stroke on the block aimed at: dust (this window only), the brush
    /// sound, and the block entity's count. The brush wears when the block
    /// changed state.
    pub(in crate::game::server_side) fn brush_stroke(&mut self, hit: &Hit, slot: usize) {
        let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let name = self.common.res.facts().get(state).name.clone();
        let (sound, brushable) = match &*name {
            "minecraft:suspicious_sand" => ("minecraft:item.brush.brushing.sand", true),
            "minecraft:suspicious_gravel" => ("minecraft:item.brush.brushing.gravel", true),
            _ => ("minecraft:item.brush.brushing.generic", false),
        };
        let at = hit.pos.map(|v| f64::from(v) + 0.5);
        self.server_play_sound(sound, at, 1.0, 1.0);
        if brushable && self.brush_block(hit.pos, hit.face, slot) {
            self.damage_held(1);
        }
    }
    /// `BrushableBlockEntity.checkReset`, from `BrushableBlock.tick`: a block
    /// left alone loses two counts, again every four ticks.
    pub(in crate::game) fn brushable_tick(&mut self, pos: [i32; 3]) {
        let time = self.common.world.game_time;
        let Some(mut brush) = self.server.brushing.remove(&pos) else {
            return;
        };
        let before = completion_state(brush.count);
        if brush.count != 0 && time >= brush.resets_at {
            brush.count = (brush.count - 2).max(0);
            brush.resets_at = time + 4;
        }
        let after = completion_state(brush.count);
        if brush.count != 0 {
            self.server.brushing.insert(pos, brush);
            self.schedule_brushable_tick(pos);
        } else if let Some(BlockEntity::Brushable(b)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.hit_direction = None;
        }
        if before != after {
            self.set_dusted(pos, after);
        }
    }
    /// `BrushableBlockEntity.brushingCompleted`: the item drops out of the
    /// brushed face and the block turns into its sand or gravel.
    pub(in crate::game::server_side) fn brushing_completed(
        &mut self,
        pos: [i32; 3],
        face: Option<[i32; 3]>,
        slot: usize,
    ) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let turns_into = match &*self.common.res.facts().get(state).name {
            "minecraft:suspicious_sand" => "minecraft:sand",
            "minecraft:suspicious_gravel" => "minecraft:gravel",
            _ => "minecraft:air",
        };
        // dropContent: the item at the middle of the block beside that face.
        let item = match self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        {
            Some(BlockEntity::Brushable(b)) => b.item.clone(),
            _ => None,
        };
        if let Some(mut item) = item {
            let face = face.unwrap_or([0, 1, 0]);
            let take = 10 + self.common.world.random.next_int_bound(21).unwrap_or(0);
            item.count = item.count.min(take as u32);
            let at = [
                f64::from(pos[0] + face[0]) + 0.5,
                f64::from(pos[1] + face[1]) + 0.625,
                f64::from(pos[2] + face[2]) + 0.5,
            ];
            let mut drops = vec![item];
            let tool = self.common.stack_in(slot).cloned();
            let origin = pos.map(|c| f64::from(c) + 0.5);
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                "",
                origin,
                Some((pos, state)),
                None,
                player,
                tool.as_ref(),
                &mut drops,
            );
            for item in drops {
                self.spawn_item(item, at, [0.0; 3], 0);
            }
        }
        // Level event 3008 for every player, then the block change.
        self.server_level_event_at(3008, pos, state.0 as i32);
        if let Some(base) = self.common.pack.assets.registry.default_state(turns_into) {
            let _ = self.level_set(pos, base);
        }
    }
    /// `BrushItem.onUseTick` for a brush used `ticks` ticks so far, the
    /// hand of `slot` acting.
    pub(in crate::game::server_side) fn tick_brush(
        &mut self,
        stack: &sunrise_items::items::ItemStack,
        slot: usize,
        ticks: u32,
    ) {
        // No block under the crosshair: vanilla calls releaseUsingItem, which
        // stops the use and causes the interaction vibration.
        let Some(hit) = self.item_pick() else {
            self.stop_using_item(stack);
            return;
        };
        // `BrushItem.onUseTick` aims with `getHitResultOnViewVector` out to the
        // block interaction range, for mobs too: a mob between the player and
        // the block is nearer, so the hit is not a block and the use releases
        // the same way. A guest acts on its client's targeted block, whose view
        // we cannot trace here, so only the local aim runs the entity test.
        if self.server.forced_hit.is_none()
            && let Some((eye, _)) = self.aim()
            && let Some((_, mob, _)) = self.mob_hit_within(self.common.block_reach)
        {
            let block = (
                hit.point[0] - eye[0],
                hit.point[1] - eye[1],
                hit.point[2] - eye[2],
            );
            if mob <= (block.0 * block.0 + block.1 * block.1 + block.2 * block.2).sqrt() {
                self.stop_using_item(stack);
                return;
            }
        }
        let elapsed = (USE_DURATION as i32)
            .wrapping_sub(sunrise_gameplay_common::game::item_use_common::ticks_left(
                self.common.use_duration,
                ticks,
            ))
            .wrapping_add(1);
        // The last tick before the backswing.
        if elapsed % 10 == 5 {
            self.brush_stroke(&hit, slot);
        }
        // updateUsingItem owns the countdown and completion, including a
        // replacement remaining counter from the Tick event.
        self.common.using = Some((slot, ticks + 1));
    }
    /// `BrushableBlockEntity.unpackLootTable`: the table's roll becomes the
    /// block entity's item. `LootContextParamSets.ARCHAEOLOGY`: the brusher is
    /// THIS_ENTITY, the brush in `slot` the TOOL and the block's centre the
    /// ORIGIN, with the brusher's luck.
    pub(in crate::game::server_side) fn unpack_brush_loot(&mut self, pos: [i32; 3], slot: usize) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Brushable(b)) = self.common.world.block_entities.get(&key) else {
            return;
        };
        let Some(sunrise_core::nbt::Tag::String(table)) = &b.loot_table else {
            return;
        };
        let table = table.to_string_lossy();
        let mut tag = save::block_entity_encode::encode(
            key,
            &BlockEntity::Brushable(b.clone()),
            "minecraft:brushable_block",
            self.common.res.items(),
        );
        self.trigger_loot_table(&table);
        let loot = self.server.server_loot.0.get();
        let brusher = self.killer_loot_entity(None);
        let luck = self.acting_luck();
        let tool = self
            .common
            .stack_in(slot)
            .map(sunrise_gameplay_common::game::creatures::to_loot);
        let caller = self.caller_loot(Some(pos.map(|c| f64::from(c) + 0.5)), tool);
        caller.with(|| {
            decorated_pot::unpack_loot(
                &mut tag,
                loot.as_deref(),
                &mut self.common.world.random,
                &mut self.server.rand_sequences,
                self.common.level.seed,
                self.common.res.items(),
                sunrise_world::loot::ContainerLootContext {
                    luck,
                    entity: Some(&brusher),
                    ..Default::default()
                },
            )
        });
        if let Some(BlockEntity::Brushable(b)) = self.common.world.block_entities.get_mut(&key) {
            b.loot_table = tag.get("LootTable").cloned();
            b.table_seed_tag = tag.get("LootTableSeed").cloned();
            b.item = tag.get("item").and_then(save::item_from_nbt);
        }
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::brush::BrushGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
