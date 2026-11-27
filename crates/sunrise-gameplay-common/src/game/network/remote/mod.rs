//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
pub mod selector_metadata;
#[cfg(test)]
pub use selector_metadata::hurt_sound_for;
pub mod attack_target;
pub mod fall_sound;
pub mod general;
pub mod interaction_actions;
pub mod interaction_prediction;
#[allow(unused_imports)]
pub use inventory::{
    CREATIVE_SYNCED, creative_sync_slot, creative_sync_stacks, mark_synced, predict_clicks,
};
pub mod recipe_properties;
pub mod use_item;

pub mod block_entity_data;
#[allow(unused_imports)]
pub use super::{Dim, Hit, Screen};
#[cfg(test)]
pub use movement::MovePacket;
#[allow(unused_imports)]
#[cfg(test)]
// The move-packet tests reach it through `super::super`.
pub use movement::stepped_move;
pub use std::collections::HashMap;
#[cfg(test)]
pub use std::sync::Arc;
pub use sunrise_core::geometry::Vec3;
#[allow(unused_imports)]
pub use sunrise_items::items::ItemStack;
#[allow(unused_imports)]
pub use sunrise_level::{chunk::Chunk, containers::BlockEntity};
#[allow(unused_imports)]
pub use sunrise_protocol::{
    item::Stack,
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic},
    secure_chat::Signature,
    text::Language,
};
pub use sunrise_sim::first_person_sim::Hand;
#[allow(unused_imports)]
pub use sunrise_sim::{
    join_seq::JoinSeq,
    menu::Input,
    mobs::{Mob, MobType},
    motion::PlayerMotion,
    sprint::Keys,
};
#[allow(unused_imports)]
pub use {
    sunrise_core::nbt::Tag,
    sunrise_mod_runtime::attachments::{Holder, ResidentAttachments},
    sunrise_world::{
        registry::BlockStateId,
        stepped_interpolation::{Path, PosRot, SteppedInterpolation},
    },
};

pub mod abilities;
pub mod chat;
pub mod chat_received;
pub mod commands;
#[allow(unused_imports)]
#[cfg(test)]
pub use chat::Decoration;
pub mod client_level_tick;
pub mod effect;
pub mod entity_data_schema;
pub mod entity_section_steps;
pub mod inventory;
pub mod item_use;
pub mod kept_columns;
pub mod minecarts;
pub mod motion;
pub mod movement;
pub mod packets_entity;
pub mod packets_object;
pub mod packets_player;
pub mod packets_world;
pub mod passengers;
pub mod skin;
pub mod status;
pub mod tables;
pub mod tick;
pub mod ui;
#[path = "use.rs"]
pub mod r#use;

#[allow(unused_imports)]
pub use super::villager_client::builtin_registries;

pub mod chat_queue;

/// A text component the server sent, for the HUD.
pub fn remote_text(tag: &Tag) -> sunrise_game_data::text::Text {
    use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
    comp_text(&tag_to_json(tag))
}

/// `Player.getVoicePitch`: a constant 1.0
/// (`javap -p -c -constants net.minecraft.world.entity.player.Player`:
/// `0: fconst_1  1: freturn`), so a player's own hurt and death sounds are
/// neither raised for a baby nor jittered - `LivingEntity.getVoicePitch`, which
/// the mobs port, does both. It takes no random, so it draws nothing from the
/// level random either.
pub fn player_voice_pitch() -> f32 {
    1.0
}

/// `ClientboundOpenScreenPacket.title` as a container screen draws it: the
/// component's styles, and the `§` codes a literal holds (`StringDecomposer`).
pub fn menu_title(tag: &Tag) -> sunrise_game_data::text::Text {
    remote_text(tag).resolve_legacy()
}

/// Smooth an entity toward a new position over three ticks (InterpolationHandler).
#[derive(Clone, Copy)]
/// `LinearInterpolationHandler`'s target: a position and rotation (yRot,
/// xRot) reached in `steps` ticks.
pub struct Lerp {
    pub target: [f64; 3],
    pub rotation: Option<(f32, f32)>,
    pub steps: u32,
}

