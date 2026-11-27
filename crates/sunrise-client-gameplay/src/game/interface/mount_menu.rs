//! The horse inventory (`HorseInventoryMenu`, `HorseInventoryScreen`): the
//! saddle and body armour slots (a llama's carpet, a nautilus's armour), the
//! chest of donkeys, mules and llamas and the mount drawn following the
//! cursor. A nautilus has the same two slots and no chest
//! (`NautilusInventoryMenu`). It opens with E while riding a tamed mount or
//! with shift + use on a tamed one. On a
//! server the screen shows the server's menu (`ClientboundMountScreenOpenPacket`)
//! and its clicks go out as container clicks; a server's players get theirs
//! from game/network/lan_mount_menu.rs.
use super::Screen;
use sunrise_client::hud::{HudBatch, rgba};
use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Mob, Species, llama, nautilus};

/// `AbstractChestedHorse.INV_CHEST_COUNT`.
pub(in crate::game) const CHEST_SIZE: usize = 15;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::mount_rules::{
    ARMOR, CHEST, SADDLE, body_armor_fits, has_mount_menu, mount_columns, shows_body, shows_saddle,
};

/// The open mount menu: copies of the mount's slots, written back on every change.
#[derive(Default)]
pub struct MountMenu {
    pub(in crate::game) open: Option<Open>,
}
impl MountMenu {
    /// The mount whose menu is open.
    #[allow(dead_code)]
    pub fn opened_mob(&self) -> Option<u32> {
        self.open.as_ref().map(|o| o.mob)
    }
}
pub(in crate::game) struct Open {
    pub(in crate::game) mob: u32,
    pub(in crate::game) species: Species,
    pub(in crate::game) columns: usize,
    pub(in crate::game) saddle: Option<ItemStack>,
    pub(in crate::game) armor: Option<ItemStack>,
    pub(in crate::game) items: Vec<Option<ItemStack>>,
    /// `shouldRenderSaddleSlot` / `shouldRenderArmorSlot`.
    pub(in crate::game) show_saddle: bool,
    pub(in crate::game) show_armor: bool,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Whether the mount menu is the open screen.
    pub(in crate::game::client_side) fn mount_active(&self) -> bool {
        self.client.screen == Screen::Inventory && self.client.mount_menu.open.is_some()
    }
    pub(in crate::game::client_side) fn mount_mob_index(&self, id: u32) -> Option<usize> {
        self.common.mobs.iter().position(|m| m.id == id)
    }
    /// `AbstractHorse.openCustomInventoryScreen` for a tamed adult mount.
    pub fn open_mount_menu(&mut self, index: usize) {
        let m = &self.common.mobs[index];
        let species = m.species();
        if !has_mount_menu(species) {
            return;
        }
        let mut items = m.extra.items.clone();
        items.resize(CHEST_SIZE, None);
        let show_armor = shows_body(species);
        self.client.mount_menu.open = Some(Open {
            mob: m.id,
            species,
            columns: mount_columns(m),
            saddle: m.saddle_item(),
            armor: show_armor
                .then(|| m.equipment_stacks()[6].clone())
                .flatten(),
            items,
            show_saddle: shows_saddle(species),
            show_armor,
        });
        self.show_mount_menu();
    }
    /// Open the screen for the server's already installed mount window. Its
    /// equipment and chest stacks arrive in the menu's content packets.
    pub(in crate::game::client_side) fn open_remote_mount_menu(
        &mut self,
        entity: u32,
        species: Species,
        items: Vec<Option<ItemStack>>,
    ) {
        let columns = items.len() / 3;
        self.client.mount_menu.open = Some(Open {
            mob: entity,
            species,
            columns,
            saddle: None,
            armor: None,
            items,
            show_saddle: shows_saddle(species),
            show_armor: shows_body(species),
        });
        self.show_mount_menu();
    }
    pub(in crate::game::client_side) fn show_mount_menu(&mut self) {
        if self.common.is_logical_server() {
            self.common.carried = None;
        }
        self.clear_station_client();
        self.common.container = None;
        self.common.craft_size = 2;
        self.set_screen(Screen::Inventory);
        self.capture(false);
    }
    /// Riding a mount that has a horse inventory (`HasCustomInventoryScreen`).
    pub(in crate::game::client_side) fn riding_mount_with_menu(&self) -> bool {
        self.common
            .riding
            .and_then(|id| self.mount_mob_index(id))
            .is_some_and(|i| has_mount_menu(self.common.mobs[i].species()))
    }
    /// A server closed its window: the screen's copy goes.
    pub(in crate::game::client_side) fn mount_menu_reset(&mut self) {
        self.client.mount_menu.open = None;
    }
    /// The entity type of the open mount menu's mount (`minecraft:horse`).
    pub(in crate::game::client_side) fn mount_menu_entity(&self) -> Option<String> {
        let open = self.client.mount_menu.open.as_ref()?;
        let m = self.common.mobs.iter().find(|m| m.id == open.mob)?;
        Some(m.kind.id.to_string())
    }
    /// The chest columns of the open mount menu (the server's window's).
    pub fn mount_menu_columns(&self) -> Option<usize> {
        self.client.mount_menu.open.as_ref().map(|o| o.columns)
    }
    /// The menu ends when the mount dies or its box leaves the entity
    /// interaction range plus 4 of the player's eyes
    /// (`AbstractMountInventoryMenu.stillValid` with
    /// `Player.isWithinEntityInteractionRange(mount, 4.0)`: 7 blocks away, 9
    /// in creative).
    #[allow(dead_code)]
    pub fn tick_mount_menu(&mut self) {
        // A server's menu ends when the server closes it.
        if self.client.remote.is_some() {
            return;
        }
        let Some(open) = self.client.mount_menu.open.as_ref() else {
            return;
        };
        let mob = open.mob;
        let mount = self
            .mount_mob_index(mob)
            .map(|i| (self.common.mobs[i].alive(), self.common.mobs[i].bounds()));
        let valid = match mount {
            Some((true, bounds)) => self.common.within_entity_interaction_range(&bounds, 4.0),
            _ => false,
        };
        if !valid {
            self.close_container_client();
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
    }
    /// Closing the screen clears the client's open mount menu copy.
    pub(in crate::game) fn mount_closed(&mut self) {
        self.client.mount_menu.open = None;
    }
    pub(in crate::game::client_side) fn mount_slots(&self) -> Option<Vec<usize>> {
        let open = self
            .client
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        let mut v: Vec<usize> = (0..36).collect();
        if open.show_saddle {
            v.push(SADDLE);
        }
        if open.show_armor {
            v.push(ARMOR);
        }
        v.extend((0..open.columns * 3).map(|i| CHEST + i));
        Some(v)
    }
    /// How many slots `mount_slots` lists, counted without building it
    /// (`Game::slot_count` asks every frame).
    pub(in crate::game::client_side) fn mount_slot_count(&self) -> Option<usize> {
        let open = self
            .client
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        Some(36 + usize::from(open.show_saddle) + usize::from(open.show_armor) + open.columns * 3)
    }
    pub(in crate::game::client_side) fn mount_layout_inputs(&self) -> Option<(usize, bool, bool)> {
        self.client
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())
            .map(|s| (s.columns, s.show_saddle, s.show_armor))
    }
    pub fn mount_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let open = self
            .client
            .mount_menu
            .open
            .as_ref()
            .filter(|_| self.mount_active())?;
        match i {
            SADDLE => Some((8.0, 18.0)),
            ARMOR => Some((8.0, 36.0)),
            _ if i >= CHEST => {
                let n = i - CHEST;
                (n < open.columns * 3).then(|| {
                    (
                        80.0 + (n % open.columns) as f32 * 18.0,
                        18.0 + (n / open.columns) as f32 * 18.0,
                    )
                })
            }
            _ => None,
        }
    }
    pub(in crate::game::client_side) fn is_mount_chest_slot(&self, i: usize) -> bool {
        self.client
            .mount_menu
            .open
            .as_ref()
            .is_some_and(|open| i >= CHEST && i - CHEST < open.columns * 3)
    }
    pub(in crate::game::client_side) fn mount_slot(&self, i: usize) -> &Option<ItemStack> {
        match self.client.mount_menu.open.as_ref() {
            Some(open) if i == SADDLE => &open.saddle,
            Some(open) if i == ARMOR => &open.armor,
            Some(open) if i >= CHEST && i - CHEST < open.items.len() => &open.items[i - CHEST],
            _ => &self.common.scratch_slot,
        }
    }
    pub(in crate::game::client_side) fn mount_slot_mut(
        &mut self,
        i: usize,
    ) -> &mut Option<ItemStack> {
        match self.client.mount_menu.open.as_mut() {
            Some(open) if i == SADDLE => &mut open.saddle,
            Some(open) if i == ARMOR => &mut open.armor,
            Some(open) if i >= CHEST && i - CHEST < open.items.len() => &mut open.items[i - CHEST],
            _ => &mut self.common.scratch_slot,
        }
    }
    /// `ArmorSlot.mayPlace` and the stack limits of the mount's slots.
    pub(in crate::game::client_side) fn mount_slot_rules(
        &self,
        slot: usize,
        stack: &ItemStack,
    ) -> (bool, u32) {
        let Some(open) = self.client.mount_menu.open.as_ref() else {
            return (false, 64);
        };
        match slot {
            SADDLE => (&*stack.item == "minecraft:saddle", 1),
            ARMOR => (body_armor_fits(self.items(), open.species, stack), 1),
            _ => (true, 64),
        }
    }
    /// A click in the mount menu before the generic handling.
    pub(in crate::game::client_side) fn mount_click(&mut self, right: bool) -> bool {
        // A server's menu: the click goes out like any container click.
        if !self.mount_active() || self.client.remote.is_some() {
            return false;
        }
        let Some(r) = self.client.renderer.0.as_ref() else {
            return true;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        );
        let Some(slot) = self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1) else {
            return true;
        };
        let shift = self.has_shift_down();
        if shift {
            self.mount_quick_move(slot);
        } else if slot >= SADDLE {
            self.click_filtered(slot, right);
        } else {
            return false;
        }
        true
    }
    /// The inventory slot ids in menu order.
    pub(in crate::game::client_side) fn mount_player_slots() -> Vec<usize> {
        (9..36).chain(0..9).collect()
    }
    /// `AbstractMountInventoryMenu.quickMoveStack`.
    pub(in crate::game::client_side) fn mount_quick_move(&mut self, slot: usize) {
        let Some(open) = self.client.mount_menu.open.as_ref() else {
            return;
        };
        let (columns, show_saddle, show_armor) = (open.columns, open.show_saddle, open.show_armor);
        let Some(mut stack) = self.slot_mut(slot).take() else {
            return;
        };
        if slot >= SADDLE {
            let player = Self::mount_player_slots();
            self.move_to(&mut stack, &player, true);
        } else {
            let armor_free = self.mount_slot_mut(ARMOR).is_none();
            let saddle_free = self.mount_slot_mut(SADDLE).is_none();
            let armor_ok = show_armor && self.mount_slot_rules(ARMOR, &stack).0 && armor_free;
            let saddle_ok = show_saddle && self.mount_slot_rules(SADDLE, &stack).0 && saddle_free;
            if armor_ok {
                self.move_to(&mut stack, &[ARMOR], false);
            } else if saddle_ok {
                self.move_to(&mut stack, &[SADDLE], false);
            } else {
                let chest: Vec<usize> = (0..columns * 3).map(|i| CHEST + i).collect();
                let moved = columns > 0 && self.move_to(&mut stack, &chest, false);
                if !moved {
                    // Between the hotbar and the main inventory.
                    let other: Vec<usize> = if slot < 9 {
                        (9..36).collect()
                    } else {
                        (0..9).collect()
                    };
                    self.move_to(&mut stack, &other, false);
                }
            }
        }
        if stack.count > 0 {
            *self.slot_mut(slot) = Some(stack);
        }
    }
    /// `HorseInventoryScreen`'s background, slot frames and the mount.
    pub(in crate::game::client_side) fn draw_mount_background(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
    ) {
        let Some(open) = self.client.mount_menu.open.as_ref() else {
            return;
        };
        let atlas = &self.client.gui.atlas;
        // NautilusInventoryScreen draws its own background.
        let background = if nautilus::is_nautilus(open.species) {
            "gui/container/nautilus"
        } else {
            "gui/container/horse"
        };
        batch.sprite_region(atlas, background, [ox, oy, 176.0, 166.0], 256.0, 256.0);
        if open.columns > 0 {
            batch.sprite_region(
                atlas,
                "container/horse/chest_slots",
                [ox + 79.0, oy + 17.0, open.columns as f32 * 18.0, 54.0],
                90.0,
                54.0,
            );
        }
        if open.show_saddle {
            batch.sprite(atlas, "container/slot", ox + 7.0, oy + 17.0);
        }
        if open.show_armor {
            batch.sprite(atlas, "container/slot", ox + 7.0, oy + 35.0);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        if open.show_saddle && open.saddle.is_none() {
            batch.sprite(atlas, "container/slot/saddle", ox + 8.0, oy + 18.0);
        }
        if open.show_armor && open.armor.is_none() {
            let sprite = if llama::is_llama(open.species) {
                "container/slot/llama_armor"
            } else if nautilus::is_nautilus(open.species) {
                "container/slot/nautilus_armor_inventory"
            } else {
                "container/slot/horse_armor"
            };
            batch.sprite(atlas, sprite, ox + 8.0, oy + 36.0);
        }
        let grey = rgba(64, 64, 64, 255);
        let title = match self.common.mobs.iter().find(|m| m.id == open.mob) {
            Some(m) => m.custom_name.clone().unwrap_or_else(|| {
                let key = format!(
                    "entity.minecraft.{}",
                    m.kind.id.trim_start_matches("minecraft:")
                );
                self.translate_with_fallback(&key, &key)
            }),
            None => String::new(),
        };
        super::screen_info::name_container(|| title.clone());
        batch.text(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
        if let Some(m) = self.common.mobs.iter().find(|m| m.id == open.mob) {
            self.draw_mob_preview(
                batch,
                m,
                ox + 26.0,
                oy + 18.0,
                ox + 78.0,
                oy + 70.0,
                17.0,
                0.25,
            );
        }
    }
    /// `extractEntityInInventoryFollowsMouse` for a mob: depth-sorted,
    /// back-face-culled quads turned toward the cursor.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn draw_mob_preview(
        &self,
        batch: &mut HudBatch,
        mob: &Mob,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        size: f32,
        offset_y: f32,
    ) {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let x_angle = ((cx - self.client.cursor.0) / 40.0).atan();
        let y_angle = ((cy - self.client.cursor.1) / 40.0).atan();
        let mut m = mob.clone();
        m.previous = m.position;
        m.body_yaw = 180.0 + x_angle * 20.0;
        m.previous_body_yaw = m.body_yaw;
        m.head_yaw = m.body_yaw + x_angle * 20.0;
        m.previous_head_yaw = m.head_yaw;
        m.head_pitch = -y_angle * 20.0;
        m.previous_head_pitch = m.head_pitch;
        m.hurt_time = 0;
        m.death_time = 0;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        // A screen preview: custom entity model animations see is_in_gui = 1.
        sunrise_client::mobs::cem_anim::in_gui(|| {
            sunrise_client::mobs::render::render_with(
                &m,
                1.0,
                camera,
                &lookup,
                240 | 240 << 8,
                self,
                &[],
                &mut verts,
            );
        });
        let height = m.kind.height as f32;
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, height / 2.0 + offset_y, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x((y_angle * 20.0).to_radians());
        let mut quads: Vec<(f32, [[f32; 2]; 4], [[f32; 2]; 4], u32)> = Vec::new();
        for q in verts.as_chunks::<4>().0 {
            let p: [glam::Vec3; 4] =
                std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.z >= 0.0 {
                continue;
            }
            let n = n.normalize_or_zero();
            let shade = (0.55 + 0.45 * -n.z - 0.15 * n.y).clamp(0.4, 1.0);
            let c = q[0].color;
            let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
            let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
            let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
            quads.push((
                depth,
                p.map(|v| [v.x, v.y]),
                std::array::from_fn(|i| q[i].uv),
                color,
            ));
        }
        quads.sort_by(|a, b| a.0.total_cmp(&b.0));
        let start = batch.vertices.len();
        for (_, p, uv, color) in quads {
            batch.quad(p, uv, color);
        }
        batch.clip_since(start, x0, y0, x1, y1);
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    /// A stack of `item` whose `equippable` component has `slot` and, when given,
    /// `allowed` entity types.
    fn worn(item: &str, slot: &str, allowed: Option<&[&str]>) -> ItemStack {
        use sunrise_core::nbt::Tag;
        let mut fields = vec![("slot", Tag::String(slot.into()))];
        if let Some(allowed) = allowed {
            fields.push((
                "allowed_entities",
                Tag::List(allowed.iter().map(|id| Tag::String((*id).into())).collect()),
            ));
        }
        let mut stack = ItemStack::new(item, 1);
        stack
            .data_mut()
            .set_other("minecraft:equippable", Some(Tag::compound(fields)));
        stack
    }

    #[test]
    fn the_equippable_component_decides_what_a_horse_wears() {
        let items = sunrise_items::items::Items::empty();
        let horses = ["minecraft:horse", "minecraft:zombie_horse"];
        let mod_armor = worn("example:obsidian_plate", "body", Some(&horses));
        assert!(body_armor_fits(&items, Species::Horse, &mod_armor));
        assert!(body_armor_fits(&items, Species::ZombieHorse, &mod_armor));
        assert!(!body_armor_fits(&items, Species::Donkey, &mod_armor));
        // A `_horse_armor` id without the component fits nothing.
        let bare = ItemStack::new("minecraft:iron_horse_armor", 1);
        assert!(!body_armor_fits(&items, Species::Horse, &bare));
        // Another slot does not fit the body slot.
        let helmet = worn("example:hat", "head", None);
        assert!(!body_armor_fits(&items, Species::Horse, &helmet));
    }

    #[test]
    fn vanilla_body_armour_still_fits_its_mobs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = sunrise_items::items::Items::catalog(
            &root,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let fits = |species, item| body_armor_fits(&items, species, &ItemStack::new(item, 1));
        assert!(fits(Species::Horse, "minecraft:iron_horse_armor"));
        assert!(fits(Species::ZombieHorse, "minecraft:diamond_horse_armor"));
        assert!(!fits(Species::Donkey, "minecraft:iron_horse_armor"));
        assert!(!fits(Species::Horse, "minecraft:iron_chestplate"));
        assert!(fits(Species::Nautilus, "minecraft:iron_nautilus_armor"));
        assert!(fits(Species::Llama, "minecraft:red_carpet"));
        assert!(!fits(Species::Llama, "minecraft:iron_horse_armor"));
    }

    #[test]
    fn llamas_wear_no_saddle() {
        assert!(shows_body(Species::TraderLlama));
        assert!(!shows_saddle(Species::Llama));
        assert!(shows_saddle(Species::Donkey) && !shows_body(Species::Donkey));
    }
}
