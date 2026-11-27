//! Role-dispatched side operations for the `modapi-modding` lane.
use crate::game::Dim;
use crate::game::{commands_common::Target, native_server::PlayerRead};
use core::ops::ControlFlow;
use mod_api::{
    BlockCapabilitySite, BlockEntityRef, BlockFace, BlockSide, CollisionFlags, Compound,
    DatapackRegistrySet, DimensionKey, EntityReadError, EntityRef, EntityResidency, EntityTypeKey,
    EquipmentSlot, ItemEntityRef, LevelReadError, LevelRef, LivingRef, MainHand, MenuRef,
    PlayerRef, SupportType, WorldEpoch, client::first_person::Hand,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use sunrise_assets::resource_source::ResourceSource;
use sunrise_core::identifier::Identifier;
use sunrise_items::items::{ItemStack, Items};
use sunrise_level::world::World;
use sunrise_sim::{lan::PlayerSender, mobs::finalize::EquipmentRead, motion::PlayerMotion};
use sunrise_world::{native_load_context::LoopContext, registry::BlockStateId};

pub enum FunctionRegistry {
    Predicate,
    ItemModifier,
    SlotSource,
}

pub trait ModapiModdingSideGameExt<'__sg>: crate::game::SideGame<'__sg> {
    #[inline(always)]
    fn side_vanilla_data_disabled(&self) -> bool {
        <Self::State as SideOps>::side_vanilla_data_disabled(self.side_state())
    }

    #[inline(always)]
    fn side_allow_custom_components(&self) -> bool {
        <Self::State as SideOps>::side_allow_custom_components(self.side_state())
    }

    #[inline(always)]
    fn side_native_player_body(&self, target: Target) -> Option<&PlayerMotion> {
        <Self::State as SideOps>::side_native_player_body(self, target)
    }

    #[inline(always)]
    fn side_native_client_tracked(&self, entity: EntityRef) -> Option<(&str, [f64; 3])> {
        <Self::State as SideOps>::side_native_client_tracked(self, entity)
    }

    #[inline(always)]
    fn side_native_client_level(&self) -> Option<LevelRef> {
        <Self::State as SideOps>::side_native_client_level(self)
    }

    #[inline(always)]
    fn side_native_level_key(&self) -> DimensionKey {
        <Self::State as SideOps>::side_native_level_key(self)
    }

    #[inline(always)]
    fn side_client_tracked_player_fire_immunity(&self, entity: EntityRef) -> Option<bool> {
        <Self::State as SideOps>::side_client_tracked_player_fire_immunity(self, entity)
    }

    #[inline(always)]
    fn side_entity_body_fire_facts(
        &self,
        entity: EntityRef,
        target: Target,
    ) -> Option<(i32, bool, Option<BuiltinKind>)> {
        <Self::State as SideOps>::side_entity_body_fire_facts(self, entity, target)
    }

    #[inline(always)]
    fn side_client_entity_tick_count(&self, entity: EntityRef) -> Option<i32> {
        <Self::State as SideOps>::side_client_entity_tick_count(self, entity)
    }

    #[inline(always)]
    fn side_client_entity_custom_name(
        &self,
        entity: EntityRef,
    ) -> Option<Option<sunrise_game_data::text::Text>> {
        <Self::State as SideOps>::side_client_entity_custom_name(self, entity)
    }

    #[inline(always)]
    fn side_native_client_entity_body_target(
        &self,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        <Self::State as SideOps>::side_native_client_entity_body_target(self, entity)
    }

    #[inline(always)]
    fn side_native_client_general_type_slot(&self, entity: EntityRef) -> Option<usize> {
        <Self::State as SideOps>::side_native_client_general_type_slot(self, entity)
    }

    #[inline(always)]
    fn side_client_entity_on_fire(&self, entity: EntityRef) -> Option<bool> {
        <Self::State as SideOps>::side_client_entity_on_fire(self.side_state(), entity)
    }

    #[inline(always)]
    fn side_remote_logged_in(&self) -> bool {
        <Self::State as SideOps>::side_remote_logged_in(self.side_state())
    }

    #[inline(always)]
    fn side_level_player_name<'s>(
        &'s self,
        entity: EntityRef,
    ) -> Option<std::result::Result<Option<&'s str>, std::num::TryFromIntError>>
    where
        '__sg: 's,
    {
        <Self::State as SideOps>::side_level_player_name(self.side_state(), entity)
    }

    #[inline(always)]
    fn side_remote_overworld_clock(&self) -> Option<i64> {
        <Self::State as SideOps>::side_remote_overworld_clock(self.side_state())
    }

    #[inline(always)]
    fn side_guest_player_effects<'s>(
        &'s self,
        conn: u32,
    ) -> Option<&'s sunrise_level::effects::PlayerEffects>
    where
        '__sg: 's,
    {
        <Self::State as SideOps>::side_guest_player_effects(self.side_state(), conn)
    }

    #[inline(always)]
    fn side_guest_used_hand(&self, conn: u32) -> Option<sunrise_sim::first_person_sim::Hand> {
        <Self::State as SideOps>::side_guest_used_hand(self.side_state(), conn)
    }

    #[inline(always)]
    fn side_remote_registry_names<'s>(
        &'s self,
    ) -> Option<sunrise_protocol::registry::SyncedNames<'s>>
    where
        '__sg: 's,
    {
        <Self::State as SideOps>::side_remote_registry_names(self.side_state())
    }

    #[inline(always)]
    fn side_server_registry_protocol_id(
        &self,
        registry: &Identifier,
        name: &Identifier,
    ) -> Option<i32> {
        <Self::State as SideOps>::side_server_registry_protocol_id(
            self.side_state(),
            registry,
            name,
        )
    }

    #[inline(always)]
    fn side_registry_sync(&self) -> Option<Arc<sunrise_sim::lan::data::SyncData>> {
        <Self::State as SideOps>::side_registry_sync(self)
    }

    fn side_after_entity_constructing_fault(&mut self, id: u32) {
        <Self::State as SideOps>::side_after_entity_constructing_fault(self, id)
    }

    fn side_bind_constructed_entity_attachments(&mut self, id: u32) {
        <Self::State as SideOps>::side_bind_constructed_entity_attachments(self, id)
    }

    fn side_collision_scan_world(&self, level: LevelRef) -> Option<(&World, bool)> {
        <Self::State as SideOps>::side_collision_scan_world(self, level)
    }

    fn side_crafting_remainder_hook(&self, stack: &ItemStack, remainder: &mut Option<ItemStack>) {
        <Self::State as SideOps>::side_crafting_remainder_hook(self, stack, remainder)
    }

    fn side_server_target_position(&self, target: Target) -> Option<[f64; 3]> {
        <Self::State as SideOps>::side_server_target_position(self, target)
    }

    fn side_entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        <Self::State as SideOps>::side_entity_bounding_box(self, entity)
    }

    fn side_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        <Self::State as SideOps>::side_entity_eye_height(self, entity)
    }

    fn side_entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool> {
        <Self::State as SideOps>::side_entity_is_shift_key_down(self, entity)
    }

    fn side_entity_kind(&self, entity: EntityRef) -> Option<EntityTypeKey> {
        <Self::State as SideOps>::side_entity_kind(self, entity)
    }

    fn side_entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        <Self::State as SideOps>::side_entity_level(self, entity)
    }

    fn side_entity_main_arm(&self, entity: EntityRef) -> Option<MainHand> {
        <Self::State as SideOps>::side_entity_main_arm(self, entity)
    }

    fn side_entity_residency(&self, entity: EntityRef) -> EntityResidency {
        <Self::State as SideOps>::side_entity_residency(self, entity)
    }

    fn side_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        <Self::State as SideOps>::side_entity_velocity(self, entity)
    }

    fn side_item_combat_actor(&self, actor: LivingRef) -> Option<(bool, bool, f64, bool)> {
        <Self::State as SideOps>::side_item_combat_actor(self, actor)
    }

    fn side_item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef> {
        <Self::State as SideOps>::side_item_entity(self, entity)
    }

    fn side_level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool> {
        <Self::State as SideOps>::side_level_block_face_sturdy(self, level, pos, face, support)
    }

    fn side_level_datapack_set(&self, level: LevelRef) -> Option<Arc<DatapackRegistrySet>> {
        <Self::State as SideOps>::side_level_datapack_set(self, level)
    }

    fn side_received_registry_datapacks(&self) -> Option<Arc<DatapackRegistrySet>> {
        <Self::State as SideOps>::side_received_registry_datapacks(self.side_state())
    }

    fn side_level_handle_current(&self, level: LevelRef) -> bool {
        <Self::State as SideOps>::side_level_handle_current(self, level)
    }

    fn side_level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        <Self::State as SideOps>::side_level_is_raining_at(self, level, pos)
    }

    fn side_client_view_world_result(&self, level: LevelRef) -> Result<&World, LevelReadError> {
        <Self::State as SideOps>::side_client_view_world_result(self, level)
    }

    fn side_level_view_world(&self, level: LevelRef) -> Option<&World> {
        <Self::State as SideOps>::side_level_view_world(self, level)
    }

    fn side_level_visit_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> std::result::Result<ControlFlow<()>, LevelReadError> {
        <Self::State as SideOps>::side_level_visit_entities_in(self, level, min, max, visit)
    }

    fn side_lid_openers(&self) -> Option<&HashMap<(i32, i32, i32), i32>> {
        <Self::State as SideOps>::side_lid_openers(self)
    }

    fn side_lifecycle_open_at(&mut self, epoch: WorldEpoch) {
        <Self::State as SideOps>::side_lifecycle_open_at(self, epoch)
    }

    fn side_lifecycle_world_closed(&mut self) {
        <Self::State as SideOps>::side_lifecycle_world_closed(self)
    }

    fn side_lifecycle_world_ready(&mut self) {
        <Self::State as SideOps>::side_lifecycle_world_ready(self)
    }

    fn side_list_native_mods(&mut self) {
        <Self::State as SideOps>::side_list_native_mods(self)
    }

    fn side_living_absorption(&self, living: LivingRef) -> Option<f32> {
        <Self::State as SideOps>::side_living_absorption(self, living)
    }

    fn side_living_attribute_value(&self, living: LivingRef, id: &str) -> Option<f64> {
        <Self::State as SideOps>::side_living_attribute_value(self, living, id)
    }

    fn side_living_equipment_stack(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'_>> {
        <Self::State as SideOps>::side_living_equipment_stack(self, living, slot)
    }

    fn side_living_read_target(&self, living: LivingRef) -> Option<Target> {
        <Self::State as SideOps>::side_living_read_target(self, living)
    }

    fn side_living_target_alive(&self, id: u32) -> bool {
        <Self::State as SideOps>::side_living_target_alive(self, id)
    }

    fn side_living_use_ticks(&self, living: LivingRef) -> Option<Option<u32>> {
        <Self::State as SideOps>::side_living_use_ticks(self, living)
    }

    fn side_living_used_item_hand(&self, living: LivingRef) -> Option<Hand> {
        <Self::State as SideOps>::side_living_used_item_hand(self, living)
    }

    fn side_menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>> {
        <Self::State as SideOps>::side_menu_carried(self, menu)
    }

    fn side_menu_kind(&self, menu: MenuRef) -> Option<Option<String>> {
        <Self::State as SideOps>::side_menu_kind(self, menu)
    }

    fn side_menu_slot(&self, menu: MenuRef, slot: usize) -> Option<Option<ItemStack>> {
        <Self::State as SideOps>::side_menu_slot(self, menu, slot)
    }

    fn side_menu_state_id(&self, menu: MenuRef) -> Option<i32> {
        <Self::State as SideOps>::side_menu_state_id(self, menu)
    }

    fn side_menu_slot_count(&self, menu: MenuRef) -> Option<usize> {
        <Self::State as SideOps>::side_menu_slot_count(self, menu)
    }

    fn side_pick_section_move(&mut self, id: u32, to: [f64; 3]) {
        <Self::State as SideOps>::side_pick_section_move(self, id, to)
    }

    fn side_pick_section_remove(&mut self, id: u32) {
        <Self::State as SideOps>::side_pick_section_remove(self, id)
    }

    fn side_pick_section_sync_moved_id(&mut self, id: u32) {
        <Self::State as SideOps>::side_pick_section_sync_moved_id(self, id)
    }

    fn side_player(&self, uuid: u128) -> Option<PlayerRef> {
        <Self::State as SideOps>::side_player(self, uuid)
    }

    fn side_player_client_brand(&self, player: PlayerRef) -> Option<&str> {
        <Self::State as SideOps>::side_player_client_brand(self, player)
    }

    fn side_player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> std::result::Result<Option<CollisionFlags>, LevelReadError> {
        <Self::State as SideOps>::side_player_collision_flags(self, player)
    }

    fn side_player_container_menu(&self, player: PlayerRef) -> Option<MenuRef> {
        <Self::State as SideOps>::side_player_container_menu(self, player)
    }

    fn side_player_destroy_progress(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<f32> {
        <Self::State as SideOps>::side_player_destroy_progress(self, player, level, pos, state)
    }

    fn side_player_dimension_key(&self, player: PlayerRef) -> Option<DimensionKey> {
        <Self::State as SideOps>::side_player_dimension_key(self, player)
    }

    fn side_player_has_correct_tool_for_drops(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool> {
        <Self::State as SideOps>::side_player_has_correct_tool_for_drops(
            self, player, level, pos, state,
        )
    }

    fn side_player_read_both(&self, player: PlayerRef) -> Option<PlayerRead<'_>> {
        <Self::State as SideOps>::side_player_read_both(self, player)
    }

    fn side_retire_disabled_instance_groups(&self, effective: &BTreeSet<String>) {
        <Self::State as SideOps>::side_retire_disabled_instance_groups(self, effective)
    }

    fn side_set_audio_loop_context(&mut self, audio: Option<LoopContext>) {
        <Self::State as SideOps>::side_set_audio_loop_context(self, audio)
    }

    fn side_signal_block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        <Self::State as SideOps>::side_signal_block_entity(self, level, pos)
    }

    fn side_signal_block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        <Self::State as SideOps>::side_signal_block_entity_data(self, entity)
    }

    fn side_read_level_capability_site(
        &self,
        world: &World,
        level: LevelRef,
        at: BlockSide,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite> {
        <Self::State as SideOps>::side_read_level_capability_site(
            self,
            world,
            level,
            at,
            entity_removed,
        )
    }

    fn side_signal_capability_site(&self, at: BlockSide) -> Option<BlockCapabilitySite> {
        <Self::State as SideOps>::side_signal_capability_site(self, at)
    }

    fn side_signal_container_size(&self, world: &World, at: BlockSide) -> Option<usize> {
        <Self::State as SideOps>::side_signal_container_size(self, world, at)
    }

    fn side_signal_lend_container_stack(
        &self,
        world: &World,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        <Self::State as SideOps>::side_signal_lend_container_stack(self, world, at, slot, read)
    }

    fn side_tick_uses_server_sections(&self) -> bool {
        <Self::State as SideOps>::side_tick_uses_server_sections(self)
    }

    fn side_world_table_items(&self) -> Option<&Arc<Items>> {
        <Self::State as SideOps>::side_world_table_items(self)
    }

    fn side_level_ref(&self, epoch: WorldEpoch, key: DimensionKey) -> LevelRef {
        <Self::State as SideOps>::side_level_ref(self, epoch, key)
    }

    fn side_online_player_sender(&self, player: PlayerRef) -> Option<PlayerSender> {
        <Self::State as SideOps>::side_online_player_sender(self, player)
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> ModapiModdingSideGameExt<'__sg> for T {}

// Generated by Shape G codemod from side_* role dispatchers.
pub trait SideOps: Sized {
    type Game<'g>;

    fn side_visit_parked_dimensions(&self, visit: impl FnMut(Dim));
    fn side_open_server_data_resource<'a>(
        &'a self,
        logical_server: bool,
        content: &'a ResourceSource,
        vanilla: Option<&'a ResourceSource>,
        id: &mod_api::Identifier,
    ) -> std::io::Result<Option<Box<dyn std::io::Read + 'a>>>;
    fn side_append_function_pack_sources<'a>(&'a self, to: &mut Vec<&'a ResourceSource>);
    fn side_function_registry(
        &self,
        registry: FunctionRegistry,
    ) -> Option<&BTreeMap<String, serde_json::Value>>;
    fn side_slot_source_tag_contains(&self, tag: &str, id: &str) -> bool;
    fn side_vanilla_data_disabled(&self) -> bool;
    fn side_allow_custom_components(&self) -> bool;
    fn side_native_player_body<'a>(
        game: &'a Self::Game<'_>,
        target: Target,
    ) -> Option<&'a PlayerMotion>;
    fn side_native_client_tracked<'g>(
        game: &'g Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<(&'g str, [f64; 3])>;
    fn side_native_client_level(game: &Self::Game<'_>) -> Option<LevelRef>;
    fn side_native_level_key(game: &Self::Game<'_>) -> DimensionKey;
    fn side_client_tracked_player_fire_immunity(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<bool>;
    fn side_entity_body_fire_facts(
        game: &Self::Game<'_>,
        entity: EntityRef,
        target: Target,
    ) -> Option<(i32, bool, Option<BuiltinKind>)>;
    fn side_client_entity_tick_count(game: &Self::Game<'_>, entity: EntityRef) -> Option<i32>;
    fn side_client_entity_custom_name(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<Option<sunrise_game_data::text::Text>>;
    fn side_native_client_entity_body_target(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError>;
    fn side_native_client_general_type_slot(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<usize>;
    fn side_client_entity_on_fire(&self, entity: EntityRef) -> Option<bool>;
    fn side_remote_logged_in(&self) -> bool;
    fn side_level_player_name(
        &self,
        entity: EntityRef,
    ) -> Option<std::result::Result<Option<&str>, std::num::TryFromIntError>>;
    fn side_remote_overworld_clock(&self) -> Option<i64>;
    fn side_guest_player_effects(
        &self,
        conn: u32,
    ) -> Option<&sunrise_level::effects::PlayerEffects>;
    fn side_guest_used_hand(&self, conn: u32) -> Option<sunrise_sim::first_person_sim::Hand>;
    fn side_remote_registry_names(&self) -> Option<sunrise_protocol::registry::SyncedNames<'_>>;
    fn side_server_registry_protocol_id(
        &self,
        registry: &Identifier,
        name: &Identifier,
    ) -> Option<i32>;
    fn side_registry_sync(game: &Self::Game<'_>) -> Option<Arc<sunrise_sim::lan::data::SyncData>>;

    fn side_after_entity_constructing_fault(game: &mut Self::Game<'_>, id: u32);
    fn side_bind_constructed_entity_attachments(game: &mut Self::Game<'_>, id: u32);
    fn side_collision_scan_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<(&'__side_g World, bool)>;
    fn side_crafting_remainder_hook(
        game: &Self::Game<'_>,
        stack: &ItemStack,
        remainder: &mut Option<ItemStack>,
    );
    fn side_server_target_position(game: &Self::Game<'_>, target: Target) -> Option<[f64; 3]>;
    fn side_entity_bounding_box(game: &Self::Game<'_>, entity: EntityRef) -> Option<[[f64; 3]; 2]>;
    fn side_entity_eye_height(game: &Self::Game<'_>, entity: EntityRef) -> Option<f64>;
    fn side_entity_is_shift_key_down(game: &Self::Game<'_>, entity: EntityRef) -> Option<bool>;
    fn side_entity_kind(game: &Self::Game<'_>, entity: EntityRef) -> Option<EntityTypeKey>;
    fn side_entity_level(game: &Self::Game<'_>, entity: EntityRef) -> Option<LevelRef>;
    fn side_entity_main_arm(game: &Self::Game<'_>, entity: EntityRef) -> Option<MainHand>;
    fn side_entity_residency(game: &Self::Game<'_>, entity: EntityRef) -> EntityResidency;
    fn side_entity_velocity(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]>;
    fn side_item_combat_actor(
        game: &Self::Game<'_>,
        actor: LivingRef,
    ) -> Option<(bool, bool, f64, bool)>;
    fn side_item_entity(game: &Self::Game<'_>, entity: EntityRef) -> Option<ItemEntityRef>;
    fn side_level_block_face_sturdy(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool>;
    fn side_level_datapack_set(
        game: &Self::Game<'_>,
        level: LevelRef,
    ) -> Option<Arc<DatapackRegistrySet>>;
    fn side_received_registry_datapacks(&self) -> Option<Arc<DatapackRegistrySet>> {
        None
    }
    fn side_level_handle_current(game: &Self::Game<'_>, level: LevelRef) -> bool;
    fn side_level_is_raining_at(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool>;
    fn side_client_view_world_result<'a>(
        game: &'a Self::Game<'_>,
        level: LevelRef,
    ) -> Result<&'a World, LevelReadError>;
    fn side_level_view_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<&'__side_g World>;
    fn side_level_visit_entities_in(
        game: &Self::Game<'_>,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> std::result::Result<ControlFlow<()>, LevelReadError>;
    fn side_lid_openers<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g HashMap<(i32, i32, i32), i32>>;
    fn side_lifecycle_open_at(game: &mut Self::Game<'_>, epoch: WorldEpoch);
    fn side_lifecycle_world_closed(game: &mut Self::Game<'_>);
    fn side_lifecycle_world_ready(game: &mut Self::Game<'_>);
    fn side_list_native_mods(game: &mut Self::Game<'_>);
    fn side_living_absorption(game: &Self::Game<'_>, living: LivingRef) -> Option<f32>;
    fn side_living_attribute_value(
        game: &Self::Game<'_>,
        living: LivingRef,
        id: &str,
    ) -> Option<f64>;
    fn side_living_equipment_stack<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'__side_g>>;
    fn side_living_read_target(game: &Self::Game<'_>, living: LivingRef) -> Option<Target>;
    fn side_living_target_alive(game: &Self::Game<'_>, id: u32) -> bool;
    fn side_living_use_ticks(game: &Self::Game<'_>, living: LivingRef) -> Option<Option<u32>>;
    fn side_living_used_item_hand(game: &Self::Game<'_>, living: LivingRef) -> Option<Hand>;
    fn side_menu_carried(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<ItemStack>>;
    fn side_menu_kind(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<String>>;
    fn side_menu_slot(
        game: &Self::Game<'_>,
        menu: MenuRef,
        slot: usize,
    ) -> Option<Option<ItemStack>>;
    fn side_menu_state_id(game: &Self::Game<'_>, menu: MenuRef) -> Option<i32>;
    fn side_menu_slot_count(game: &Self::Game<'_>, menu: MenuRef) -> Option<usize>;
    fn side_pick_section_move(game: &mut Self::Game<'_>, id: u32, to: [f64; 3]);
    fn side_pick_section_remove(game: &mut Self::Game<'_>, id: u32);
    fn side_pick_section_sync_moved_id(game: &mut Self::Game<'_>, id: u32);
    fn side_player(game: &Self::Game<'_>, uuid: u128) -> Option<PlayerRef>;
    fn side_player_client_brand<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<&'__side_g str>;
    fn side_player_collision_flags(
        game: &Self::Game<'_>,
        player: PlayerRef,
    ) -> std::result::Result<Option<CollisionFlags>, LevelReadError>;
    fn side_player_container_menu(game: &Self::Game<'_>, player: PlayerRef) -> Option<MenuRef>;
    fn side_player_destroy_progress(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<f32>;
    fn side_player_dimension_key(game: &Self::Game<'_>, player: PlayerRef) -> Option<DimensionKey>;
    fn side_player_has_correct_tool_for_drops(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool>;
    fn side_player_read_both<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<PlayerRead<'__side_g>>;
    fn side_retire_disabled_instance_groups(game: &Self::Game<'_>, effective: &BTreeSet<String>);
    fn side_set_audio_loop_context(game: &mut Self::Game<'_>, audio: Option<LoopContext>);
    fn side_signal_block_entity(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockEntityRef>;
    fn side_signal_block_entity_data(
        game: &Self::Game<'_>,
        entity: BlockEntityRef,
    ) -> Option<Compound>;
    fn side_read_level_capability_site(
        game: &Self::Game<'_>,
        world: &World,
        level: LevelRef,
        at: BlockSide,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite>;
    fn side_signal_capability_site(
        game: &Self::Game<'_>,
        at: BlockSide,
    ) -> Option<BlockCapabilitySite>;
    fn side_signal_container_size(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
    ) -> Option<usize>;
    fn side_signal_lend_container_stack(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    );
    fn side_tick_uses_server_sections(game: &Self::Game<'_>) -> bool;
    fn side_world_table_items<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g Arc<Items>>;
    fn side_level_ref(game: &Self::Game<'_>, epoch: WorldEpoch, key: DimensionKey) -> LevelRef;
    fn side_online_player_sender(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerSender>;
}

use crate::game::native_entity_base::BuiltinKind;