/// `SteppedInterpolationHandler.doInterpolate`'s `setPos` and `setRot` on a
/// mob: its yaw kept continuous (the same turn as vanilla's, without the
/// wrap that would spin it in the frame between ticks), its pitch clamped.
pub fn place_interpolated(m: &mut Mob, at: PosRot) {
    m.position = Vec3::new(at.position[0], at.position[1], at.position[2]);
    m.yaw += wrap_degrees(at.yaw - m.yaw);
    m.head_pitch = (at.pitch % 360.0).clamp(-90.0, 90.0);
}

/// Where a mob is, as its `SteppedInterpolationHandler` sees it.
pub fn mob_pos_rot(m: &Mob) -> PosRot {
    PosRot {
        position: [m.position.x, m.position.y, m.position.z],
        yaw: m.yaw,
        pitch: m.head_pitch,
    }
}

/// The open menu name of a server's horse inventory, then its chest's
/// columns (the packet names no `MenuType`).
pub const MOUNT_MENU: &str = "sunrise:mount";
/// ServerboundPlayerInputPacket's shift bit.
pub const SHIFT_INPUT: u8 = 1 << 5;

pub fn face_direction(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        [1, 0, 0] => 5,
        _ => 1,
    }
}

/// A protocol number format as the scoreboard's.
pub fn number_format(f: &play::NumberFormat) -> sunrise_game_data::scoreboard::NumberFormat {
    use sunrise_game_data::scoreboard::{NumberFormat, tag_to_json};
    match f {
        play::NumberFormat::Blank => NumberFormat::Blank,
        play::NumberFormat::Styled(t) => NumberFormat::Styled(tag_to_json(t)),
        play::NumberFormat::Fixed(t) => NumberFormat::Fixed(tag_to_json(t)),
    }
}

impl crate::game::CommonGame<'_> {
    // remote_sound_name: remote/sync.rs

    // connect_remote_session: remote/session.rs

    // init_remote_for_replay: remote/session.rs

    // leave_server: remote/session.rs

    /// Their kind and place, for `status` (shared projectiles; client callers append `hooks_status`).
    pub fn projectiles_status(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        let at = |p: sunrise_core::geometry::Vec3| [p.x, p.y, p.z];
        let mut out = Vec::new();
        for f in &self.fangs.list {
            out.push(
                json!({"id": f.id, "kind": "minecraft:evoker_fangs", "position": f.pos,
                "yaw": f.yaw, "started": f.started, "life": f.life}),
            );
        }
        for t in &self.tnts {
            out.push(json!({"id": t.id, "kind": "minecraft:tnt", "position": at(t.position), "fuse": t.fuse}));
        }
        for f in &self.falling {
            out.push(json!({"id": f.id, "kind": "minecraft:falling_block", "position": at(f.position), "velocity": at(f.velocity), "state": f.state.0}));
        }
        for b in &self.boats {
            out.push(json!({"id": b.id, "kind": "minecraft:boat", "position": at(b.position), "velocity": at(b.velocity)}));
        }
        for c in &self.carts {
            out.push(json!({"id": c.id, "kind": "minecraft:minecart", "position": at(c.position), "velocity": at(c.velocity)}));
        }
        for c in &self.crystals {
            out.push(json!({"id": c.id, "kind": "minecraft:end_crystal", "position": at(c.position), "show_bottom": c.show_bottom}));
        }
        for t in &self.tridents {
            out.push(json!({"id": t.id, "kind": "minecraft:trident", "position": at(t.position), "velocity": at(t.velocity), "on_fire": t.fire_ticks > 0}));
        }
        for r in &self.rockets {
            out.push(json!({"id": r.id, "kind": "minecraft:firework_rocket", "position": at(r.position), "velocity": at(r.velocity)}));
        }
        for p in &self.creatures.projectiles {
            out.push(json!({"id": p.id, "kind": format!("{:?}", p.kind).split([' ', '{']).next().unwrap_or(""), "position": at(p.position), "velocity": at(p.velocity),
                "player_owned": p.owner == Some(sunrise_sim::mobs::Who::Player),
                "on_fire": p.arrow.as_ref().is_some_and(|a| a.on_fire())}));
        }
        for (kind, p, _) in self.potions.thrown_entities() {
            out.push(json!({"kind": kind, "position": at(p)}));
        }
        for (id, p) in self.end.eye_positions() {
            out.push(json!({"id": id, "kind": "minecraft:eye_of_ender", "position": p}));
        }
        out
    }

    // move_entity: remote/motion.rs

    // remote_objective: remote/sync.rs

    // remote_teleport: remote/sync.rs

    // tick_remote and tick_remote_player: remote/tick.rs

    // tick_remote_guardian_eyes: remote/tick.rs

    // tick_remote_entities: remote/tick.rs

    // remote_mining: remote/use.rs

    // remote_use_block: remote/use.rs

    // remote_block_reacts: remote/use.rs
    // remote_block_template: remote/use.rs

    // remote_place_preview: remote/use.rs

    // remote_use_bucket: remote/use.rs

    // remote_start_held_use: remote/use.rs

    // remote_use_instrument: remote/use.rs

    // remote_use_hands: remote/use.rs

    // remote_complete_using: remote/use.rs

    // remote_send_carried: remote/sync.rs

    // remote_player_attack: remote/ui.rs

    // remote_key: remote/ui.rs

    // remote_click: remote/use.rs

    // remote_overlay: remote/ui.rs

    // remote_status: remote/status.rs
}

