//! The level tick: what `Game::tick` runs after the local player's own update (`tick_world`),
//! and the mob batch inside it (`tick_mobs`). Entities other than mobs tick from their own files;
//! `ordered_tick.rs` interleaves orbs, primed TNT and falling blocks by join order.
use super::*;

impl crate::game::ServerGame<'_> {
    /// What the tick runs after the local player's own update: effects, the
    /// world's entities and blocks, and the network hub.
    pub fn tick_world(&mut self, running: bool) -> Result<()> {
        // Entities added since the last tick join the level before it ticks.
        span("mods_joins_flush", || self.native_joins_flush(false, 0));
        span("share_spawn_conditions", || self.share_spawn_conditions());
        sunrise_sim::mobs::sulfur_cube::set_tnt_explodes(
            self.common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES),
        );
        self.update_chunk_tickets();
        self.score_destroyed_entities();
        sunrise_level::spans::time("tick_effects", || self.tick_effects(running));
        // Player.tick's ItemCooldowns.tick for the window's player (a
        // server's players count theirs down in `tick_guests`).
        self.server.shield_cooldown = self.server.shield_cooldown.saturating_sub(1);
        self.common.cooldowns.tick();
        sunrise_level::spans::time("tick_using", || self.tick_using_server());
        sunrise_level::spans::time("postprocess_chunks", || self.postprocess_chunks())?;
        let mut sent = None;
        if running {
            sunrise_level::spans::time("tick_lan_packets", || self.tick_lan_packets());
            sent = sunrise_level::spans::time("tick_blocks", || self.tick_blocks())?;
            // What the blocks spawned (a dispensed minecart, a lit TNT, a
            // placed boat) joins before the entities tick.
            span("mods_joins_flush", || self.native_joins_flush(false, 0));
            if self.dimension_type().has_ender_dragon_fight {
                // ServerLevel.tick runs the fight before visiting the entity tick list.
                span("tick_dragon_fight", || self.tick_fight());
                span("mods_joins_flush", || self.native_joins_flush(false, 0));
            }
            span("lan_mark_synced", || self.lan_mark_synced());
            // `EntityTickList.forEach`: what joined before the entity pass
            // (function summons, dropped or dispensed items) ticks in it;
            // what joins during it waits for the next tick
            // (`ensureActiveIsNotIterated`). Primed TNT and falling blocks
            // keep their own packet cutoff (`ordered.announced`, #10063).
            self.server.ticked_entities_through = self.server.next_entity;
        }
        // ChunkMap still ticks while /tick freeze holds block/entity ticks.
        if !running {
            span("save_chunks_eagerly", || self.save_chunks_eagerly());
        }
        self.begin_ordered_ticks();
        if running {
            sunrise_level::spans::time("tick_paintings", || self.tick_paintings());
            sunrise_level::spans::time("tick_item_frames", || self.tick_item_frames());
            self.tick_explicit_knots();
            self.tick_summoned_eyes();
            self.ordered_before(ordered_tick::Batch::ArmorStands)?;
            sunrise_level::spans::time("tick_armor_stands", || self.begin_armor_stand_ticks())?;
            sunrise_level::spans::time("tick_cushions", || self.tick_cushions());
            // ServerLevel.tick: the arrivals of this server tick are in the level
            // (`addDuringTeleport`), and each vehicle handles its portal as its
            // own tick opens.
            span("portal_arrivals", || self.land_portal_arrivals());
            span("vehicle_portals", || self.handle_vehicle_portals());
            self.ordered_before(ordered_tick::Batch::Boats)?;
            sunrise_level::spans::time("tick_boats", || self.tick_boats());
            self.ordered_before(ordered_tick::Batch::Carts)?;
            sunrise_level::spans::time("tick_carts", || self.tick_carts());
            self.tick_vehicle_watch();
            let from = self.lan_synced_from();
            sunrise_level::spans::time("tick_thunder", || self.tick_thunder());
            self.lan_synced_since(from);
            sunrise_level::spans::time("tick_lecterns", || self.tick_lecterns());
            sunrise_level::spans::time("tick_maps", || self.tick_maps());
            self.tick_lodestone_compasses();
            sunrise_level::spans::time("tick_bolts", || self.server_tick_bolts());
            self.ordered_before(ordered_tick::Batch::Fishing)?;
            sunrise_level::spans::time("tick_fishing", || self.tick_fishing());
            self.ordered_before(ordered_tick::Batch::Tridents)?;
            sunrise_level::spans::time("tick_tridents", || self.tick_tridents());
            self.ordered_before(ordered_tick::Batch::Rockets)?;
            sunrise_level::spans::time("tick_rockets", || self.tick_rockets());
            sunrise_level::spans::time("tick_ominous_item_spawners", || {
                self.tick_ominous_item_spawners()
            });
        }
        if running {
            sunrise_level::spans::time("entity_portals", || {
                self.handle_entity_portals();
                self.handle_end_gateway_portals();
            });
            self.ordered_before(ordered_tick::Batch::Items)?;
            sunrise_level::spans::time("tick_items", || self.tick_items());
            if self.common.is_logical_server() {
                span("hoppers_catch_items", || self.hoppers_catch_items());
            }
            // Merged, burnt, despawned or picked up.
            span("mods_leaves_flush", || self.native_leaves_flush());
            span("mods_joins_flush", || self.native_joins_flush(false, 0));
            sunrise_level::spans::time("touch_arrows", || self.touch_arrows());
        }
        sunrise_level::spans::time("spawn_loaded_entities", || self.spawn_loaded_entities());
        // The ride the player's file names, once the vehicle is in the world.
        self.restore_root_vehicle();
        if running {
            self.ordered_before(ordered_tick::Batch::Mobs)?;
            sunrise_level::spans::time("tick_mobs", || self.tick_mobs())?;
            // Dead, despawned or gone into a hive.
            span("mods_leaves_flush", || self.native_leaves_flush());
            sunrise_level::spans::time("recheck_golem_openers", || self.recheck_golem_openers());
            span("mod_viewers", || self.tick_mod_container_viewers());
            // The mobile kinds' entities that joined before the earliest
            // general row still to tick go first, in join order; then the
            // general rows (#792), then the mobile kinds' remainder.
            self.ordered_before(ordered_tick::Batch::General)?;
            sunrise_level::spans::time("tick_general", || self.tick_general())?;
            // Orbs, primed TNT, falling blocks and observed unsimulated entities still to tick.
            self.run_ordered_ticks(sunrise_sim::join_seq::JoinSeq::MAX, false)?;
            sunrise_level::spans::time("tick_experience", || self.tick_experience());
            // Merged, despawned or picked up.
            span("mods_leaves_flush", || self.native_leaves_flush());
            sunrise_level::spans::time("entity_portals", || {
                self.find_entity_portals();
                self.resolve_arrivals();
            });
        }
        // The rest of the tick's spawns join, and what left is told of.
        span("mods_joins_flush", || self.native_joins_flush(false, 0));
        span("mods_leaves_flush", || self.native_leaves_flush());
        // ServerLevel.tick broadcasts the changed blocks after the scheduled and
        // random ticks (`tick_blocks`), before the block events, the entities
        // and the block entities: what those change goes out with the next
        // tick's. A piston's moved block reaches the clients once their own
        // copy of it finished moving, so a slow tick rate shows it slide
        // (#8430).
        if let Some(sent_blocks) = sent
            && let Some(log) = self.common.world.block_log.as_mut()
        {
            self.common.world.block_log_held = log.split_off(sent_blocks.min(log.len()));
        }
        sunrise_level::spans::time("tick_lan", || self.tick_lan(false));
        let held = std::mem::take(&mut self.common.world.block_log_held);
        if let Some(log) = self.common.world.block_log.as_mut() {
            log.extend(held);
        }
        sunrise_level::spans::time("tick_sleep", || self.tick_sleep());
        span("sleep_status", || self.announce_sleep_status());
        span("sleep_status", || self.free_left_beds());
        if running {
            sunrise_level::spans::time("tick_end", || self.tick_end())?;
            sunrise_level::spans::time("end_gateways", || {
                self.find_end_gateway_portals();
                self.resolve_end_gateway_arrivals()
            })?;
            span("end_gateways", || self.keep_gateway_destinations());
        }
        if running {
            // Primed TNT and falling blocks that joined since the spawn packets.
            self.run_ordered_ticks(sunrise_sim::join_seq::JoinSeq::MAX, true)?;
            span("late_wind_pushes", || self.apply_late_wind_pushes());
        }
        // Everything that has joined has had its spawn packet and, if it joined
        // before the entity passes (TNT a piston or repeater primed), its first
        // tick in the late pass above (#10063).
        self.server.ordered.announced = sunrise_sim::join_seq::JoinSeq::upcoming();
        if running {
            sunrise_level::spans::time("tick_moving_pistons", || self.tick_moving_pistons())?;
            sunrise_level::spans::time("tick_block_entities", || self.tick_block_entities())?;
            sunrise_level::spans::time("tick_sculk_entities", || self.tick_sculk_entities());
            sunrise_level::spans::time("tick_sculk_blocks", || self.tick_sculk_blocks())?;
        }
        if running {
            // The bells' and conduits' server tickers, after the other block entities.
            sunrise_level::spans::time("tick_block_entity_servers", || {
                self.tick_block_entity_servers()
            });
            // A conduit's hurt teleports an enderman now (`Enderman.hurtServer`).
            span("settle_hurt_teleports", || self.settle_hurt_teleports())?;
        }
        // ServerLevel.tick's `entityManager.tick()`, frozen or not, after every
        // block entity ticker (the bells, conduits and gateways above too): the
        // chunks loaded before this pass have their entities loaded from the next.
        self.common.world.entities_pending.clear();
        if running {
            // MinecraftServer.tickServer: GameTestTicker.SINGLETON.tick().
            span("tick_game_tests", || self.tick_game_tests());
        }
        // ServerBossEvent broadcasts, after this level's entities have ticked.
        span("publish_boss_bars", || self.publish_boss_bars());
        if running {
            sunrise_level::spans::time("lan_note_joined", || self.lan_note_joined());
        }
        Ok(())
    }

    pub fn tick_mobs(&mut self) -> Result<()> {
        self.tick_creatures()?;
        if !self.common.entity_attachments.is_empty() {
            self.prune_entity_attachments();
        }
        self.refresh_stand_and_dragon_equipment_attributes();
        self.position_rider();
        self.tick_player_stat_counters();
        self.tick_scoreboard();
        self.load_bossbars();
        self.common.player_border();
        self.border_damage();
        self.tick_advancements();
        Ok(())
    }
}

use sunrise_gameplay_common::game::mounts_common::MountsCommonGameExt1 as _;
