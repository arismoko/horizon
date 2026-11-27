//! Test instance blocks on the server: `handleTestInstanceBlockAction` and
//! the `TestInstanceBlockEntity` operations behind it (reset, save, export,
//! the structure's placement), over the game tests the registry holds.
use super::{
    game_tests::Info,
    structure_blocks::{create_seed, template_size},
    test_entities::{aabb_of, inflate},
};
use sunrise_core::{position::BlockPos, ticks::TickArea};
use sunrise_game_data::text::{self, Text};
use sunrise_level::{
    containers::BlockEntity,
    structure_block::rotation_from_ordinal,
    test_instance_block::{Data, Status, TestInstanceBlock},
};
use sunrise_protocol::packets::play::{self, Clientbound as C};
use {
    sunrise_core::bounding_box::BoundingBox,
    sunrise_world::{
        level::{TemplateSettings, flags},
        worldgen::structure::{Mirror, Rotation},
    },
};

pub(in crate::game) type Pos = (i32, i32, i32);

/// `ServerboundTestInstanceBlockActionPacket.Action`.
pub(in crate::game) const INIT: i32 = 0;
pub(in crate::game) const QUERY: i32 = 1;
pub(in crate::game) const RESET: i32 = 3;
pub(in crate::game) const SAVE: i32 = 4;
pub(in crate::game) const EXPORT: i32 = 5;
pub(in crate::game) const RUN: i32 = 6;

#[allow(unused_imports)]
pub use sunrise_gameplay_common::test_instances::{TestDef, parse_test_def};

#[cfg(any(test, feature = "test-support"))]
pub trait TestDefSample {
    fn sample(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Self;
}

#[cfg(any(test, feature = "test-support"))]
impl TestDefSample for TestDef {
    /// A test with the timing and type a runner test needs.
    fn sample(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Self {
        Self {
            structure: "minecraft:empty".into(),
            rotation: Rotation::None,
            padding: 0,
            required: true,
            sky_access: false,
            block_based,
            function: None,
            callback: None,
            environment: "minecraft:default".into(),
            inline_environment: None,
            dimension: "minecraft:overworld".into(),
            max_ticks,
            setup_ticks,
            max_attempts: 1,
            required_successes: 1,
            description: Text::literal(""),
        }
    }
}

pub(in crate::game::server_side) fn red(key: &str, args: &[Text]) -> Text {
    Text::translate(key, args).with_color(text::RED)
}

impl crate::game::ServerGame<'_> {
    /// Resolve a test from the world's frozen test instance registry.
    pub fn test_def(&self, id: &str) -> Option<TestDef> {
        let table = match &self.server.world_tables.0 {
            Some(tables) => &tables.test_instances,
            None => self.common.res.test_instances(),
        };
        if id.contains(':') {
            table.get(id).cloned()
        } else {
            table.get(&format!("minecraft:{id}")).cloned()
        }
    }

    /// Publish the frozen test instance and environment IDs for command completion.
    pub fn sync_test_registry_ids(&mut self) {
        let Some(tables) = self.server.world_tables.0.as_ref() else {
            return;
        };
        let mut instance_ids: Vec<String> = tables.test_instances.keys().cloned().collect();
        instance_ids.sort();
        self.common
            .ui
            .registries
            .by_registry
            .entry("minecraft:test_instance".into())
            .or_default()
            .ids = instance_ids;
        let mut env_ids: Vec<String> = tables.test_environments.keys().cloned().collect();
        env_ids.sort();
        self.common
            .ui
            .registries
            .by_registry
            .entry("minecraft:test_environment".into())
            .or_default()
            .ids = env_ids;
    }

    pub(in crate::game) fn test_instance_at(&self, pos: Pos) -> Option<&TestInstanceBlock> {
        match self.common.world.block_entities.get(&pos)? {
            BlockEntity::TestInstanceBlock(b) => Some(b),
            _ => None,
        }
    }

    pub(in crate::game) fn test_instance_mut(
        &mut self,
        pos: Pos,
    ) -> Option<&mut TestInstanceBlock> {
        match self.common.world.block_entities.get_mut(&pos)? {
            BlockEntity::TestInstanceBlock(b) => Some(b),
            _ => None,
        }
    }