/// `Entity.lookAt`'s rotation from `from` towards `to`, as (yaw, pitch).
pub fn look_at_rotation(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    use sunrise_core::geometry::trig::atan2;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // One multiply by `Mth.RAD_TO_DEG` on the double.
    let degrees = |r: f64| r * f64::from(sunrise_core::geometry::trig::RAD_TO_DEG);
    let pitch = wrap_degrees((-degrees(atan2(dy, horizontal))) as f32);
    let yaw = wrap_degrees(degrees(atan2(dz, dx)) as f32 - 90.0);
    (yaw, pitch)
}

/// `PositionMoveRotation.calculateAbsolute`'s pitch: the packet's pitch,
/// added to `source` when it is relative, clamped to +-90.
pub fn position_packet_pitch(source: f32, change: f32, relative: bool) -> f32 {
    let offset = if relative { source } else { 0.0 };
    (offset + change).clamp(-90.0, 90.0)
}

/// `Mth.wrapDegrees`.
pub fn wrap_degrees(a: f32) -> f32 {
    let a = a % 360.0;
    if a >= 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `LivingEntity.tick`'s body target and `tickHeadTurn` for another player:
/// the body turns toward where it walks (backwards when walking back), or to
/// the look while the arm swings, and stays within 50 degrees of the look.
pub fn remote_player_head_turn(m: &mut Mob) {
    let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
    let mut target = m.body_yaw;
    if (dx * dx + dz * dz) as f32 > 0.002_500_000_2 {
        let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
        let facing = (wrap_degrees(m.yaw) - walk).abs();
        target = if 95.0 < facing && facing < 265.0 {
            walk - 180.0
        } else {
            walk
        };
    }
    if m.anim.attack_anim > 0.0 {
        target = m.yaw;
    }
    m.body_yaw += wrap_degrees(target - m.body_yaw) * 0.3;
    let head = wrap_degrees(m.yaw - m.body_yaw);
    if head.abs() > 50.0 {
        m.body_yaw += head - head.signum() * 50.0;
    }
}

/// ServerboundContainerClickPacket's `ContainerInput` ordinal.
pub fn input_ordinal(input: Input) -> i32 {
    match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    }
}

pub mod dry_level;
#[cfg(test)]
pub use dry_level::DryLevel;

