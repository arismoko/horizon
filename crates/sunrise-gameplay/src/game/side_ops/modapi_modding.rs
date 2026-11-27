#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_visit_parked_dimensions(&self, visit: impl FnMut(Dim)) {
        self.levels.0.parked.keys().copied().for_each(visit);
    }
    fn side_open_server_data_resource<'a>(
        &'a self,
        logical_server: bool,
        content: &'a ResourceSource,
        vanilla: Option<&'a ResourceSource>,
        id: &mod_api::Identifier,
    ) -> std::io::Result<Option<Box<dyn std::io::Read + 'a>>> {
        let selected = logical_server
            .then(|| self.fns.0.enabled_packs_highest_first())
            .into_iter()
            .flatten();
        sunrise_level::datapack::open_resource(
            selected,
            std::iter::once(content).chain(vanilla),
            id,
        )
    }
    fn side_append_function_pack_sources<'a>(&'a self, to: &mut Vec<&'a ResourceSource>) {
        to.extend(
            self.fns
                .0
                .enabled_packs()
                .map(sunrise_level::datapack::DataPack::source),
        );
    }
    fn side_function_registry(
        &self,
        registry: FunctionRegistry,
    ) -> Option<&BTreeMap<String, serde_json::Value>> {
        Some(match registry {
            FunctionRegistry::Predicate => &self.fns.0.store.predicates,
            FunctionRegistry::ItemModifier => &self.fns.0.store.item_modifiers,
            FunctionRegistry::SlotSource => &self.fns.0.store.slot_sources,
        })
    }
    fn side_slot_source_tag_contains(&self, tag: &str, id: &str) -> bool {
        self.fns
            .0
            .store
            .slot_source_tag(tag)
            .is_ok_and(|members| members.iter().any(|member| member == id))
    }
    fn side_allow_custom_components(&self) -> bool {
        true
    }
    fn side_vanilla_data_disabled(&self) -> bool {
        self.cmds.0.vanilla_disabled
    }
    fn side_native_player_body<'a>(
        game: &'a Self::Game<'_>,
        target: Target,
    ) -> Option<&'a PlayerMotion> {
        game.native_player_body(target)
    }
    fn side_native_client_tracked<'g>(
        game: &'g Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<(&'g str, [f64; 3])> {
        None
    }
    fn side_native_client_level(game: &Self::Game<'_>) -> Option<LevelRef> {
        None
    }
    fn side_native_level_key(game: &Self::Game<'_>) -> DimensionKey {
        game.dim.key()
    }
    fn side_client_tracked_player_fire_immunity(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<bool> {
        None
    }
    fn side_entity_body_fire_facts(
        game: &Self::Game<'_>,
        _entity: EntityRef,
        target: Target,
    ) -> Option<(i32, bool, Option<BuiltinKind>)> {
        let view = game.read_entity_base(target)?;
        Some((view.base.fire_ticks, view.fire_immune, view.kind))
    }
    fn side_client_entity_tick_count(game: &Self::Game<'_>, entity: EntityRef) -> Option<i32> {
        None
    }
    fn side_client_entity_custom_name(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<Option<sunrise_game_data::text::Text>> {
        None
    }

    fn side_native_client_entity_body_target(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        Err(EntityReadError::Unavailable)
    }

    fn side_native_client_general_type_slot(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<usize> {
        None
    }
    fn side_client_entity_on_fire(&self, entity: EntityRef) -> Option<bool> {
        None
    }
    fn side_remote_logged_in(&self) -> bool {
        false
    }
    fn side_level_player_name(
        &self,
        entity: EntityRef,
    ) -> Option<std::result::Result<Option<&str>, std::num::TryFromIntError>> {
        None
    }
    fn side_remote_overworld_clock(&self) -> Option<i64> {
        None
    }
    fn side_guest_player_effects(
        &self,
        conn: u32,
    ) -> Option<&sunrise_level::effects::PlayerEffects> {
        Some(&self.lan.as_ref()?.hub.guest(conn)?.effects)
    }
    fn side_guest_used_hand(&self, conn: u32) -> Option<sunrise_sim::first_person_sim::Hand> {
        self.lan.as_ref()?.play.get(&conn)?.used_hand()
    }
    fn side_remote_registry_names(&self) -> Option<sunrise_protocol::registry::SyncedNames<'_>> {
        None
    }
    fn side_server_registry_protocol_id(
        &self,
        registry: &Identifier,
        name: &Identifier,
    ) -> Option<i32> {
        self.lan.as_ref()?.hub.synced_id(registry, name)
    }
    fn side_registry_sync(game: &Self::Game<'_>) -> Option<Arc<sunrise_sim::lan::data::SyncData>> {
        game.server_registry_sync()
    }

    fn side_after_entity_constructing_fault(_game: &mut Self::Game<'_>, id: u32) {
        #[cfg(any(test, feature = "test-support"))]
        {
            use crate::game::pending_births::escaping_fault::{At, reach};
            reach(At::Constructed, id);
        }
    }

    fn side_bind_constructed_entity_attachments(game: &mut Self::Game<'_>, id: u32) {
        {
            game.server_bind_constructed_entity_attachments(id);
        }
    }
    fn side_collision_scan_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<(&'__side_g World, bool)> {
        game.server_collision_scan_world(level)
    }
    fn side_crafting_remainder_hook(
        game: &Self::Game<'_>,
        stack: &ItemStack,
        remainder: &mut Option<ItemStack>,
    ) {
        {
            game.server_crafting_remainder_hook(stack, remainder);
        }
    }
    fn side_server_target_position(game: &Self::Game<'_>, target: Target) -> Option<[f64; 3]> {
        game.target_position(target)
    }
    fn side_entity_bounding_box(game: &Self::Game<'_>, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        game.server_entity_bounding_box(entity)
    }
    fn side_entity_eye_height(game: &Self::Game<'_>, entity: EntityRef) -> Option<f64> {
        game.server_entity_eye_height(entity)
    }
    fn side_entity_is_shift_key_down(game: &Self::Game<'_>, entity: EntityRef) -> Option<bool> {
        game.server_entity_is_shift_key_down(entity)
    }
    fn side_entity_kind(game: &Self::Game<'_>, entity: EntityRef) -> Option<EntityTypeKey> {
        game.server_entity_kind(entity)
    }
    fn side_entity_level(game: &Self::Game<'_>, entity: EntityRef) -> Option<LevelRef> {
        game.server_entity_level(entity)
    }
    fn side_entity_main_arm(game: &Self::Game<'_>, entity: EntityRef) -> Option<MainHand> {
        game.server_entity_main_arm(entity)
    }
    fn side_entity_residency(game: &Self::Game<'_>, entity: EntityRef) -> EntityResidency {
        game.server_entity_residency(entity)
    }
    fn side_entity_velocity(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.server_entity_velocity(entity)
    }
    fn side_item_combat_actor(
        game: &Self::Game<'_>,
        actor: LivingRef,
    ) -> Option<(bool, bool, f64, bool)> {
        game.server_item_combat_actor(actor)
    }
    fn side_item_entity(game: &Self::Game<'_>, entity: EntityRef) -> Option<ItemEntityRef> {
        game.server_item_entity(entity)
    }
    fn side_level_block_face_sturdy(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool> {
        game.server_level_block_face_sturdy(level, pos, face, support)
    }
    fn side_level_datapack_set(
        game: &Self::Game<'_>,
        level: LevelRef,
    ) -> Option<Arc<DatapackRegistrySet>> {
        game.server_level_datapack_set(level)
    }
    fn side_level_handle_current(game: &Self::Game<'_>, level: LevelRef) -> bool {
        game.server_level_handle_current(level)
    }
    fn side_level_is_raining_at(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        game.server_level_is_raining_at(level, pos)
    }
    fn side_client_view_world_result<'a>(
        game: &'a Self::Game<'_>,
        level: LevelRef,
    ) -> Result<&'a World, LevelReadError> {
        Err(LevelReadError::Unavailable)
    }
    fn side_level_view_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<&'__side_g World> {
        game.server_level_view_world(level)
    }
    fn side_level_visit_entities_in(
        game: &Self::Game<'_>,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> std::result::Result<ControlFlow<()>, LevelReadError> {
        game.server_level_visit_entities_in(level, min, max, visit)
    }
    fn side_lid_openers<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g HashMap<(i32, i32, i32), i32>> {
        game.server_lid_openers()
    }
    fn side_lifecycle_open_at(game: &mut Self::Game<'_>, epoch: WorldEpoch) {
        game.server_lifecycle_open_at(epoch);
    }
    fn side_lifecycle_world_closed(game: &mut Self::Game<'_>) {
        {
            game.server_lifecycle_world_closed();
        }
    }
    fn side_lifecycle_world_ready(game: &mut Self::Game<'_>) {
        {
            game.server_lifecycle_world_ready();
        }
    }
    fn side_list_native_mods(game: &mut Self::Game<'_>) {
        {}
    }
    fn side_living_absorption(game: &Self::Game<'_>, living: LivingRef) -> Option<f32> {
        game.server_living_absorption(living)
    }
    fn side_living_attribute_value(
        game: &Self::Game<'_>,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        game.server_living_attribute_value(living, id)
    }
    fn side_living_equipment_stack<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'__side_g>> {
        game.server_living_equipment_stack(living, slot)
    }
    fn side_living_read_target(game: &Self::Game<'_>, living: LivingRef) -> Option<Target> {
        game.server_living_read_target(living)
    }
    fn side_living_target_alive(game: &Self::Game<'_>, id: u32) -> bool {
        game.server_living_target_alive(id)
    }
    fn side_living_use_ticks(game: &Self::Game<'_>, living: LivingRef) -> Option<Option<u32>> {
        game.server_living_use_ticks(living)
    }
    fn side_living_used_item_hand(game: &Self::Game<'_>, living: LivingRef) -> Option<Hand> {
        game.server_living_used_item_hand(living)
    }
    fn side_menu_carried(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<ItemStack>> {
        game.server_menu_carried(menu)
    }
    fn side_menu_kind(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<String>> {
        game.server_menu_kind(menu)
    }
    fn side_menu_slot(
        game: &Self::Game<'_>,
        menu: MenuRef,
        slot: usize,
    ) -> Option<Option<ItemStack>> {
        game.server_menu_slot(menu, slot)
    }
    fn side_menu_state_id(game: &Self::Game<'_>, menu: MenuRef) -> Option<i32> {
        game.menu_state_id(menu)
    }
    fn side_menu_slot_count(game: &Self::Game<'_>, menu: MenuRef) -> Option<usize> {
        game.server_menu_slot_count(menu)
    }
    fn side_pick_section_move(game: &mut Self::Game<'_>, id: u32, to: [f64; 3]) {
        {
            game.server_pick_section_move(id, to);
        }
    }
    fn side_pick_section_remove(game: &mut Self::Game<'_>, id: u32) {
        {
            game.server_pick_section_remove(id);
        }
    }
    fn side_pick_section_sync_moved_id(game: &mut Self::Game<'_>, id: u32) {
        {
            game.server_pick_section_sync_moved_id(id);
        }
    }
    fn side_player(game: &Self::Game<'_>, uuid: u128) -> Option<PlayerRef> {
        game.server_player(uuid)
    }
    fn side_player_client_brand<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<&'__side_g str> {
        game.server_player_client_brand(player)
    }
    fn side_player_collision_flags(
        game: &Self::Game<'_>,
        player: PlayerRef,
    ) -> std::result::Result<Option<CollisionFlags>, LevelReadError> {
        game.server_player_collision_flags(player)
    }
    fn side_player_container_menu(game: &Self::Game<'_>, player: PlayerRef) -> Option<MenuRef> {
        game.server_player_container_menu(player)
    }
    fn side_player_destroy_progress(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<f32> {
        game.server_player_destroy_progress(player, level, pos, state)
    }
    fn side_player_dimension_key(game: &Self::Game<'_>, player: PlayerRef) -> Option<DimensionKey> {
        game.server_player_dimension_key(player)
    }
    fn side_player_has_correct_tool_for_drops(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool> {
        game.server_player_has_correct_tool_for_drops(player, level, pos, state)
    }
    fn side_player_read_both<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<PlayerRead<'__side_g>> {
        game.server_player_read_both(player)
    }
    fn side_retire_disabled_instance_groups(game: &Self::Game<'_>, effective: &BTreeSet<String>) {
        {
            game.server_retire_disabled_instance_groups(effective);
        }
    }
    fn side_set_audio_loop_context(game: &mut Self::Game<'_>, audio: Option<LoopContext>) {
        {
            game.server_set_audio_loop_context(audio);
        }
    }
    fn side_signal_block_entity(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockEntityRef> {
        game.server_signal_block_entity(level, pos)
    }
    fn side_signal_block_entity_data(
        game: &Self::Game<'_>,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        game.server_signal_block_entity_data(entity)
    }
    fn side_read_level_capability_site(
        game: &Self::Game<'_>,
        world: &World,
        level: LevelRef,
        at: BlockSide,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite> {
        let mobs = game.signal_level_mobs(level)?;
        let [x, y, z] = at.pos;
        game.level_containers(world, mobs)
            .capability_site(at, world.block(x, y, z), entity_removed)
    }
    fn side_signal_capability_site(
        game: &Self::Game<'_>,
        at: BlockSide,
    ) -> Option<BlockCapabilitySite> {
        game.server_signal_capability_site(at)
    }
    fn side_signal_container_size(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
    ) -> Option<usize> {
        game.server_signal_container_size(world, at)
    }
    fn side_signal_lend_container_stack(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        game.server_signal_lend_container_stack(world, at, slot, read)
    }
    fn side_tick_uses_server_sections(game: &Self::Game<'_>) -> bool {
        game.server_tick_uses_server_sections()
    }
    fn side_world_table_items<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g Arc<Items>> {
        game.server_world_table_items()
    }
    fn side_level_ref(game: &Self::Game<'_>, epoch: WorldEpoch, key: DimensionKey) -> LevelRef {
        game.level_ref(epoch, key)
    }
    fn side_online_player_sender(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerSender> {
        game.online_player_record(player).map(|g| g.sender.clone())
    }
}
use core::ops::ControlFlow;
use mod_api::BlockCapabilitySite;
use mod_api::BlockEntityRef;
use mod_api::BlockFace;
use mod_api::BlockSide;
use mod_api::CollisionFlags;
use mod_api::Compound;
use mod_api::DatapackRegistrySet;
use mod_api::DimensionKey;
use mod_api::EntityReadError;
use mod_api::EntityRef;
use mod_api::EntityResidency;
use mod_api::EntityTypeKey;
use mod_api::EquipmentSlot;
use mod_api::ItemEntityRef;
use mod_api::LevelReadError;
use mod_api::LevelRef;
use mod_api::LivingRef;
use mod_api::MainHand;
use mod_api::MenuRef;
use mod_api::PlayerRef;
use mod_api::SupportType;
use mod_api::WorldEpoch;
use mod_api::client::first_person::Hand;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;
use sunrise_assets::resource_source::ResourceSource;
use sunrise_core::identifier::Identifier;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::native_server::PlayerRead;
use sunrise_gameplay_common::game::side_ops::modapi_modding::{FunctionRegistry, SideOps};
use sunrise_items::items::ItemStack;
use sunrise_items::items::Items;
use sunrise_level::world::World;
use sunrise_sim::lan::PlayerSender;
use sunrise_sim::mobs::finalize::EquipmentRead;
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::registry::BlockStateId;

use sunrise_gameplay_common::game::native_entity_base::BuiltinKind;