    /// `getTestHolder`.
    pub(in crate::game::server_side) fn test_holder(&self, pos: Pos) -> Option<TestDef> {
        let key = self.test_instance_at(pos)?.data.test.clone()?;
        self.test_def(&key)
    }

    /// The padding of the block's test, 0 without one.
    pub(in crate::game) fn test_holder_padding(&self, pos: Pos) -> i32 {
        self.test_holder(pos).map_or(0, |t| t.padding)
    }

    pub(in crate::game) fn server_test_holder_padding(&self, pos: Pos) -> i32 {
        self.test_holder_padding(pos)
    }

    /// `getRotation`: the test's rotation turned by the block's.
    pub(in crate::game) fn test_rotation(&self, pos: Pos) -> Rotation {
        let own = self
            .test_instance_at(pos)
            .map_or(Rotation::None, |b| b.data.rotation);
        self.test_holder(pos)
            .map_or(Rotation::None, |t| t.rotation)
            .get_rotated(own)
    }

    pub(in crate::game) fn server_test_rotation(&self, pos: Pos) -> Rotation {
        self.test_rotation(pos)
    }

    /// `getTestName`.
    pub(in crate::game) fn test_name(&self, pos: Pos) -> Text {
        match self.test_instance_at(pos).and_then(|b| b.data.test.clone()) {
            Some(key) => Text::literal(key),
            None => Text::translate("test_instance_block.invalid_test", &[]),
        }
    }

    /// `ServerGamePacketListenerImpl.handleTestInstanceBlockAction` for a
    /// guest: only a creative player with game master commands. A query or
    /// the screen's first look answers with a status packet.
    pub(in crate::game::server_side) fn lan_test_instance_action(
        &mut self,
        conn: u32,
        p: play::TestInstanceBlockAction,
    ) {
        let Some(g) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        let reply = self
            .as_guest_messaged(conn, |game| {
                allowed
                    .then(|| game.apply_test_instance_action(&p))
                    .flatten()
            })
            .flatten();
        if let Some((status, size)) = reply
            && let Some(host) = self.server.lan.as_mut()
        {
            host.hub.send_to(
                conn,
                &C::TestInstanceBlockStatus(play::TestInstanceBlockStatus {
                    status: sunrise_sim::lan::data::component_nbt(&status.component()),
                    size,
                }),
            );
        }
    }

    pub(in crate::game::server_side) fn apply_test_instance_action(
        &mut self,
        p: &play::TestInstanceBlockAction,
    ) -> Option<(Text, Option<[i32; 3]>)> {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        self.test_instance_at(pos)?;
        // `ByIdMap.continuous(.., ZERO)`: an unknown action is INIT.
        let action = if (0..=RUN).contains(&p.action) {
            p.action
        } else {
            INIT
        };
        if action != QUERY && action != INIT {
            // `blockEntity.set(packet.data())`.
            let data = Data {
                test: p.data.test.clone(),
                size: p.data.size,
                rotation: rotation_from_ordinal(p.data.rotation).unwrap_or(Rotation::None),
                ignore_entities: p.data.ignore_entities,
                status: Status::by_index(p.data.status),
                error_message: p.data.error_message.clone(),
            };
            if let Some(b) = self.test_instance_mut(pos) {
                b.data = data;
            }
            match action {
                RESET => self.reset_test(pos),
                SAVE => {
                    self.save_test(pos);
                }
                EXPORT => self.export_test(pos),
                RUN => self.run_test(pos),
                _ => {}
            }
            self.mark_block_entity_updated(pos);
            return None;
        }
        let def = p.data.test.as_deref().and_then(|key| self.test_def(key));
        let status = def.map_or_else(
            || red("test_instance.description.no_test", &[]),
            |d| d.description,
        );
        // QUERY: the size of the test's structure.
        let size = (action == QUERY)
            .then(|| {
                let def = p.data.test.as_deref().and_then(|key| self.test_def(key))?;
                let entry = self.template_entry(&def.structure)?;
                Some(template_size(&entry.tag))
            })
            .flatten();
        Some((status, size))
    }

    /// `getStructureBoundingBox`.
    pub(in crate::game) fn test_structure_box(&self, pos: Pos) -> Option<BoundingBox> {
        let padding = self.test_holder(pos).map_or(0, |t| t.padding);
        let rotation = self.test_rotation(pos);
        Some(
            self.test_instance_at(pos)?
                .structure_box([pos.0, pos.1, pos.2], padding, rotation),
        )
    }