pub type DrySound = (
    sunrise_core::position::BlockPos,
    std::sync::Arc<str>,
    mod_api::SoundSource,
    f32,
    f32,
);
/// The blocks a use on the dry level set.
pub type DryChanges = Vec<(sunrise_core::position::BlockPos, BlockStateId)>;
/// What a use on the dry level changed, and the sounds it played.
pub type DryUse = (DryChanges, Vec<DrySound>);
/// Where `BlockItem.place` on the dry level placed, and the blocks it set.
pub struct DryPlace {
    pub pos: sunrise_core::position::BlockPos,
    /// Sound type comes from BlockItem.place's `placedState`, read before setPlacedBy writes.
    pub sound_state: BlockStateId,
    pub matches_candidate: bool,
    pub changes: DryChanges,
}

/// The blocks a guest's hand flips that its use predicts (and hears at once): a wooden or
/// copper door, trapdoor or fence gate, a lever, a button or a cauldron, whose fill level
/// changes as the bucket, bottle or dye lands. An iron door or trapdoor has no hand use.
pub fn hand_operated(behaviours: &sunrise_world::level::Behaviours, state: BlockStateId) -> bool {
    behaviours
        .data(state)
        .is(sunrise_world::level::classes::Class::FenceGate)
        || named_hand_operated(behaviours.table.name(state))
}

pub fn named_hand_operated(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    ((name.ends_with("_door") || name.ends_with("_trapdoor")) && !name.starts_with("iron_"))
        || name.ends_with("_button")
        || name == "lever"
        || name.ends_with("cauldron")
}

/// `InteractionHand` on the wire.
pub fn hand_id(hand: Hand) -> i32 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

/// Whether `Item.use` of `info` ends `Minecraft.startUseItem`'s click, so the
/// off hand gets no turn. `FireworkRocketItem.use` answers SUCCESS while the
/// player glides (the rocket boosts them) and PASS otherwise.
pub fn use_ends_click(info: &sunrise_items::items::ItemInfo, fall_flying: bool) -> bool {
    item_has_use(info) || (fall_flying && &*info.id == "minecraft:firework_rocket")
}

/// Whether `Item.use` of this item answers SUCCESS on this client, which
/// swings (`MultiPlayerGameMode.useItem`'s `result.shouldSwing()`), rather
/// than CONSUME, which only ends the click: a bow, crossbow, trident or
/// spear starting its charge, `ItemUtils.startUsingInstantly`'s food, drink,
/// spyglass, shield and goat horn, and the ender eye's SUCCESS_SERVER (the
/// server's swing source, not this client's).
pub fn remote_use_swings(held: &ItemStack, consumable: bool) -> bool {
    let id = &*held.item;
    !matches!(
        id,
        "minecraft:bow"
            | "minecraft:crossbow"
            | "minecraft:trident"
            | "minecraft:spyglass"
            | "minecraft:goat_horn"
            | "minecraft:ender_eye"
            | "minecraft:brush"
    ) && !consumable
        && sunrise_sim::spear_sim::Kinetic::of(id).is_none()
}

/// Whether an object interaction on the client predicts a hand swing (`InteractionResult.SUCCESS`).
/// Boarding a minecart answers CONSUME and does not swing.
pub fn remote_object_swings(response: Option<bool>, is_boarding: bool) -> bool {
    response.unwrap_or(!is_boarding)
}

/// Whether right-clicking an item frame passes (`InteractionResult.PASS`).
/// A fixed frame or an empty frame when clicking with an empty hand passes.
pub fn item_frame_interact_passes(fixed: bool, frame_has_item: bool, hand_has_item: bool) -> bool {
    fixed || (!frame_has_item && !hand_has_item)
}

/// Whether right-clicking a boat passes (`InteractionResult.PASS`) on a client.
/// `AbstractBoat.interact` passes when sneaking or out of control (60 ticks), and
/// answers SUCCESS otherwise without checking for room (only the server's
/// `startRiding` does). `AbstractChestBoat.interact` opens the chest instead of
/// passing, unless it is out of control, not sneaked at and has room.
pub fn boat_interact_passes(
    chest: bool,
    sneaking: bool,
    out_of_control: bool,
    has_room: bool,
) -> bool {
    if chest {
        !sneaking && out_of_control && has_room
    } else {
        sneaking || out_of_control
    }
}

