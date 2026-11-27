//! The totem of undying (`LivingEntity.checkTotemDeathProtection`: a hand's
//! item carrying the `death_protection` component, whatever item it is, and
//! entity event 35 on the client).
use crate::game::ClientGame;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    block_entity_render::vertex_light,
    mesher::Vertex,
    particles::{TrackingBody, TrackingTarget},
};
use sunrise_gameplay_common::game::totem_common::TOTEM;
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Entity event 35 from a server, for this client's own player or an
    /// entity it draws.
    pub(in crate::game::client_side) fn remote_totem(&mut self, id: i32) {
        if let Some((target, bounds)) = self.remote_tracking_body(id) {
            self.totem_event(
                target,
                Body {
                    bounds,
                    protector: None,
                },
            );
        }
    }
    /// `ClientPacketListener.handleEntityEvent` 35: the totem particles (a
    /// 30-tick emitter) and its sound, heard here only (`playLocalSound`),
    /// and the player's own rumble.
    pub(in crate::game::client_side) fn totem_event(&mut self, target: TrackingTarget, body: Body) {
        self.client.particles.0.tracking_emitter_for(
            ParticleOptions::simple(ParticleType::TotemOfUndying),
            target,
            body.bounds,
            30,
            None,
        );
        self.play_audio_unshared("minecraft:item.totem.use", Some(body.bounds.feet), 1.0, 1.0);
        if target == TrackingTarget::LocalPlayer {
            self.input_feedback(sunrise_client::input_source::Feedback::Totem);
            // findTotem, then LocalPlayer.displayItemActivation. The offsets
            // come from the player's own random in vanilla, not the level's:
            // the client-only particle random stands in for it. The server
            // half hands over the stack it used up; only an event from a
            // remote server, whose item is still in the hand, looks one up.
            let totem = body.protector.unwrap_or_else(|| self.find_totem());
            self.client
                .item_activation
                .activate(totem, self.client.particles.0.random());
        }
    }
    /// `ClientPacketListener.findTotem`: the first hand's item with
    /// `death_protection`, else a new totem (a used single totem is gone by
    /// the time the event arrives).
    pub(in crate::game::client_side) fn find_totem(&self) -> ItemStack {
        [self.common.held(), self.common.offhand.as_ref()]
            .into_iter()
            .flatten()
            .find(|s| self.common.has_death_protection(s))
            .cloned()
            .unwrap_or_else(|| ItemStack::new(TOTEM, 1))
    }
    /// `GameRenderer.tick`: the activated item counts down.
    pub(in crate::game::client_side) fn tick_item_activation(&mut self) {
        if self.common.player.is_some() {
            self.client.item_activation.tick();
        }
    }
    /// `ScreenEffectRenderer.renderItemActivationAnimation`, for the hand
    /// pass: the item in view space with the hand's projection but none of
    /// the camera's turn or bob (a fresh `PoseStack`), so those are undone
    /// here. `FIXED` display, no owner, seed 0 (`LevelExtractor`), full
    /// brightness and `Lighting.Entry.ITEMS_3D`. Only F1 hides it.
    pub(in crate::game::client_side) fn item_activation_geometry(
        &self,
        partial: f32,
        view: glam::Mat4,
        aspect: f32,
        out: &mut Vec<Vertex>,
    ) {
        if self.client.chat_ui.hide_gui {
            return;
        }
        let (Some(stack), Some(pose)) = (
            self.client.item_activation.active(),
            self.client.item_activation.pose(partial, aspect),
        ) else {
            return;
        };
        let state = self.resolve_item(stack, DisplayContext::Fixed, None, 0);
        let to_world = view.inverse();
        let (a, b) = sunrise_client::item_render::gui::items_3d_lights();
        let lights = (
            to_world.transform_vector3(a).normalize(),
            to_world.transform_vector3(b).normalize(),
        );
        // PvP & HUD's totem animation size (100% is vanilla's), about the item's centre.
        let size = self.pvp_totem_scale();
        let pose = pose * glam::Mat4::from_scale(glam::Vec3::splat(size));
        self.submit_item_lit(&state, to_world * pose, vertex_light(0xFF), lights, out);
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod death_protection_tests {
    use serde_json::json;
    use sunrise_assets::item_model::ItemModels;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::totem_common::death_protection_of;
    use sunrise_items::items::ItemStack;

    /// The lookup gives what the stack's whole component map gives, for
    /// stacks that keep, set or remove the component.
    #[test]
    fn hand_lookup_matches_the_component_map() {
        let totem = json!({"death_effects": [{"type": "minecraft:clear_all_effects"}]});
        let mut models = ItemModels::default();
        let prototypes = std::sync::Arc::make_mut(&mut models.prototypes);
        prototypes.insert(
            "minecraft:totem_of_undying".into(),
            [("minecraft:death_protection".to_owned(), totem.clone())].into(),
        );
        prototypes.insert("minecraft:diamond_sword".into(), Default::default());
        let mut patched_sword = ItemStack::new("minecraft:diamond_sword", 1);
        patched_sword.data_mut().set_other(
            "minecraft:death_protection",
            Some(Tag::Compound(Default::default())),
        );
        let mut stripped_totem = ItemStack::new("minecraft:totem_of_undying", 1);
        stripped_totem.data_mut().set_other(
            "!minecraft:death_protection",
            Some(Tag::Compound(Default::default())),
        );
        for (stack, expected) in [
            (ItemStack::new("minecraft:totem_of_undying", 1), Some(totem)),
            (ItemStack::new("minecraft:diamond_sword", 1), None),
            (patched_sword, Some(json!({}))),
            (stripped_totem, None),
            (ItemStack::new("minecraft:unknown_item", 1), None),
        ] {
            let full = models
                .components(&sunrise_sim::item_models::stack_view(&stack))
                .get("minecraft:death_protection")
                .cloned();
            let fast =
                death_protection_of(&models, &stack.item, Some(&stack)).map(|v| v.into_owned());
            assert_eq!(fast, expected, "{}", stack.item);
            assert_eq!(full, expected, "{}", stack.item);
            // A mob's off hand is an item id alone.
            if stack.data().is_none() {
                let by_id = death_protection_of(&models, &stack.item, None).map(|v| v.into_owned());
                assert_eq!(by_id, expected, "{}", stack.item);
            }
        }
    }

    /// A server sends the component as wire bytes, not as a saved tag: the
    /// stack's data has to carry it for the hand lookup, or an apple given
    /// `[death_protection={}]` animates as a plain totem (#12632).
    #[test]
    fn a_servers_patch_reaches_the_hand_lookup() {
        use sunrise_items::items::{ConsumeEffect, ConsumeEffectSpec};
        use sunrise_protocol::item::{Component, Stack, component};
        use sunrise_protocol::registry::{Builtin, Dynamic, Ids};

        let items = [
            "minecraft:air",
            "minecraft:apple",
            "minecraft:totem_of_undying",
        ];
        let mut names = vec![String::new(); component::DEATH_PROTECTION as usize + 1];
        names[component::DEATH_PROTECTION as usize] = "minecraft:death_protection".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".into(),
                    Ids::from_names(items.iter().map(|s| s.to_string()).collect()),
                ),
                (
                    "minecraft:data_component_type".into(),
                    Ids::from_names(names),
                ),
                (
                    "minecraft:mob_effect".into(),
                    Ids::from_names(vec!["minecraft:speed".into()]),
                ),
            ]
            .into(),
        };
        let from_wire = |item, added, removed| {
            let mut wire = Stack::new(item, 1);
            wire.added = added;
            wire.removed = removed;
            sunrise_sim::item_sync::from_network(&wire, &builtin, &Dynamic::default())
        };
        let models = ItemModels::default();
        // `death_protection={}`: an empty effect list and no saved tag.
        let apple = from_wire(
            1,
            vec![(component::DEATH_PROTECTION, Component::Raw(vec![0]))],
            vec![],
        );
        assert_eq!(
            death_protection_of(&models, &apple.item, Some(&apple)).map(|v| v.into_owned()),
            Some(json!({})),
            "the apple carries the server's component"
        );
        // `clear_all_effects`, then speed 1 for 100 ticks.
        let mut bytes = vec![2, 2, 0, 1, 0, 1, 100, 0, 1, 1, 0];
        bytes.extend(1.0f32.to_bits().to_be_bytes());
        let boosted = from_wire(
            1,
            vec![(component::DEATH_PROTECTION, Component::Raw(bytes))],
            vec![],
        );
        let component = death_protection_of(&models, &boosted.item, Some(&boosted))
            .expect("the apple carries the server's component")
            .into_owned();
        assert_eq!(
            sunrise_items::items::parse_death_effects(&component).unwrap(),
            [
                ConsumeEffect::ClearAllEffects,
                ConsumeEffect::ApplyEffects {
                    effects: vec![ConsumeEffectSpec {
                        id: "minecraft:speed".into(),
                        duration: 100,
                        amplifier: 1,
                        ambient: false,
                        show_particles: true,
                        show_icon: true,
                    }],
                    probability: 1.0,
                }
            ],
            "death_effects cross the wire"
        );
        // `totem_of_undying[!death_protection]` keeps the prototype's
        // component from protecting.
        let mut models = ItemModels::default();
        std::sync::Arc::make_mut(&mut models.prototypes).insert(
            "minecraft:totem_of_undying".into(),
            [("minecraft:death_protection".to_owned(), json!({}))].into(),
        );
        let stripped = from_wire(2, vec![], vec![component::DEATH_PROTECTION]);
        assert_eq!(
            death_protection_of(&models, &stripped.item, Some(&stripped)),
            None,
            "a removed component is no protection"
        );
    }
}

/// Where an entity event's entity stands and its size, and, for the event
/// this window's own player fires after the server half used a stack up, the
/// protector copy the activation animation shows.
#[derive(Clone)]
pub(in crate::game) struct Body {
    pub bounds: TrackingBody,
    pub(in crate::game) protector: Option<ItemStack>,
}