    /// `processStructureBoundary`: the floor and walls (and the ceiling,
    /// unless the test has sky access) of the box around the structure.
    pub(in crate::game::server_side) fn structure_boundary(&self, pos: Pos) -> Vec<Pos> {
        let Some(bounds) = self.test_structure_box(pos) else {
            return Vec::new();
        };
        let ceiling = !self.test_holder(pos).is_some_and(|t| t.sky_access);
        let low = bounds.min.map(|v| v - 1);
        // BlockPos.containing(maxX...) of the AABB, one past the box.
        let high = bounds.max.map(|v| v + 1);
        let mut out = Vec::new();
        for x in low[0]..=high[0] {
            for y in low[1]..=high[1] {
                for z in low[2]..=high[2] {
                    let edge =
                        x == low[0] || x == high[0] || z == low[2] || z == high[2] || y == low[1];
                    if edge || (y == high[1] && ceiling) {
                        out.push((x, y, z));
                    }
                }
            }
        }
        out
    }

    /// `setBlockAndUpdate(pos, state)`.
    pub(in crate::game) fn set_test_block_state(&mut self, at: Pos, name: &str, flags: i32) {
        let Some(id) = self.common.pack.assets.registry.default_state(name) else {
            return;
        };
        let pos = BlockPos::new(at.0, at.1, at.2);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, id, flags));
    }

    /// `removeBarriers`.
    pub(in crate::game) fn remove_test_barriers(&mut self, pos: Pos) {
        let barrier = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:barrier");
        for at in self.structure_boundary(pos) {
            if Some(self.common.world.block(at.0, at.1, at.2)) == barrier {
                self.set_test_block_state(at, "minecraft:air", flags::UPDATE_ALL);
            }
        }
        let _ = self.server_process_level_events();
    }

    /// `clearErrorMarkers`.
    pub(in crate::game::server_side) fn clear_error_markers(&mut self, pos: Pos) {
        if let Some(b) = self.test_instance_mut(pos)
            && !b.error_markers.is_empty()
        {
            b.error_markers.clear();
        }
    }

    /// `resetTest`.
    pub(in crate::game) fn reset_test(&mut self, pos: Pos) {
        self.remove_test_barriers(pos);
        self.clear_error_markers(pos);
        if self.place_test_structure(pos) {
            let name = self.test_name(pos);
            self.server_system_message(
                Text::translate("test_instance_block.reset_success", &[name])
                    .with_color(text::GREEN),
            );
        }
        // `data.withStatus(CLEARED)`: the error message goes too.
        if let Some(b) = self.test_instance_mut(pos) {
            b.data.status = Status::Cleared;
            b.data.error_message = None;
        }
    }

    /// `placeStructure()`: the test's structure, cleared space first.
    pub(in crate::game::server_side) fn place_test_structure(&mut self, pos: Pos) -> bool {
        let Some(def) = self.test_holder(pos) else {
            return false;
        };
        let Some(entry) = self.template_entry(&def.structure) else {
            return false;
        };
        let rotation = self.test_rotation(pos);
        let padding = def.padding;
        let Some(block) = self.test_instance_at(pos).cloned() else {
            return false;
        };
        let origin = [pos.0, pos.1, pos.2];
        // `forceLoadChunks`, then `StructureUtils.clearSpaceForStructure(
        // getTestBoundingBox())`.
        if let Some(inner) = self.test_structure_box(pos) {
            self.force_load_test_chunks(&inner);
            let area = BoundingBox::new(
                inner.min.map(|v| v - padding),
                inner.max.map(|v| v + padding),
            );
            self.clear_test_space(&area);
            // removeEntities: `getTestBounds()`, the structure's box inflated.
            self.discard_entities_in(inflate(aabb_of(&inner), f64::from(padding)));
        }
        let corner = block.start_corner(origin, padding, rotation);
        let settings = TemplateSettings {
            mirror: Mirror::None,
            rotation,
            pivot: [0; 3],
            ignore_entities: block.data.ignore_entities,
            known_shape: true,
            integrity: 1.0,
            seed: create_seed(0),
            flags: 818,
        };
        self.place_template_entry(
            &entry,
            BlockPos::new(corner[0], corner[1], corner[2]),
            settings,
            Mirror::None,
            rotation,
        );
        true
    }

    /// `StructureUtils.clearSpaceForStructure`'s blocks: air in the box (stone
    /// below the ground height, which no row of the box is).
    pub(in crate::game) fn clear_test_space(&mut self, area: &BoundingBox) {
        let ground = area.min[1] - 1;
        for y in area.min[1]..=area.max[1] {
            for z in area.min[2]..=area.max[2] {
                for x in area.min[0]..=area.max[0] {
                    let name = if y < ground {
                        "minecraft:stone"
                    } else {
                        "minecraft:air"
                    };
                    self.set_test_block_state((x, y, z), name, 818);
                }
            }
        }
        // `getBlockTicks().clearArea`, `clearBlockEvents`.
        self.clear_test_ticks(area);
        // The entities but the players.
        self.discard_entities_in(aabb_of(area));
    }

    /// `getBlockTicks().clearArea(bounds)` and `clearBlockEvents(bounds)`.
    pub(in crate::game::server_side) fn clear_test_ticks(&mut self, bounds: &BoundingBox) {
        let area = TickArea::new(
            BlockPos::new(bounds.min[0], bounds.min[1], bounds.min[2]),
            BlockPos::new(bounds.max[0], bounds.max[1], bounds.max[2]),
        );
        self.common.block_ticks.blocks.clear_area(area);
        self.common.block_ticks.retain_block_events(|e| {
            !(bounds.min[0]..=bounds.max[0]).contains(&e.pos.x)
                || !(bounds.min[1]..=bounds.max[1]).contains(&e.pos.y)
                || !(bounds.min[2]..=bounds.max[2]).contains(&e.pos.z)
        });
    }

    /// `saveTest`: the test's structure id, saved from the world.
    pub(in crate::game::server_side) fn save_test(&mut self, pos: Pos) -> Option<String> {
        let id = self
            .test_holder(pos)
            .map(|t| t.structure)
            .or_else(|| self.test_instance_at(pos).and_then(|b| b.data.test.clone()));
        let Some(id) = id else {
            self.server_system_message(red(
                "test_instance_block.error.unable_to_save",
                &[pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string())),
            ));
            return None;
        };
        let block = self.test_instance_at(pos)?.clone();
        let padding = self.test_holder(pos).map_or(0, |t| t.padding);
        let corner = TestInstanceBlock::structure_pos_of(
            [0, 1, 2].map(|i| [pos.0, pos.1, pos.2][i] + padding),
        );
        self.save_structure_area(
            &id,
            corner,
            block.data.size,
            block.data.ignore_entities,
            "",
            true,
            true,
        );
        Some(id)
    }

    /// `exportTest`: saves, then writes to the test template folder, which
    /// only a development build has.
    pub(in crate::game::server_side) fn export_test(&mut self, pos: Pos) {
        if self.save_test(pos).is_some() {
            self.server_system_message(
                Text::literal("Test structure exporting is disabled").with_color(text::RED),
            );
        }
    }

    /// `runTest`: the checks, the structure's placement and the test.
    pub(in crate::game::server_side) fn run_test(&mut self, pos: Pos) {
        let coordinates = [pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string()));
        let Some(def) = self.test_holder(pos) else {
            self.server_system_message(red("test_instance_block.error.no_test", &coordinates));
            return;
        };
        if !self.place_test_structure(pos) {
            self.server_system_message(red(
                "test_instance_block.error.no_test_structure",
                &coordinates,
            ));
            return;
        }
        self.clear_error_markers(pos);
        // GameTestTicker.SINGLETON.clear(), FailedTestTracker.forgetFailedTests().
        self.clear_game_tests();
        self.server.game_tests.0.failed.clear();
        let Some(block) = self.test_instance_at(pos) else {
            return;
        };
        let key = block.data.test.clone().unwrap_or_default();
        let rotation = block.data.rotation;
        let key = if key.contains(':') {
            key
        } else {
            format!("minecraft:{key}")
        };
        self.server_system_message(Text::translate(
            "test_instance_block.starting",
            &[Text::literal(key.clone())],
        ));
        let id = self.server.game_tests.0.next_id();
        let info = Info::new(id, key, def, Some(pos), self.common.dim, rotation);
        self.track_and_start_runner(vec![info], false, None, false);
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