/// Whether `Item.use` usually acts on this item (eats, drinks, equips, raises,
/// throws, shoots, fills or places), which ends `Minecraft.startUseItem` at
/// its hand. Tools and materials pass, and the off hand gets its turn. The
/// server decides; this is the client's guess of whether to go on. A carrot or warped fungus
/// on a stick is not here: `FoodOnAStickItem.use` passes on a client, and only the server's
/// answer to a boost that can run swings the hand.
pub fn item_has_use(info: &sunrise_items::items::ItemInfo) -> bool {
    let id = info.id.trim_start_matches("minecraft:");
    id.ends_with("_spear")
        || info.is("minecraft:fishing_rod") // a content mod's rod too
        || info.consumable.is_some()
        || (info.equip_slot.is_some() && info.swappable)
        || info.blocks_attacks.is_some()
        || sunrise_items::items::bucket_contents(&info.id).is_some()
        || id.ends_with("_boat")
        || id.ends_with("_raft")
        || id.ends_with("_bundle")
        || matches!(
            id,
            "bow"
                | "crossbow"
                | "trident"
                | "spyglass"
                | "snowball"
                | "egg"
                | "blue_egg"
                | "brown_egg"
                | "ender_pearl"
                | "ender_eye"
                | "wind_charge"
                | "splash_potion"
                | "lingering_potion"
                | "experience_bottle"
                | "bucket"
                | "glass_bottle"
                | "map"
                | "writable_book"
                | "written_book"
                | "goat_horn"
                | "bundle"
                | "brush"
                | "knowledge_book"
        )
}

/// The view distance a client asks for: its render distance, which Sunrise
/// lets go past vanilla's 32 (a vanilla server still caps it at its own).
pub fn client_view_distance(render_distance: i32) -> i8 {
    render_distance.clamp(2, i32::from(i8::MAX)) as i8
}

