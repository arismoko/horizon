//! Potions and status effects in the game: drinking (`PotionItem` via
//! `Consumable`), throwing splash/lingering potions (`ThrownSplashPotion`),
//! filling glass bottles (`BottleItem`), the per-tick effect application,
//! effect attribute modifiers and the HUD effect icons (`Hud.extractEffects`).
use super::{Screen, item_draw::owner_at};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    block_entity_render::vertex_light, hud::HudBatch, hud_layout, hud_overlays, inventory_effects,
    mesher::Vertex,
};
#[cfg(test)]
use sunrise_core::geometry::Aabb;
use sunrise_core::{geometry::Vec3, random::LegacyRandom};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::potions_common::{PotionState, ThrownPotion};
use sunrise_items::items::ItemStack;
#[cfg(test)]
use sunrise_level::effects::EffectInstance;
use sunrise_level::effects::PlayerEffects;
#[cfg(test)]
use sunrise_sim::mobs::Who;

/// The player (not one of `skip`, the thrower and who rides with them before
/// it has left them) whose box, grown by `margin` (`ProjectileUtil.computeMargin`),
/// holds `p` (`Projectile.canHitEntity`).
#[cfg(test)]
pub(in crate::game::client_side) fn thrown_player_at<'a>(
    players: &'a [(Who, Aabb)],
    skip: &[Who],
    p: Vec3,
    margin: f64,
) -> Option<&'a (Who, Aabb)> {
    players.iter().find(|(who, b)| {
        !skip.contains(who) && b.inflate(Vec3::new(margin, margin, margin)).contains(p)
    })
}
/// `ProjectileUtil.computeMargin`: how far the pick box is grown at `age`.
#[cfg(test)]
pub(in crate::game::client_side) fn thrown_margin(age: u32) -> f64 {
    ((f64::from(age) - 2.0) / 20.0).clamp(0.0, 0.3)
}
/// `Projectile.isOutsideOwnerCollisionRange` (`checkLeftOwner`): none of the
/// boxes of the thrower's vehicle tree (`owner.getRootVehicle().getSelfAndPassengers()`,
/// here `ridden`) is within a block of the potion's (0.25 wide) grown by its
/// motion; true too when there is no thrower (`ridden` empty).
#[cfg(test)]
pub(in crate::game::client_side) fn thrown_left_owner(
    ridden: &[Aabb],
    position: Vec3,
    velocity: Vec3,
) -> bool {
    let h = 0.125;
    let reach = Aabb::new(
        Vec3::new(position.x - h, position.y, position.z - h),
        Vec3::new(position.x + h, position.y + 2.0 * h, position.z + h),
    )
    .expand_towards(velocity)
    .inflate(Vec3::new(1.0, 1.0, 1.0));
    !ridden.iter().any(|b| b.intersects(reach))
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A client's own effects (`LivingEntity.tickEffects` on the client,
    /// `MobEffectInstance.tickClient`): they count down and blend, and the
    /// server removes them. Their modifiers and flags move the predicted
    /// player.
    pub fn tick_client_effects(&mut self) {
        client_tick_effects(
            &mut self.common.potions.effects,
            &mut self.client.darkness_blend,
        );
        self.apply_effect_status();
    }
    /// `ThrowableProjectile.tick` on a server's client: it falls and moves
    /// by itself between the server's updates and stops at a block; only
    /// the server breaks it.
    pub fn step_remote_thrown(&mut self) {
        let mut thrown = std::mem::take(&mut self.common.potions.thrown);
        for t in &mut thrown {
            t.previous = t.position;
            t.age += 1;
            self.common.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            let state = self.common.world.block(
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            );
            if self.common.res.facts().get(state).full_collision {
                t.velocity = Vec3::ZERO;
            } else {
                t.position = next;
            }
        }
        self.common.potions.thrown = thrown;
    }
    /// `ThrownItemRenderer` for thrown splash and lingering potions and
    /// experience bottles: the item (GROUND) facing the camera at scale 1,
    /// lit at the light probe (the eye, 0.85 of the 0.25 height).
    pub(in crate::game::client_side) fn thrown_potion_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        for t in &self.common.potions.thrown {
            if self.hides_as_camera(t.id) {
                continue;
            }
            let previous = glam::DVec3::new(t.previous.x, t.previous.y, t.previous.z);
            let position = glam::DVec3::new(t.position.x, t.position.y, t.position.z);
            // ThrowableProjectile.shouldRenderAtSqrDistance: hidden for its
            // first two ticks within 3.5 blocks (not in the thrower's face).
            if t.age < 2 && position.distance_squared(camera) < 12.25 {
                continue;
            }
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = previous.lerp(position, alpha);
            let owner = owner_at(p.to_array(), 0.0, &t.stack.item);
            let state = self.resolve_item(&t.stack, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            let light = self.common.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            // Frame generation and temporal upscaling follow its own flight; one not yet
            // given an id (0) can't be told apart from another, so it keeps the camera's.
            let motion = if t.id == 0 {
                0
            } else {
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(t.id)
            };
            self.mark_motion(out.len(), motion);
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
        self.mark_motion(out.len(), 0);
    }
    /// `Screen.showsActiveEffects`: the survival and creative inventories
    /// draw their own effect list when `EffectsInInventory.canSeeEffects`
    /// (32 or more pixels right of the screen).
    pub(in crate::game::client_side) fn screen_shows_active_effects(
        &self,
        batch: &HudBatch,
    ) -> bool {
        let (ox, image_width) = match self.client.screen {
            Screen::Inventory
                if self.common.container.is_none()
                    && self.common.craft_size == 2
                    && !self.mount_active() =>
            {
                (self.inventory_origin(batch).0, 176)
            }
            Screen::Creative => (
                Self::creative_origin(batch).0,
                sunrise_client::creative::Creative::WIDTH as i32,
            ),
            _ => return false,
        };
        batch.width as i32 - (ox as i32 + image_width + 2) >= 32
    }
    /// `Hud.extractEffects`: icons top-right, beneficial row then harmful row;
    /// every screen but the inventories keeps them.
    pub(in crate::game::client_side) fn draw_effect_icons(&self, batch: &mut HudBatch) {
        if self.common.potions.effects.list.is_empty() || self.screen_shows_active_effects(batch) {
            return;
        }
        let icons: Vec<hud_overlays::EffectIcon> = self
            .common
            .potions
            .effects
            .list
            .iter()
            .map(hud_overlays::EffectIcon::from)
            .collect();
        let draws = hud_overlays::effects(batch.width as i32, false, &icons);
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
    /// `EffectsInInventory`: panels to the right of survival/creative inventory.
    /// The caller draws the returned tooltip after every other tooltip.
    /// `ScreenEvent.RenderInventoryMobEffects` fires when the build lays the
    /// panels out once a frame, and its answer is kept in the build.
    pub(in crate::game::client_side) fn draw_inventory_effects(
        &self,
        batch: &mut HudBatch,
        origin: (f32, f32),
        image_width: i32,
    ) -> Option<Vec<(Text, u32)>> {
        let space = Self::effects_space(batch, [origin.0, origin.1], image_width as f32);
        let layout = self.built_container(batch).map_or_else(
            || inventory_effects::EffectsLayout::vanilla(space.available()),
            |layout| layout.effects_layout,
        );
        let panels = self.effect_panels(&space, &layout);
        let hovered = inventory_effects::paint(
            batch,
            &self.client.gui.atlas,
            &panels,
            [self.client.cursor.0 as i32, self.client.cursor.1 as i32],
        )?;
        Some(self.effect_tooltip(batch, hovered))
    }
    /// A hovered panel's tooltip lines, after `GatherEffectScreenTooltipsEvent`
    /// (fired only while a mod listens).
    pub(in crate::game::client_side) fn effect_tooltip(
        &self,
        batch: &HudBatch,
        hovered: inventory_effects::HoveredEffectPanel<'_, '_>,
    ) -> Vec<(Text, u32)> {
        let panel = hovered.panel;
        let mut lines = inventory_effects::tooltip(hovered);
        if mod_api::__private::listened(mod_api::HookId::GatherEffectScreenTooltips) {
            let view = self.screen_view(batch, self.client.screen);
            let effect =
                sunrise_gameplay_common::game::native_living_body::active_effect(panel.effect);
            self.native_gather_effect_screen_tooltips(&view, &effect, &mut lines);
        }
        lines
    }
    /// The rectangle `draw_inventory_effects` covers, none with no effects
    /// or too little room right of the window, laid out as `layout` says.
    pub(in crate::game::client_side) fn inventory_effects_rect(
        &self,
        batch: &HudBatch,
        origin: [f32; 2],
        image_width: f32,
        layout: &inventory_effects::EffectsLayout,
    ) -> Option<[f32; 4]> {
        let space = Self::effects_space(batch, origin, image_width);
        let panels = self.effect_panels(&space, layout);
        let first = panels.first()?;
        let width = panels.iter().map(|p| p.width).max()?;
        let height = (panels.len() as i32 - 1) * first.spacing + 32;
        Some([first.x as f32, first.y as f32, width as f32, height as f32])
    }
    /// The room the effect panels lay out in (`EffectsInInventory`'s space):
    /// the window's width, the container's origin and its image width.
    pub(in crate::game::client_side) fn effects_space(
        batch: &HudBatch,
        origin: [f32; 2],
        image_width: f32,
    ) -> inventory_effects::EffectsSpace {
        inventory_effects::EffectsSpace {
            screen_width: batch.width as i32,
            origin: [origin[0] as i32, origin[1] as i32],
            image_width: image_width as i32,
        }
    }
    pub(in crate::game::client_side) fn effect_panels(
        &self,
        space: &inventory_effects::EffectsSpace,
        layout: &inventory_effects::EffectsLayout,
    ) -> Vec<inventory_effects::EffectPanel<'_>> {
        inventory_effects::panels(
            space,
            layout,
            &self.common.potions.effects.list,
            self.common.clock.rate(),
            &|key| self.common.pack.names.display_get(key),
            &|text| self.client.gui.atlas.font.width(text).ceil() as i32,
        )
    }
    /// Vanilla's layout, as `native_screen_render_inventory_mob_effects`
    /// leaves it, for the panels and their exclusion rect. No fire with no
    /// effect to show or no room (vanilla returns first), nor with no mod
    /// listening. The answer is taken once a frame: the display's change
    /// builds twice in one frame (`apply_ui_settings` then `observe_screen`),
    /// and the second build reuses the first one's answer.
    pub(in crate::game::client_side) fn inventory_effects_layout(
        &self,
        space: &inventory_effects::EffectsSpace,
        rect: [f32; 4],
    ) -> inventory_effects::EffectsLayout {
        let mut layout = inventory_effects::EffectsLayout::vanilla(space.available());
        if self.common.potions.effects.list.is_empty()
            || space.available() < 32
            || !mod_api::__private::listened(mod_api::HookId::ScreenRenderInventoryMobEffects)
        {
            return layout;
        }
        if let Some(answer) = self.client.screen_state.0.build.effects_answer(
            self.client.frames,
            self.client.screen,
            *space,
        ) {
            return answer;
        }
        // Asked while the screen is laid out: nothing is drawn beside it yet.
        let view = super::screen_backgrounds::without_regions(
            &self.screen_view_at(self.client.screen, rect),
        );
        let mut canceled = false;
        self.native_screen_render_inventory_mob_effects(
            &view,
            space.available(),
            &mut layout.compact,
            &mut layout.offset,
            &mut canceled,
        );
        layout.hidden = canceled;
        self.client.screen_state.0.build.keep_effects_answer(
            self.client.frames,
            self.client.screen,
            *space,
            layout,
        );
        layout
    }
    pub(in crate::game::client_side) fn effects_json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.potions.effects.list.iter().map(|e| serde_json::json!({ "id": e.id, "duration": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            "thrown": self.potions.thrown.iter().map(|t| [t.position.x, t.position.y, t.position.z]).collect::<Vec<_>>(),
        })
    }
}

/// `LivingEntity.tickEffects` on a client: each `MobEffectInstance.tickClient`
/// counts its effect down, then ticks its `BlendState`. The darkness effect's
/// is the fade `darkness_lighting` and `darkness_fog` draw; the integrated
/// level's `tick_sculk_entities` is not run for a server's client (#3393).
pub(in crate::game::client_side) fn client_tick_effects(
    effects: &mut PlayerEffects,
    darkness: &mut (f32, f32),
) {
    for e in &mut effects.list {
        e.tick_client();
    }
    *darkness = super::sculk::darkness_blend_tick(effects, *darkness);
}

/// `handleAddEntity` for a thrown potion or bottle on a server's client:
/// drawn where the server says (its item comes in its entity data).
pub fn add_remote_thrown(potions: &mut PotionState, id: u32, kind: &str, at: Vec3, velocity: Vec3) {
    potions.thrown.retain(|t| t.id != id);
    let item = match kind {
        "minecraft:lingering_potion" | "minecraft:experience_bottle" => kind,
        _ => "minecraft:splash_potion",
    };
    potions.thrown.push(ThrownPotion {
        id,
        stack: ItemStack::new(item, 1),
        previous: at,
        position: at,
        velocity,
        age: 0,
        owner: None,
        thrower: None,
        guest: None,
        left_owner: false,
        last_deflected_by: None,
        random: LegacyRandom::new(i64::from(id)),
    });
}

/// ThrowableItemProjectile.DATA_ITEM_STACK: the potion it shows.
pub(in crate::game) fn remote_thrown_item(potions: &mut PotionState, id: u32, stack: ItemStack) {
    if let Some(t) = potions.thrown.iter_mut().find(|t| t.id == id) {
        t.stack = stack;
    }
}

pub(in crate::game) fn move_remote_thrown(potions: &mut PotionState, id: u32, at: Vec3) -> bool {
    let Some(t) = potions.thrown.iter_mut().find(|t| t.id == id) else {
        return false;
    };
    t.position = at;
    true
}

/// `handleSetEntityMotion` for a thrown potion. False when `id` isn't one.
pub(in crate::game) fn remote_thrown_motion(
    potions: &mut PotionState,
    id: u32,
    velocity: Vec3,
) -> bool {
    let Some(t) = potions.thrown.iter_mut().find(|t| t.id == id) else {
        return false;
    };
    t.velocity = velocity;
    true
}

pub(in crate::game) fn remove_remote_thrown(potions: &mut PotionState, id: u32) {
    potions.thrown.retain(|t| t.id != id);
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::sculk::darkness_blend_added;
    use super::*;
    use sunrise_client::screen_effects::nausea_blend_added;

    /// #5876: a potion thrown straight up from inside its thrower's box does
    /// not hit them while it climbs out, then hits them on the way down;
    /// another player in its path is hit at once.
    #[test]
    fn a_thrown_potion_hits_its_thrower_only_after_leaving_them() {
        // A player's box: 0.6 wide, 1.8 high, feet at the origin.
        let body = |x: f64| Aabb::new(Vec3::new(x - 0.3, 0.0, -0.3), Vec3::new(x + 0.3, 1.8, 0.3));
        let thrower = Who::Mob(1);
        let players = [(thrower, body(0.0)), (Who::Mob(2), body(5.0))];
        let ridden = [body(0.0)];
        let eye = Vec3::new(0.0, 1.52, 0.0);
        let up = Vec3::new(0.0, 0.5, 0.0);
        let m = thrown_margin(10);
        assert!((m - 0.3).abs() < 1e-12 && thrown_margin(1) == 0.0);
        // Thrown from the eye: still within a block of its thrower.
        assert!(!thrown_left_owner(&ridden, eye, up));
        assert!(thrown_player_at(&players, &[thrower], eye, m).is_none());
        // Without the gate the thrower's box does hold it.
        assert_eq!(
            thrown_player_at(&players, &[], eye, m).map(|t| t.0),
            Some(thrower)
        );
        // Four blocks up and rising: clear of the box grown by a block.
        let high = Vec3::new(0.0, 5.0, 0.0);
        assert!(thrown_left_owner(&ridden, high, up));
        // Falling back (left_owner now set), the thrower is hit at their head.
        let head = Vec3::new(0.0, 2.0, 0.0);
        assert_eq!(
            thrown_player_at(&players, &[], head, m).map(|t| t.0),
            Some(thrower)
        );
        // Another player is hit though the thrower has not left; a miss is none.
        let beside = Vec3::new(5.2, 1.0, 0.0);
        assert_eq!(
            thrown_player_at(&players, &[thrower], beside, m).map(|t| t.0),
            Some(Who::Mob(2))
        );
        assert!(thrown_player_at(&players, &[thrower], Vec3::new(2.5, 1.0, 0.0), m).is_none());
        // A passenger sharing the thrower's vehicle is skipped like the thrower.
        assert!(thrown_player_at(&players, &[thrower, Who::Mob(2)], beside, m).is_none());
        // The vehicle's box keeps it from leaving: a mount 2 blocks wide.
        let mount = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let beside_mount = Vec3::new(1.9, 0.0, 0.0);
        assert!(thrown_left_owner(&[body(0.0)], beside_mount, up));
        assert!(!thrown_left_owner(&[body(0.0), mount], beside_mount, up));
        // No thrower (a dispenser's potion): it has left them already.
        assert!(thrown_left_owner(&[], eye, up));
    }

    /// A server's client (multiplayer and the default single player) fades
    /// the darkness effect in over its 22 blend ticks, holds it, and fades
    /// it out over the last 22 (#3393).
    #[test]
    fn a_servers_client_blends_the_darkness_effect() {
        let mut effects = PlayerEffects::default();
        effects.add(EffectInstance::new("darkness", 60, 0));
        let mut darkness = (0.0, 0.0);
        client_tick_effects(&mut effects, &mut darkness);
        assert_eq!(darkness, (0.0, 1.0 / 22.0), "the first tick fades in");
        for _ in 1..22 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!((darkness.1 - 1.0).abs() < 1e-5, "{darkness:?}");
        // 38 ticks left: full until 22 remain, then the fade out.
        for _ in 0..16 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert_eq!(effects.get("darkness").map(|e| e.duration), Some(22));
        assert!(
            darkness.1 < darkness.0,
            "fading out at 22 left: {darkness:?}"
        );
        for _ in 0..21 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!(darkness.1.abs() < 1e-5, "{darkness:?}");
    }

    /// `ClientPacketListener.handleUpdateMobEffect`: a new darkness fades in
    /// when the server blends it; one sent unblended (joining, changing
    /// dimension) is at full strength at once.
    #[test]
    fn a_new_darkness_starts_blended_or_at_once() {
        let e = EffectInstance::new("darkness", 260, 0);
        assert_eq!(darkness_blend_added(&e, true), (0.0, 0.0));
        assert_eq!(darkness_blend_added(&e, false), (1.0, 1.0));
        let ending = EffectInstance::new("darkness", 10, 0);
        assert_eq!(darkness_blend_added(&ending, false), (0.0, 0.0));
        let infinite = EffectInstance::new("darkness", -1, 0);
        assert_eq!(darkness_blend_added(&infinite, false), (1.0, 1.0));
    }

    /// `ClientPacketListener.handleUpdateMobEffect`: a new nausea fades in
    /// when the server blends it; one sent unblended (joining, changing
    /// dimension) is at full strength at once, unless it is already within
    /// its blend-out ticks.
    #[test]
    fn a_new_nausea_starts_blended_or_at_once() {
        let e = EffectInstance::new("nausea", 600, 0);
        assert_eq!(nausea_blend_added(&e, true), (0.0, 0.0));
        assert_eq!(nausea_blend_added(&e, false), (1.0, 1.0));
        let ending = EffectInstance::new("nausea", 40, 0);
        assert_eq!(nausea_blend_added(&ending, false), (0.0, 0.0));
        let infinite = EffectInstance::new("nausea", -1, 0);
        assert_eq!(nausea_blend_added(&infinite, false), (1.0, 1.0));
    }

    /// Real panel painting/hover selection dispatches even for a fully visible
    /// widget. Both inventories retain vanilla lines, suppression and layout rules.
    #[test]
    #[ignore = "daily"]
    fn a_mod_adds_a_line_to_an_effect_tooltip() {
        use mod_api::{self, ClientHost, GatherEffectScreenTooltipsEvent, Hooks, ModEntry, Mods};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone, Copy)]
        struct Case {
            room: i32,
            id: &'static str,
            name: &'static str,
            default_tooltip: bool,
        }
        #[derive(Clone)]
        struct Mod {
            screen: &'static str,
            case: Case,
            suppress: bool,
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn gather_effect_screen_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut GatherEffectScreenTooltipsEvent<'_, '_>,
            ) {
                self.calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(
                    event.effect.id,
                    mod_platform::__private::part_tests::effect_key(self.case.id)
                );
                assert_eq!((event.effect.duration, event.effect.amplifier), (450, 1));
                assert_eq!(event.screen.id, self.screen);
                let initial: Vec<_> = event.tooltip.iter().collect();
                if self.case.default_tooltip {
                    assert_eq!(
                        initial,
                        [
                            (mod_api::Text::literal(self.case.name), u32::MAX),
                            (mod_api::Text::literal("00:22"), u32::MAX)
                        ]
                    );
                } else {
                    assert!(initial.is_empty());
                }
                if self.suppress {
                    event.tooltip.clear();
                } else {
                    event
                        .tooltip
                        .push((mod_api::Text::literal("Fast feet"), 0xff55ffff))
                        .unwrap();
                }
            }
        }
        struct HidePanels;
        #[mod_api::__subscriptions]
        impl Hooks for HidePanels {
            fn screen_render_inventory_mob_effects(
                &self,
                _: &dyn ClientHost,
                _: &mut mod_api::ScreenRenderInventoryMobEffectsEvent<'_>,
            ) -> mod_api::Flow {
                mod_api::Flow::Cancel
            }
        }
        super::super::native_entity_refs::tests::with_client_game(|game| {
            for (screen, screen_id, image_width) in [
                (Screen::Inventory, "inventory", 176),
                (Screen::Creative, "creative", 195),
            ] {
                game.client.screen = screen;
                // Fixed panel position; right-side room selects wide or compact.
                // The longer effect name also clips in the smallest wide layout.
                for case in [
                    Case {
                        room: 200,
                        id: "speed",
                        name: "Speed II",
                        default_tooltip: false,
                    },
                    Case {
                        room: 100,
                        id: "speed",
                        name: "Speed II",
                        default_tooltip: true,
                    },
                    Case {
                        room: 120,
                        id: "fire_resistance",
                        name: "Fire Resistance II",
                        default_tooltip: true,
                    },
                ] {
                    game.potions.effects.list = vec![EffectInstance::new(case.id, 450, 1)];
                    let left = 102 + image_width;
                    let origin = (100.0, 50.0);
                    let mut batch = HudBatch::scratch([(left + case.room) as u32, 480]);
                    game.client.cursor = ((left + 7) as f32, 57.0);
                    let vanilla = if case.default_tooltip {
                        vec![
                            (case.name.to_owned(), u32::MAX),
                            ("00:22".to_owned(), u32::MAX),
                        ]
                    } else {
                        Vec::new()
                    };
                    let lines = game
                        .draw_inventory_effects(&mut batch, origin, image_width)
                        .unwrap();
                    assert_eq!(
                        lines
                            .iter()
                            .map(|(text, color)| (text.plain(), *color))
                            .collect::<Vec<_>>(),
                        vanilla
                    );
                    // The existing shared renderer draws nothing for an unchanged
                    // empty list, without running other tooltip hooks.
                    let painted = batch.vertices.len();
                    game.draw_tooltip_lines(
                        &mut batch,
                        &lines,
                        game.client.cursor.0,
                        game.client.cursor.1,
                    );
                    assert_eq!(batch.vertices.len() > painted, case.default_tooltip);

                    for suppress in [false, true] {
                        let calls = Arc::new(AtomicUsize::new(0));
                        let listener = Mod {
                            screen: screen_id,
                            case,
                            suppress,
                            calls: calls.clone(),
                        };
                        let mods: Mods =
                            vec![ModEntry::new("effect-tips", Box::new(listener.clone()))].into();
                        mod_platform::testing::with_mods(mods, || {
                            let lines = game
                                .draw_inventory_effects(&mut batch, origin, image_width)
                                .unwrap();
                            let plain: Vec<_> = lines
                                .iter()
                                .map(|(text, color)| (text.plain(), *color))
                                .collect();
                            let mut expected = vanilla.clone();
                            if suppress {
                                expected.clear();
                            } else {
                                expected.push(("Fast feet".to_owned(), 0xff55ffff));
                            }
                            assert_eq!(plain, expected);
                            assert_eq!(calls.load(Ordering::Relaxed), 1);
                            let painted = batch.vertices.len();
                            game.draw_tooltip_lines(
                                &mut batch,
                                &lines,
                                game.client.cursor.0,
                                game.client.cursor.1,
                            );
                            assert_eq!(batch.vertices.len() > painted, !suppress);

                            // A miss, a hidden layout and too little room must
                            // never gather tooltips, even with a subscriber.
                            game.client.cursor = (0.0, 0.0);
                            assert!(
                                game.draw_inventory_effects(&mut batch, origin, image_width)
                                    .is_none()
                            );
                            game.client.cursor = ((left + 7) as f32, 57.0);
                            let hidden: Mods = vec![
                                ModEntry::new("effect-tips", Box::new(listener.clone())),
                                ModEntry::new("hide-panels", Box::new(HidePanels)),
                            ]
                            .into();
                            mod_platform::testing::with_mods(hidden, || {
                                // Layout hooks run at screen construction, not painting.
                                game.client.frames += 1;
                                let space = crate::game::ClientGame::effects_space(
                                    &batch,
                                    [origin.0, origin.1],
                                    image_width as f32,
                                );
                                let layout = game.inventory_effects_layout(
                                    &space,
                                    [origin.0, origin.1, image_width as f32, 166.0],
                                );
                                let panels = game.effect_panels(&space, &layout);
                                assert!(
                                    inventory_effects::paint(
                                        &mut batch,
                                        &game.client.gui.atlas,
                                        &panels,
                                        [game.client.cursor.0 as i32, game.client.cursor.1 as i32],
                                    )
                                    .is_none()
                                );
                            });
                            let mut narrow = HudBatch::scratch([(left + 31) as u32, 480]);
                            assert!(
                                game.draw_inventory_effects(&mut narrow, origin, image_width)
                                    .is_none()
                            );
                            assert_eq!(calls.load(Ordering::Relaxed), 1);
                        });
                    }
                }
            }
        });
    }
}
