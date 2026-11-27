//! Maps (`EmptyMapItem`, `MapItem`, `MapItemSavedData`, `MapRenderer`):
//! creating a map centred on the player's grid cell, revealing it while held
//! (`MapItem.update`: dominant map colour per pixel, height and water-depth
//! shading, the dithered edge), the player marker, `data/minecraft/maps/<id>.dat` and
//! `maps/last_id.dat`, the 128x128 texture in an atlas slot, the two-handed
//! first-person view and maps in item frames.
mod part_color;
use crate::game::ServerGame;
use sunrise_core::{nbt::Tag, position::BlockPos};
use sunrise_gameplay_common::game::maps::{
    Framed, Key, MapData, PLAYER, StaticDecoration, decoration_sprite, map_id,
};
use sunrise_items::{blocks::Fluid, items::ItemStack};

/// The host among a map's holders (the guests are their connections).
pub(in crate::game) const HOST: u32 = u32::MAX;
/// What a read of a map gives: from the cache when the map is loaded, else from its saved file.
pub(in crate::game::server_side) fn map_read<T>(
    cached: Option<&MapData>,
    saved: impl FnOnce() -> Option<MapData>,
    read: impl FnOnce(&MapData) -> T,
) -> Option<T> {
    match cached {
        Some(map) => Some(read(map)),
        None => saved().map(|map| read(&map)),
    }
}
/// `minecraft:map_decorations` of a map item, in the component's order.
pub(in crate::game::server_side) fn static_decorations(stack: &ItemStack) -> Vec<StaticDecoration> {
    let Some(Tag::Compound(entries)) = stack.data().and_then(|d| {
        d.preserved_components()
            .iter()
            .find(|(k, _)| k == "minecraft:map_decorations")
            .map(|(_, t)| t)
    }) else {
        return Vec::new();
    };
    let number = |t: Option<&Tag>| match t {
        Some(Tag::Double(b)) => Some(f64::from_bits(*b)),
        Some(Tag::Float(b)) => Some(f64::from(f32::from_bits(*b))),
        Some(Tag::Int(v)) => Some(f64::from(*v)),
        _ => None,
    };
    entries
        .iter()
        .filter_map(|(key, e)| {
            let sprite = match e.get("type") {
                Some(Tag::String(s)) => decoration_sprite(&s.to_string_lossy())?,
                _ => return None,
            };
            Some((
                key.clone(),
                sprite,
                number(e.get("x"))?,
                number(e.get("z"))?,
                number(e.get("rotation")).unwrap_or(0.0) as f32,
            ))
        })
        .collect()
}
/// `MapItemSavedData.addTargetDecoration`: the `"+"` entry of
/// `minecraft:map_decorations`, rotation 180.
pub(in crate::game::server_side) fn target_decoration(decoration: &str, at: [i32; 3]) -> Tag {
    Tag::compound([(
        "+",
        Tag::compound([
            ("type", Tag::String(decoration.into())),
            ("x", Tag::Double(f64::from(at[0]).to_bits())),
            ("z", Tag::Double(f64::from(at[2]).to_bits())),
            ("rotation", Tag::Float(180.0f32.to_bits())),
        ]),
    )])
}
impl ServerGame<'_> {
    pub(in crate::game) fn dimension_id(&self) -> &'static str {
        self.common.dim.id()
    }
    pub(in crate::game::server_side) fn map_path(&self, name: &str) -> std::path::PathBuf {
        self.server
            .world_root
            .join("data/minecraft/maps")
            .join(name)
    }
    /// `f` of map `id`'s data: the loaded map, else its `.dat` read without keeping it, so a
    /// read needs no `&mut`. None for a map id nothing made.
    pub(in crate::game::server_side) fn with_map<R>(
        &self,
        id: i32,
        f: impl FnOnce(&MapData) -> R,
    ) -> Option<R> {
        map_read(
            self.common
                .transactions
                .iter()
                .rev()
                .find_map(|level| level.new_maps.get(&id))
                .or_else(|| self.common.maps.data.get(&id)),
            || {
                if self.common.maps.missing.contains(&id) {
                    return None;
                }
                sunrise_sim::save::read_dat(&self.map_path(&format!("{id}.dat")))
                    .and_then(|saved| MapData::from_nbt(&saved))
            },
            f,
        )
    }
    /// The map's data, including new maps in the open transaction. Committed maps are
    /// loaded from `data/minecraft/maps/<id>.dat` on first use.
    pub fn map(&mut self, id: i32) -> Option<&mut MapData> {
        if let Some(data) = self
            .common
            .transactions
            .iter_mut()
            .rev()
            .find_map(|level| level.new_maps.get_mut(&id))
        {
            return Some(data);
        }
        if !self.common.maps.data.contains_key(&id) && !self.common.maps.missing.contains(&id) {
            let path = self
                .server
                .world_root
                .join(format!("data/minecraft/maps/{id}.dat"));
            let loaded = sunrise_sim::save::read_dat(&path).and_then(|t| MapData::from_nbt(&t));
            match loaded {
                Some(m) => {
                    self.common.maps.data.insert(id, m);
                }
                None => {
                    self.common.maps.missing.insert(id);
                }
            }
        }
        self.common.maps.data.get_mut(&id)
    }
    /// The next ID in this save, after any map reserved by an open transaction.
    /// Peeking must not dirty the persisted counter, even on a save with no maps.
    pub fn peek_map_id(&self) -> i32 {
        if let Some(id) = self
            .common
            .transactions
            .iter()
            .rev()
            .find_map(|level| level.new_maps.last_key_value().map(|(&id, _)| id))
        {
            return id + 1;
        }
        self.common.maps.last_id.unwrap_or_else(|| {
            sunrise_sim::save::read_dat(&self.map_path("last_id.dat"))
                .and_then(|t| match t.get("data")?.get("map")? {
                    Tag::Int(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(-1)
        }) + 1
    }

    /// Register a reserved ID; only committed data and the committed counter are saveable.
    pub(in crate::game::server_side) fn publish_map(&mut self, id: i32, data: MapData) {
        self.common.maps.last_id = Some(self.common.maps.last_id.map_or(id, |last| last.max(id)));
        self.common.maps.data.insert(id, data);
    }

    pub(in crate::game::server_side) fn store_new_map(&mut self, id: i32, data: MapData) {
        if let Some(level) = self.common.transactions.last_mut() {
            level.new_maps.insert(id, data);
        } else {
            self.publish_map(id, data);
        }
    }
    /// Maps made by exploration map trades: (id, target, zoom, decoration).
    pub fn register_exploration_maps(&mut self, made: Vec<(i32, [i32; 3], u8, String)>) {
        let dimension = self.dimension_id().to_owned();
        for (id, at, zoom, _) in made {
            // MapItem.applyNewSavedData: tracking, unlimited tracking.
            let mut data = MapData::fresh(f64::from(at[0]), f64::from(at[2]), zoom, &dimension);
            data.unlimited = true;
            self.store_new_map(id, data);
            self.render_biome_preview(id);
        }
    }
    /// `ExplorationMapFunction.run` for an item a generated chest still holds
    /// unsearched (`structure_blocks::take_pending_map`): the nearest
    /// structure of the tag around `origin`, a new map centred on it with its
    /// biome preview and target decoration. False when nothing is found: the
    /// loot table's `filtered` map_id check would have discarded the item.
    pub fn resolve_pending_map(&mut self, item: &mut ItemStack, origin: [f64; 3]) -> bool {
        let Some(map) = sunrise_sim::structure_blocks::take_pending_map(item) else {
            return true;
        };
        self.materialize_exploration_map(item, &map, origin)
    }
    /// The existing exploration-map materialization, shared with command modifiers.
    pub(in crate::game::server_side) fn materialize_exploration_map(
        &mut self,
        item: &mut ItemStack,
        map: &sunrise_world::loot::ExplorationMap,
        origin: [f64; 3],
    ) -> bool {
        let names = self.structure_tag(&map.destination);
        let from = origin.map(|v| v.floor() as i32);
        let found = self
            .common
            .end
            .locator()
            .filter(|_| !names.is_empty())
            .and_then(|t| {
                t.locate_for_map(&names, from, map.search_radius, map.skip_existing_chunks)
            });
        let Some((_, at)) = found else {
            return false;
        };
        let mut data = MapData::fresh(
            f64::from(at[0]),
            f64::from(at[2]),
            map.zoom,
            self.dimension_id(),
        );
        data.unlimited = true;
        let id = self.new_map_id(data);
        self.render_biome_preview(id);
        let d = item.data_mut();
        d.map_id = Some(id);
        d.set_other(
            "minecraft:map_decorations",
            Some(target_decoration(&map.decoration, at)),
        );
        true
    }
    /// The unsearched exploration maps of a container being opened
    /// (`RandomizableContainer.unpackLootTable` runs the function then).
    pub fn resolve_container_maps(&mut self, key: (i32, i32, i32), origin: [f64; 3]) {
        use sunrise_level::containers::BlockEntity;
        let Some(BlockEntity::Chest(slots)) = self.common.world.block_entities.get(&key) else {
            return;
        };
        let pending: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.as_ref()
                    .is_some_and(sunrise_sim::structure_blocks::has_pending_map)
            })
            .map(|(i, _)| i)
            .collect();
        for i in pending {
            let Some(BlockEntity::Chest(slots)) = self.common.world.block_entities.get_mut(&key)
            else {
                return;
            };
            let Some(mut item) = slots[i].take() else {
                continue;
            };
            let keep = self.resolve_pending_map(&mut item, origin);
            if let Some(BlockEntity::Chest(slots)) = self.common.world.block_entities.get_mut(&key)
            {
                slots[i] = keep.then_some(item);
            }
        }
    }
    /// `MapItem.renderBiomePreviewMap`: the map's biomes at sea level, water
    /// outlines (`#water_on_map_outlines`) drawn in orange and brown.
    pub(in crate::game::server_side) fn render_biome_preview(&mut self, id: i32) {
        let Some(terrain) = self.common.end.locator() else {
            return;
        };
        let dim = self.dimension_id();
        let zoom = self.server.streaming.biome_zoom;
        let watery_biomes: std::collections::HashSet<String> = self
            .tag_entries("worldgen/biome", "minecraft:water_on_map_outlines")
            .into_iter()
            .collect();
        let Some((scale, center)) = self
            .with_map(id, |map| {
                (map.dimension == dim).then_some((1i32 << map.scale, map.center))
            })
            .flatten()
        else {
            return;
        };
        let start_x = center[0] / scale - 64;
        let start_z = center[1] / scale - 64;
        let sea_level = sunrise_gameplay_common::game::Dim::sea_level_of_id(dim);
        let quarts: Vec<[i32; 3]> = (0..128)
            .flat_map(|row| {
                (0..128).map(move |column| {
                    zoom.quart_at_block(sunrise_core::position::BlockPos::new(
                        (start_x + column) * scale,
                        sea_level,
                        (start_z + row) * scale,
                    ))
                })
            })
            .collect();
        let watery: Vec<bool> = terrain
            .noise_biomes(&quarts)
            .into_iter()
            .map(|b| b.is_some_and(|b| watery_biomes.contains(&b)))
            .collect();
        let is_watery = |x: i32, z: i32| watery[(z * 128 + x) as usize];
        // MapColor ids and MapColor.Brightness ids.
        const ORANGE: u8 = 15;
        const BROWN: u8 = 26;
        const LOW: u8 = 0;
        const NORMAL: u8 = 1;
        const HIGH: u8 = 2;
        const LOWEST: u8 = 3;
        let Some(m) = self.map(id) else {
            return;
        };
        for mx in 1..127 {
            for mz in 1..127 {
                let mut water_count = 0;
                for dx in -1..2 {
                    for dz in -1..2 {
                        if (dx != 0 || dz != 0) && is_watery(mx + dx, mz + dz) {
                            water_count += 1;
                        }
                    }
                }
                let mut brightness = LOWEST;
                let mut color = 0;
                if is_watery(mx, mz) {
                    color = ORANGE;
                    if water_count > 7 && mz % 2 == 0 {
                        let sin = sunrise_core::geometry::trig::sin(f64::from(mz as f32 + 0.0));
                        match (mx + (sin * 7.0) as i32) / 8 % 5 {
                            0 | 4 => brightness = LOW,
                            1 | 3 => brightness = NORMAL,
                            2 => brightness = HIGH,
                            _ => {}
                        }
                    } else if water_count > 7 {
                        color = 0;
                    } else if water_count > 5 {
                        brightness = NORMAL;
                    } else if water_count > 1 {
                        brightness = LOW;
                    }
                } else if water_count > 0 {
                    color = BROWN;
                    brightness = if water_count > 3 { NORMAL } else { LOWEST };
                }
                if color != 0 {
                    m.update_color(mx, mz, color << 2 | brightness);
                }
            }
        }
    }
    /// Registers `data` under a free map id.
    pub fn new_map_id(&mut self, data: MapData) -> i32 {
        let id = self.peek_map_id();
        self.store_new_map(id, data);
        id
    }
    /// `EmptyMapItem.use`: a new map centred around the player.
    pub fn use_empty_map(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:map" {
            return false;
        }
        let Some(p) = self.common.player.as_ref().map(|p| p.position) else {
            return false;
        };
        let id = self.new_map_id(MapData::fresh(p.x, p.z, 0, self.dimension_id()));
        let mut filled = ItemStack::new("minecraft:filled_map", 1);
        filled.data_mut().map_id = Some(id);
        if self.common.creative {
            self.give_or_drop_stack(filled);
        } else if stack.count == 1 {
            *self.common.held_mut() = Some(filled);
        } else {
            self.consume_held();
            self.give_or_drop_stack(filled);
        }
        // EmptyMapItem.use: consumed, then counted.
        self.award_stat("used", "minecraft:map", 1);
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_map_filled(player, &name, id);
        }
        self.server_play_player_bound_sound(
            "minecraft:ui.cartography_table.take_result",
            7,
            1.0,
            1.0,
            false,
        );
        self.swing_arm();
        true
    }
    /// `ItemFrame.removeFramedMap`: the map `frame` holds forgets the frame.
    pub(in crate::game::server_side) fn remove_framed_map(
        &mut self,
        frame: &sunrise_sim::entities::ItemFrame,
    ) {
        let Some(id) = frame.item.as_ref().and_then(map_id) else {
            return;
        };
        if let Some(m) = self.map(id) {
            m.removed_from_frame(frame.pos, frame.id as i32);
        }
    }
    /// `MapItem.useOn`: a map used on a banner toggles that banner's marker
    /// on the map; when the map's data says no (`toggleBanner` fails) the use
    /// ends without a swing, and nothing else is done with the map either way.
    pub fn use_map_on_banner(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
        before: &ItemStack,
    ) -> bool {
        let Some(id) = map_id(stack) else {
            return false;
        };
        if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
            return false;
        }
        let [x, y, z] = hit.pos;
        let block = self
            .common
            .res
            .facts()
            .get(self.common.world.block(x, y, z))
            .name
            .clone();
        if !self.named_block_in_tag("minecraft:banners", &block) {
            return false;
        }
        let current = self.map_banner_at(hit.pos);
        let game_time = self.common.world.game_time;
        let failed = self
            .map(id)
            .is_some_and(|m| !m.toggle_banner(hit.pos, current, game_time));
        if failed {
            return true;
        }
        self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    /// `Inventory.tick` (and `EntityEquipment.tick` for the armour) ->
    /// `CompassItem.inventoryTick`: each compass the player carries whose
    /// lodestone is gone loses its target (`inventory_tick`).
    pub fn tick_lodestone_compasses(&mut self) {
        let (world, facts, dim) = (
            &self.common.world,
            self.common.res.facts(),
            self.common.dim.id(),
        );
        let stacks = self
            .common
            .inventory
            .iter_mut()
            .chain([&mut self.common.offhand]);
        for stack in stacks.chain(self.common.armor.iter_mut()).flatten() {
            super::inventory_tick::compass_inventory_tick(world, facts, dim, stack);
        }
    }
    pub(in crate::game::server_side) fn give_or_drop_stack(&mut self, stack: ItemStack) {
        if let Some(rest) = self.add_stack_to_inventory(stack)
            && let Some(eye) = self.common.eye_position()
        {
            self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
        }
    }
    /// `MapItemSavedData.hasMapInvisibilityItemEquipped`: a piece of
    /// `#map_invisibility_equipment` (a carved pumpkin) in an armor slot; the
    /// hands do not count.
    pub(in crate::game::server_side) fn map_invisibility_worn(
        &self,
        worn: &[Option<ItemStack>],
    ) -> bool {
        let tag = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:map_invisibility_equipment");
        worn.iter()
            .flatten()
            .any(|s| tag.is_some_and(|t| t.contains(&*s.item)))
    }
    /// Held maps update; every carried map tracks the player marker.
    pub(in crate::game::server_side) fn tick_maps(&mut self) {
        let Some(p) = self.common.player.as_ref().map(|p| p.position) else {
            return;
        };
        // A generated exploration map that reached the inventory unsearched
        // (moved out of its chest by a hopper): searched from here.
        for i in 0..self.common.inventory.len() {
            if self.common.inventory[i]
                .as_ref()
                .is_some_and(sunrise_sim::structure_blocks::has_pending_map)
                && let Some(mut item) = self.common.inventory[i].take()
            {
                let keep = self.resolve_pending_map(&mut item, [p.x, p.y, p.z]);
                self.common.inventory[i] = keep.then_some(item);
            }
        }
        let held: Vec<i32> = [
            Some(self.common.selected),
            Some(super::shield::OFFHAND_SLOT),
        ]
        .into_iter()
        .flatten()
        .filter_map(|i| self.common.stack_in(i).and_then(map_id))
        .collect();
        let carried: Vec<(i32, Vec<StaticDecoration>)> = self
            .common
            .inventory
            .iter()
            .chain(std::iter::once(&self.common.offhand))
            .flatten()
            .filter_map(|s| Some((map_id(s)?, static_decorations(s))))
            .collect();
        let (yaw, dim) = (self.common.yaw, self.dimension_id());
        let game_time = self.common.world.game_time;
        let invisible = self.map_invisibility_worn(&self.common.armor);
        // The guest whose turn it is, else the host.
        let me = self.server.lan_acting.unwrap_or(HOST);
        for (id, targets) in carried {
            let Some(m) = self.map(id) else { continue };
            let h = m.holder(me);
            h.carried = game_time;
            h.invisible = invisible;
            m.drop_holders(game_time);
            // tickCarriedBy: the player marker while tracking in this dimension.
            if m.tracking && m.dimension == dim {
                m.add_decoration(
                    PLAYER,
                    Some(game_time),
                    Key::Player(me),
                    p.x,
                    p.z,
                    f64::from(yaw),
                    None,
                );
            } else if m.decorations.iter().any(|d| d.key == Key::Player(me)) {
                m.remove_decoration(Key::Player(me));
            }
            m.hide_invisible(me);
            m.add_item_decorations(targets, game_time);
        }
        for id in held {
            self.update_map(id);
        }
    }
    /// `ServerPlayer.synchronizeSpecialItemUpdates` for each map the guest
    /// carries, and every 10 ticks (`ServerEntity.sendChanges`) for each map
    /// framed in the level: a `ClientboundMapItemDataPacket` when something
    /// changed since the guest's last one.
    pub(in crate::game::server_side) fn lan_map_updates(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let mut ids: Vec<i32> = g
            .inventory
            .iter()
            .chain(std::iter::once(&g.offhand))
            .flatten()
            .filter_map(map_id)
            .collect();
        let now = self.common.world.game_time;
        if now % 10 == 0 && host.hub.here(g) {
            let framed: Vec<(i32, Framed, bool)> = self
                .common
                .item_frames
                .iter()
                // ServerEntity.sendChanges: only a framed MapItem (a filled
                // map), not any stack that carries a map id.
                .filter_map(|f| {
                    let item = f.item.as_ref()?;
                    if &*item.item != "minecraft:filled_map" {
                        return None;
                    }
                    let frame = Framed {
                        pos: f.pos,
                        facing: f.facing,
                        id: f.id as i32,
                    };
                    let id = map_id(item)?;
                    let carries = g
                        .inventory
                        .iter()
                        .chain(std::iter::once(&g.offhand))
                        .flatten()
                        .any(|s| s.item == item.item && map_id(s) == Some(id));
                    Some((id, frame, carries))
                })
                .collect();
            let invisible = self.map_invisibility_worn(&g.armor);
            for (id, frame, carries) in framed {
                if let Some(m) = self.map(id) {
                    m.tick_framed(conn, now, carries, invisible, frame);
                    ids.push(id);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let kind = |name: &str| {
            host.hub.registry_id(
                "minecraft:map_decoration_type",
                &format!("minecraft:{name}"),
            )
        };
        for id in ids {
            if let Some(p) = self
                .common
                .maps
                .data
                .get_mut(&id)
                .and_then(|m| m.update_packet(id, conn, kind))
            {
                g.send(&sunrise_protocol::packets::play::Clientbound::MapItemData(
                    p,
                ));
            }
        }
    }
    /// `MapItemSavedData.checkBanners` on map `id` for the column `x`, `z`
    /// that `MapItem.update` is reading.
    pub(in crate::game::server_side) fn check_banners(&mut self, id: i32, x: i32, z: i32) {
        let (world, res) = (&self.common.world, &self.common.res);
        let Some(m) = self.common.maps.data.get_mut(&id) else {
            return;
        };
        if m.banners.is_empty() {
            return;
        }
        m.check_banners(x, z, |pos| {
            let state = world.block(pos[0], pos[1], pos[2]);
            sunrise_gameplay_common::game::banners::map_banner(
                pos,
                world.block_entities.get(&(pos[0], pos[1], pos[2])),
                &res.facts().get(state).name,
            )
        });
    }
    /// `MapItem.update` for the player holding map `id`.
    pub(in crate::game::server_side) fn update_map(&mut self, id: i32) {
        let dim = self.dimension_id();
        let Some(p) = self.common.player.as_ref().map(|p| p.position) else {
            return;
        };
        let has_ceiling = self.common.world.shape.has_ceiling;
        let Some(m) = self.common.maps.data.get_mut(&id) else {
            return;
        };
        if m.locked || m.dimension != dim {
            return;
        }
        let scale = 1i32 << m.scale;
        let [cx, cz] = m.center;
        let px = (p.x - f64::from(cx)).floor() as i32 / scale + 64;
        let pz = (p.z - f64::from(cz)).floor() as i32 / scale + 64;
        let mut radius = 128 / scale;
        if has_ceiling {
            radius /= 2;
        }
        let h = m.holder(self.server.lan_acting.unwrap_or(HOST));
        h.step += 1;
        let step = h.step;
        let mut changes: Vec<(i32, i32, u8)> = Vec::new();
        let mut consecutive = false;
        for img_x in px - radius + 1..px + radius {
            if (img_x & 15) != (step & 15) && !consecutive {
                continue;
            }
            consecutive = false;
            let mut previous_height = 0.0f64;
            for img_y in pz - radius - 1..pz + radius {
                if !(0..128).contains(&img_x) || !(-1..128).contains(&img_y) {
                    continue;
                }
                let d2 = (img_x - px).pow(2) + (img_y - pz).pow(2);
                let dither_black = d2 > (radius - 2) * (radius - 2);
                let min_x = (cx / scale + img_x - 64) * scale;
                let min_z = (cz / scale + img_y - 64) * scale;
                if !self
                    .common
                    .world
                    .chunks
                    .contains_key(&(min_x >> 4, min_z >> 4))
                {
                    continue;
                }
                let mut counts: Vec<(u8, u32)> = Vec::new();
                let mut add = |c: u8, n: u32| match counts.iter_mut().find(|e| e.0 == c) {
                    Some(e) => e.1 += n,
                    None => counts.push((c, n)),
                };
                let mut water_depth = 0i32;
                let mut height = 0.0f64;
                if has_ceiling {
                    let mut noise = min_x.wrapping_add(min_z.wrapping_mul(231_871));
                    noise = noise
                        .wrapping_mul(noise)
                        .wrapping_mul(31_287_121)
                        .wrapping_add(noise.wrapping_mul(11));
                    // Dirt (10) or stone (11) noise under a ceiling.
                    if noise >> 20 & 1 == 0 {
                        add(
                            self.default_map_color("minecraft:dirt", 10, BlockPos::new(0, 0, 0)),
                            10,
                        );
                    } else {
                        add(
                            self.default_map_color("minecraft:stone", 11, BlockPos::new(0, 0, 0)),
                            100,
                        );
                    }
                    height = 100.0;
                } else {
                    for dx in 0..scale {
                        for dz in 0..scale {
                            let (x, z) = (min_x + dx, min_z + dz);
                            let mut y = self.world_surface(x, z);
                            let min_y = self.common.world.bottom_y();
                            let color = if y <= min_y {
                                self.default_map_color(
                                    "minecraft:bedrock",
                                    11,
                                    BlockPos::new(x, 0, z),
                                )
                            } else {
                                let mut state;
                                loop {
                                    y -= 1;
                                    state = self.common.world.block(x, y, z);
                                    if self.map_color_at(BlockPos::new(x, y, z), state) != 0
                                        || y <= min_y
                                    {
                                        break;
                                    }
                                }
                                let f = self.common.res.facts().get(state);
                                if y > min_y && f.fluid != Fluid::None {
                                    let mut solid = y - 1;
                                    loop {
                                        let below = self
                                            .common
                                            .res
                                            .facts()
                                            .get(self.common.world.block(x, solid, z));
                                        solid -= 1;
                                        water_depth += 1;
                                        if solid <= min_y || below.fluid == Fluid::None {
                                            break;
                                        }
                                    }
                                    // getCorrectStateForFluidBlock: the fluid unless sturdy on top.
                                    if f.sturdy_full >> 1 & 1 == 0 {
                                        let b = self.common.res.behaviours();
                                        state = b.legacy_block(b.fluid(state));
                                    }
                                }
                                self.map_color_at(BlockPos::new(x, y, z), state)
                            };
                            self.check_banners(id, x, z);
                            height += f64::from(y) / f64::from(scale * scale);
                            add(color, 1);
                        }
                    }
                }
                water_depth /= scale * scale;
                // Multisets.copyHighestCountFirst: the most common, first seen on ties.
                let best = counts
                    .iter()
                    .fold(None::<(u8, u32)>, |acc, &(c, n)| match acc {
                        Some((_, bn)) if bn >= n => acc,
                        _ => Some((c, n)),
                    })
                    .map_or(0, |e| e.0);
                let parity = f64::from((img_x + img_y) & 1);
                let brightness = if best == 12 {
                    let diff = f64::from(water_depth) * 0.1 + parity * 0.2;
                    if diff < 0.5 {
                        2
                    } else if diff > 0.9 {
                        0
                    } else {
                        1
                    }
                } else {
                    let diff = (height - previous_height) * 4.0 / f64::from(scale + 4)
                        + (parity - 0.5) * 0.4;
                    if diff > 0.6 {
                        2
                    } else if diff < -0.6 {
                        0
                    } else {
                        1
                    }
                };
                previous_height = height;
                if img_y >= 0 && d2 < radius * radius && (!dither_black || (img_x + img_y) & 1 != 0)
                {
                    changes.push((img_x, img_y, best << 2 | brightness));
                }
            }
            if let Some(m) = self.common.maps.data.get_mut(&id) {
                for (x, y, c) in changes.drain(..) {
                    consecutive |= m.update_color(x, y, c);
                }
            }
        }
    }
    /// Writes changed maps and the id counter (`MapItemSavedData`, `MapIndex`).
    pub fn save_maps(&mut self) {
        for (id, m) in &mut self.common.maps.data {
            if m.dirty {
                let path = self
                    .server
                    .world_root
                    .join("data/minecraft/maps")
                    .join(format!("{id}.dat"));
                if sunrise_sim::save::write_dat(&path, m.to_nbt()).is_ok() {
                    m.dirty = false;
                }
            }
        }
        if let Some(last) = self.common.maps.last_id {
            let tag = Tag::compound([
                ("data", Tag::compound([("map", Tag::Int(last))])),
                ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ]);
            if let Err(e) = sunrise_sim::save::write_dat(
                &self
                    .server
                    .world_root
                    .join("data/minecraft/maps/last_id.dat"),
                tag,
            ) {
                log::warn!("Sunrise: the next map id is not saved: {e}");
            }
        }
    }
}

impl ServerGame<'_> {
    /// `CompassItem.useOn`: a compass used on a lodestone locks onto it (a
    /// single one in survival changes in place, else one is split off).
    pub(in crate::game::server_side) fn use_lodestone_compass(
        &mut self,
        hit: &super::Hit,
        stack: &ItemStack,
        before: &ItemStack,
    ) -> bool {
        if &*stack.item != "minecraft:compass" {
            return false;
        }
        let [x, y, z] = hit.pos;
        if self
            .common
            .res
            .facts()
            .get(self.common.world.block(x, y, z))
            .name
            .as_ref()
            != "minecraft:lodestone"
        {
            return false;
        }
        let at = [x, y, z].map(|v| f64::from(v) + 0.5);
        self.trigger_block("minecraft:item_used_on_block", hit.pos, before);
        self.server_play_sound("minecraft:item.lodestone_compass.lock", at, 1.0, 1.0);
        let target = Tag::compound([
            (
                "target",
                Tag::compound([
                    ("dimension", Tag::String(self.dimension_id().into())),
                    ("pos", Tag::IntArray(vec![x, y, z])),
                ]),
            ),
            ("tracked", Tag::Byte(1)),
        ]);
        if !self.common.creative && stack.count == 1 {
            self.edit_block_use_stack(|cell| {
                if let Some(held) = cell {
                    held.data_mut()
                        .set_other("minecraft:lodestone_tracker", Some(target.clone()));
                }
            });
        } else {
            let mut locked = stack.clone();
            locked.count = 1;
            locked
                .data_mut()
                .set_other("minecraft:lodestone_tracker", Some(target));
            self.consume_held();
            self.give_or_drop_stack(locked);
        }
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod native_read_tests;

#[cfg(test)]
pub(in crate::game) mod save_tests;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_core::nbt::JavaString;
    use sunrise_gameplay_common::game::maps::{
        Decoration, clamp_map_coordinate, decoration_pose, decoration_pose_at, decoration_rotation,
        packed_rgba,
    };

    #[test]
    fn packed_map_color_decodes_palette_and_shade() {
        // Grass at each brightness, water at NORMAL, and glow lichen (last ID) at HIGH.
        assert_eq!(packed_rgba(4), [89, 125, 39, 255]);
        assert_eq!(packed_rgba(5), [109, 153, 48, 255]);
        assert_eq!(packed_rgba(6), [127, 178, 56, 255]);
        assert_eq!(packed_rgba(7), [67, 94, 29, 255]);
        assert_eq!(packed_rgba(49), [55, 55, 220, 255]);
        assert_eq!(packed_rgba(246), [127, 167, 150, 255]);
        assert_eq!(packed_rgba(3), [0; 4]);
        assert_eq!(packed_rgba(248), [0; 4]);
        assert_eq!(packed_rgba(255), [0; 4]);
    }

    #[test]
    fn painting_a_map_marks_only_what_changed_and_refuses_another_size() {
        let mut map = MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        map.dirty = false;
        map.texture_dirty = false;
        let mut colors = vec![0; 128 * 128];
        assert_eq!(
            map.paint(&colors),
            Some(false),
            "an unchanged map stays clean"
        );
        assert!(!map.dirty && !map.texture_dirty);
        // x 5, z 2: index 5 + 2 * 128, as the map's rows run.
        colors[5 + 2 * 128] = 34;
        assert_eq!(map.paint(&colors), Some(true));
        assert_eq!(map.colors[5 + 2 * 128], 34);
        assert!(map.dirty && map.texture_dirty, "saved and redrawn");
        assert_eq!(map.paint(&colors[..100]), None);
    }

    #[test]
    fn a_mod_icon_sits_where_a_vanilla_decoration_would() {
        // Half pixels from the map's centre: x 20 is 10 map pixels right of it.
        let vanilla = Decoration {
            sprite: "map/decorations/frame",
            x: 20,
            y: -30,
            rot: 4,
            name: None,
            key: Key::Remote,
        };
        assert_eq!(
            decoration_pose(&vanilla, -0.02),
            decoration_pose_at(20, -30, 4, -0.02)
        );
        let at = decoration_pose_at(20, -30, 0, 0.0).transform_point3(glam::Vec3::ZERO);
        assert!(
            (at.x - 74.0).abs() < 0.6 && (at.y - 49.0).abs() < 0.6,
            "{at:?}"
        );
    }

    #[test]
    fn every_target_decoration_type_has_its_sprite() {
        // MapDecorationTypes: (name, asset).
        for (name, asset) in [
            ("mansion", "woodland_mansion"),
            ("monument", "ocean_monument"),
            ("red_x", "red_x"),
            ("village_desert", "desert_village"),
            ("village_plains", "plains_village"),
            ("village_savanna", "savanna_village"),
            ("village_snowy", "snowy_village"),
            ("village_taiga", "taiga_village"),
            ("jungle_temple", "jungle_temple"),
            ("swamp_hut", "swamp_hut"),
            ("trial_chambers", "trial_chambers"),
            ("abandoned_camp", "abandoned_camp"),
            ("ancient_city", "ancient_city"),
            ("desert_pyramid", "desert_pyramid"),
            ("mineshaft", "mineshaft"),
            ("ocean_ruin_warm", "warm_ocean_ruins"),
        ] {
            assert_eq!(
                decoration_sprite(&format!("minecraft:{name}")),
                Some(format!("map/decorations/{asset}").as_str()),
                "{name}"
            );
        }
        assert_eq!(decoration_sprite("minecraft:nothing"), None);
    }

    #[test]
    fn target_decorations_read_back_with_vanilla_rotation() {
        let mut stack = ItemStack::new("minecraft:buried_treasure_map", 1);
        stack.data_mut().map_id = Some(3);
        stack.data_mut().set_other(
            "minecraft:map_decorations",
            Some(target_decoration("minecraft:red_x", [120, 40, -300])),
        );
        assert_eq!(map_id(&stack), Some(3));
        let found = static_decorations(&stack);
        assert_eq!(
            found,
            vec![("+".into(), "map/decorations/red_x", 120.0, -300.0, 180.0)]
        );
        // calculateRotation(180): (byte) (188 * 16 / 360) = 8.
        assert_eq!(decoration_rotation(false, 0, 180.0), 8);
        assert_eq!(decoration_rotation(false, 0, -90.0), 12);
        assert_eq!(clamp_map_coordinate(-70.0), -128);
        assert_eq!(clamp_map_coordinate(10.25), 21);
    }
    #[test]
    fn an_undecorated_copy_does_not_erase_the_treasure_target() {
        let mut map = MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        let mut treasure = ItemStack::new("minecraft:filled_map", 1);
        treasure.data_mut().map_id = Some(7);
        treasure.data_mut().set_other(
            "minecraft:map_decorations",
            Some(target_decoration("minecraft:red_x", [20, 40, 30])),
        );
        let mut copy = ItemStack::new("minecraft:filled_map", 1);
        copy.data_mut().map_id = Some(7);
        map.add_item_decorations(static_decorations(&treasure), 0);
        map.add_item_decorations(static_decorations(&copy), 0);
        assert_eq!(map.decorations.len(), 1);
        assert_eq!(map.decorations[0].sprite, "map/decorations/red_x");
        assert_eq!((map.decorations[0].x, map.decorations[0].y), (40, 60));
    }
    #[test]
    fn named_map_targets_are_added_once_without_replacing_other_targets() {
        let mut map = MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        let target = |key: JavaString, x| (key, "map/decorations/red_x", x, 30.0, 180.0);
        let first_key = JavaString(vec![0xd800]);
        let second_key = JavaString(vec![0xd801]);
        map.add_item_decorations(vec![target(first_key.clone(), 20.0)], 0);
        map.add_item_decorations(vec![target(first_key, 40.0), target(second_key, 25.0)], 1);
        assert_eq!(map.decorations.len(), 2);
        assert_eq!((map.decorations[0].x, map.decorations[1].x), (40, 50));
        // Both entries are off the map: neither may remove an existing target.
        map.add_item_decorations(vec![target("absent".into(), 200.0)], 2);
        assert_eq!(map.decorations.len(), 2);
    }
}