/// `ClientPacketListener.handleBlockEntityData` loads the tag into the existing
/// block entity, so the state the client builds itself stays: copied from `old`
/// into `new`, the block entity decoded from the tag.
pub fn keep_client_state(old: Option<&BlockEntity>, new: &mut BlockEntity) {
    match (old, new) {
        // The turn of a vault's item and a trial spawner's mob (VaultClientData).
        (Some(BlockEntity::Vault(old)), BlockEntity::Vault(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        (Some(BlockEntity::TrialSpawner(old)), BlockEntity::TrialSpawner(new)) => {
            new.spin = old.spin;
            new.o_spin = old.o_spin;
        }
        // A beacon's beam (`BeaconBlockEntity.loadAdditional` reads only the
        // powers): a fresh one draws nothing until the next tick walks it again.
        (Some(BlockEntity::Beacon(old)), BlockEntity::Beacon(new)) => {
            new.sections.clone_from(&old.sections);
            new.checking.clone_from(&old.checking);
            (new.last_check_y, new.levels) = (old.last_check_y, old.levels);
        }
        // `eruptionTick` is set once, when the level is set; an update tag leaves it.
        (Some(BlockEntity::PotentSulfur(old)), BlockEntity::PotentSulfur(new)) => {
            new.eruption_tick = old.eruption_tick;
        }
        _ => {}
    }
}

/// `LocalPlayer.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS)`: the server's
/// flags (1 using, 2 the off hand) start a use the client is not running
/// (`startUsingItem`, which needs an item in that hand), and stop one it is
/// (`stopUsingItem`). `using` is the slot in use and its ticks.
pub fn synced_local_use(
    using: Option<(usize, u32)>,
    flags: i8,
    selected: usize,
    has_item: impl Fn(usize) -> bool,
) -> Option<(usize, u32)> {
    let server_using = flags & 1 != 0;
    match using {
        None if server_using => {
            let slot = if flags & 2 != 0 {
                crate::game::held_items::OFFHAND_SLOT
            } else {
                selected
            };
            has_item(slot).then_some((slot, 0))
        }
        Some(_) if !server_using => None,
        using => using,
    }
}

pub fn object_skin_changed(published: &HashMap<String, String>, key: &str, skin: &str) -> bool {
    published.get(key).is_none_or(|previous| previous != skin)
}

/// `AttributeInstance.calculateValue` before the attribute's range: the base,
/// `ADD_VALUE`s, then `ADD_MULTIPLIED_BASE`s of that, then each
/// `ADD_MULTIPLIED_TOTAL`.
pub fn snapshot_value(s: &play::AttributeSnapshot) -> f64 {
    let amounts = |op: i32| {
        s.modifiers
            .iter()
            .filter(move |m| m.operation == op)
            .map(|m| m.amount)
    };
    let base = s.base + amounts(0).sum::<f64>();
    let mut value = base + amounts(1).map(|a| base * a).sum::<f64>();
    for amount in amounts(2) {
        value *= 1.0 + amount;
    }
    value
}

/// `Entity.isOnFire` for the window's player: while the server's flag is set
/// the predicted tick's countdown never takes `fire_ticks` below 1.
pub fn hold_server_fire(fire_ticks: i32, server_on_fire: bool) -> i32 {
    if server_on_fire {
        fire_ticks.max(1)
    } else {
        fire_ticks
    }
}

/// `Entity.isColliding`: whether the collision `shape` of a block at `pos`
/// meets `entity`'s box. Shapes are block-local, so they move onto the block
/// first (`VoxelShape.move`), and any box that overlaps is a collision
/// (`Shapes.joinIsNotEmpty` with `BooleanOp.AND`).
pub fn shape_meets(
    shape: &sunrise_core::geometry::VoxelShape,
    pos: [i32; 3],
    entity: sunrise_core::geometry::Aabb,
) -> bool {
    let at = Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    shape
        .boxes()
        .iter()
        .any(|b| b.move_by(at).intersects(entity))
}

/// `ClientLevel.syncBlockState`'s snap: a restored block that meets the player
/// sends it back to the position the prediction remembered, unless the player
/// was teleported after that prediction was made
/// (`lastTeleportSequence < sequence`: the remembered position is stale).
pub fn snap_target(
    recorded: [f64; 3],
    last_teleport_sequence: i32,
    sequence: i32,
    meets: bool,
) -> Option<[f64; 3]> {
    (meets && last_teleport_sequence < sequence).then_some(recorded)
}

/// [`Remote::has_level`]: logged in (a Login arrived) and no reconfiguration
/// since (`reconfiguring` is set by StartConfiguration, cleared by the next
/// Login).
pub fn client_has_level(logged_in: bool, reconfiguring: Option<u64>) -> bool {
    logged_in && reconfiguring.is_none()
}

// A packet's removals (`RemoveEntities`), apart: this file may not grow.

#[cfg(any())]
#[cfg(test)]
pub mod client_camera_tests;
/// `Entity.baseTick`'s fluid update for a mob this window only displays (`fluids`, sampled before
/// the tick), with the splash it draws, and the block it stands on.
pub fn client_fluid_step(
    m: &mut sunrise_sim::mobs::Mob,
    fluids: Option<sunrise_sim::mobs::living::Fluids>,
    standing: Option<String>,
    own: Option<i32>,
    fx: &mut Vec<sunrise_sim::mobs::MobEvent>,
) {
    if let Some(fluids) = fluids {
        let was = m.fluids.in_water;
        let first = m.first_tick;
        // Entity.baseTick saves the preceding eye sample before the new fluid update.
        m.was_eye_in_water = m.fluids.eye_in_water;
        m.fluids = fluids;
        // Entity.doWaterSplashEffect on the client copy. Particles
        // only: ClientLevel draws addParticle, and playSeededSound
        // drops Entity.playSound (the server broadcasts that).
        if m.fluids.in_water && !was && !first && Some(m.id as i32) != own {
            sunrise_sim::mobs::living::client_water_splash(m, fx);
        }
        m.first_tick = false;
    }
    if let Some(name) = standing {
        m.standing_on = Some(name);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod frozen_screen_effects_tests;
#[cfg(test)]
pub mod rider_tick_tests;

#[allow(unused_imports)]
pub use inventory::{inventory_screen_slot, inventory_screen_slot_with_type, remote_station_kind};
