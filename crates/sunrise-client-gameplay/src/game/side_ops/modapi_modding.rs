#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_visit_parked_dimensions(&self, visit: impl FnMut(Dim)) {}
    fn side_open_server_data_resource<'a>(
        &'a self,
        logical_server: bool,
        content: &'a ResourceSource,
        vanilla: Option<&'a ResourceSource>,
        id: &mod_api::Identifier,
    ) -> std::io::Result<Option<Box<dyn std::io::Read + 'a>>> {
        sunrise_level::datapack::open_resource(
            std::iter::empty(),
            std::iter::once(content).chain(vanilla),
            id,
        )
    }
    fn side_append_function_pack_sources<'a>(&'a self, to: &mut Vec<&'a ResourceSource>) {}
    fn side_function_registry(
        &self,
        registry: FunctionRegistry,
    ) -> Option<&BTreeMap<String, serde_json::Value>> {
        None
    }
    fn side_slot_source_tag_contains(&self, tag: &str, id: &str) -> bool {
        false
    }
    fn side_vanilla_data_disabled(&self) -> bool {
        false
    }
    fn side_allow_custom_components(&self) -> bool {
        self.remote
            .as_ref()
            .is_none_or(|remote| remote.registries.custom_components)
    }
    fn side_native_player_body<'a>(
        game: &'a Self::Game<'_>,
        target: Target,
    ) -> Option<&'a PlayerMotion> {
        match target {
            Target::Player => game.common.player.as_ref(),
            _ => None,
        }
    }
    fn side_native_client_tracked<'g>(
        game: &'g Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<(&'g str, [f64; 3])> {
        game.native_client_tracked(entity)
    }
    fn side_native_client_level(game: &Self::Game<'_>) -> Option<LevelRef> {
        game.client_level_handle()
    }
    fn side_native_level_key(game: &Self::Game<'_>) -> DimensionKey {
        game.client
            .remote
            .as_ref()
            .map_or_else(|| game.dim.key(), |remote| remote.level_key)
    }
    fn side_client_tracked_player_fire_immunity(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<bool> {
        game.client_tracked_player_fire_immunity(entity)
    }
    fn side_entity_body_fire_facts(
        game: &Self::Game<'_>,
        entity: EntityRef,
        target: Target,
    ) -> Option<(i32, bool, Option<BuiltinKind>)> {
        game.client_entity_body_fire_facts(entity, target)
    }
    fn side_client_entity_tick_count(game: &Self::Game<'_>, entity: EntityRef) -> Option<i32> {
        game.client_entity_tick_count(entity)
    }
    fn side_client_entity_custom_name(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<Option<sunrise_game_data::text::Text>> {
        use crate::game::client_side::text_displays::Shows;

        let (remote, id) = game.native_client_entity_context(entity).ok()?;
        if remote
            .tracked(id)
            .is_some_and(|(kind, _)| Shows::from_kind(kind).is_some())
            && let Some(display) = game.client.text_displays.0.get(&id)
        {
            return Some(display.custom_name.clone());
        }
        remote
            .general
            .rows
            .get(&id)
            .map(|row| row.custom_name.clone())
    }

    fn side_native_client_entity_body_target(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Result<Target, EntityReadError> {
        game.native_client_entity_body_target(entity)
    }

    fn side_native_client_general_type_slot(
        game: &Self::Game<'_>,
        entity: EntityRef,
    ) -> Option<usize> {
        game.native_client_general(entity).map(|row| row.type_slot)
    }
    fn side_client_entity_on_fire(&self, entity: EntityRef) -> Option<bool> {
        self.remote
            .as_ref()?
            .entity_on_fire(i32::try_from(entity.id()).ok()?)
    }
    fn side_remote_logged_in(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| remote.logged_in)
    }
    fn side_level_player_name(
        &self,
        entity: EntityRef,
    ) -> Option<std::result::Result<Option<&str>, std::num::TryFromIntError>> {
        let remote = self.remote.as_ref()?;
        Some(
            i32::try_from(entity.id()).map(|id| remote.level_player(id).and_then(|(_, name)| name)),
        )
    }
    fn side_remote_overworld_clock(&self) -> Option<i64> {
        self.remote.as_ref().map(|r| r.overworld_clock_time())
    }
    fn side_guest_player_effects(
        &self,
        conn: u32,
    ) -> Option<&sunrise_level::effects::PlayerEffects> {
        None
    }
    fn side_guest_used_hand(&self, conn: u32) -> Option<sunrise_sim::first_person_sim::Hand> {
        None
    }
    fn side_remote_registry_names(&self) -> Option<sunrise_protocol::registry::SyncedNames<'_>> {
        let remote = self.remote.as_ref()?;
        Some(sunrise_protocol::registry::SyncedNames {
            builtin: &remote.builtin,
            dynamic: &remote.registries,
        })
    }
    fn side_server_registry_protocol_id(
        &self,
        registry: &Identifier,
        name: &Identifier,
    ) -> Option<i32> {
        None
    }
    fn side_registry_sync(game: &Self::Game<'_>) -> Option<Arc<sunrise_sim::lan::data::SyncData>> {
        None
    }

    fn side_after_entity_constructing_fault(_game: &mut Self::Game<'_>, _id: u32) {}

    fn side_bind_constructed_entity_attachments(game: &mut Self::Game<'_>, id: u32) {
        {
            game.client_bind_constructed_entity_attachments(id);
        }
    }
    fn side_collision_scan_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<(&'__side_g World, bool)> {
        game.client_collision_scan_world(level)
    }
    fn side_crafting_remainder_hook(
        game: &Self::Game<'_>,
        stack: &ItemStack,
        remainder: &mut Option<ItemStack>,
    ) {
        {
            game.client_crafting_remainder_hook(stack, remainder);
        }
    }
    fn side_server_target_position(_game: &Self::Game<'_>, _target: Target) -> Option<[f64; 3]> {
        None
    }
    fn side_entity_bounding_box(game: &Self::Game<'_>, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        game.client_entity_bounding_box(entity)
    }
    fn side_entity_eye_height(game: &Self::Game<'_>, entity: EntityRef) -> Option<f64> {
        game.client_entity_eye_height(entity)
    }
    fn side_entity_is_shift_key_down(game: &Self::Game<'_>, entity: EntityRef) -> Option<bool> {
        game.client_entity_is_shift_key_down(entity)
    }
    fn side_entity_kind(game: &Self::Game<'_>, entity: EntityRef) -> Option<EntityTypeKey> {
        game.client_entity_kind(entity)
    }
    fn side_entity_level(game: &Self::Game<'_>, entity: EntityRef) -> Option<LevelRef> {
        game.client_entity_level(entity)
    }
    fn side_entity_main_arm(game: &Self::Game<'_>, entity: EntityRef) -> Option<MainHand> {
        game.client_entity_main_arm(entity)
    }
    fn side_entity_residency(game: &Self::Game<'_>, entity: EntityRef) -> EntityResidency {
        game.client_entity_residency(entity)
    }
    fn side_entity_velocity(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.client_entity_velocity(entity)
    }
    fn side_item_combat_actor(
        game: &Self::Game<'_>,
        actor: LivingRef,
    ) -> Option<(bool, bool, f64, bool)> {
        game.client_item_combat_actor(actor)
    }
    fn side_item_entity(game: &Self::Game<'_>, entity: EntityRef) -> Option<ItemEntityRef> {
        game.client_item_entity(entity)
    }
    fn side_level_block_face_sturdy(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool> {
        game.client_level_block_face_sturdy(level, pos, face, support)
    }
    fn side_level_datapack_set(
        game: &Self::Game<'_>,
        level: LevelRef,
    ) -> Option<Arc<DatapackRegistrySet>> {
        game.client_level_datapack_set(level)
    }
    fn side_received_registry_datapacks(&self) -> Option<Arc<DatapackRegistrySet>> {
        self.remote.as_ref()?.datapacks.received().cloned()
    }
    fn side_level_handle_current(game: &Self::Game<'_>, level: LevelRef) -> bool {
        game.client_level_handle_current(level)
    }
    fn side_level_is_raining_at(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        game.client_level_is_raining_at(level, pos)
    }
    fn side_client_view_world_result<'a>(
        game: &'a Self::Game<'_>,
        level: LevelRef,
    ) -> Result<&'a World, LevelReadError> {
        game.client_view_world_result(level)
    }
    fn side_level_view_world<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        level: LevelRef,
    ) -> Option<&'__side_g World> {
        game.client_level_view_world(level)
    }
    fn side_level_visit_entities_in(
        game: &Self::Game<'_>,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> std::result::Result<ControlFlow<()>, LevelReadError> {
        game.client_level_visit_entities_in(level, min, max, visit)
    }
    fn side_lid_openers<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g HashMap<(i32, i32, i32), i32>> {
        game.client_lid_openers()
    }
    fn side_lifecycle_open_at(game: &mut Self::Game<'_>, epoch: WorldEpoch) {
        game.client_lifecycle_open_at(epoch);
    }
    fn side_lifecycle_world_closed(game: &mut Self::Game<'_>) {
        {
            game.client_lifecycle_world_closed();
        }
    }
    fn side_lifecycle_world_ready(game: &mut Self::Game<'_>) {
        {
            game.client_lifecycle_world_ready();
        }
    }
    fn side_list_native_mods(game: &mut Self::Game<'_>) {
        {
            game.list_native_mods();
        }
    }
    fn side_living_absorption(game: &Self::Game<'_>, living: LivingRef) -> Option<f32> {
        game.client_living_absorption(living)
    }
    fn side_living_attribute_value(
        game: &Self::Game<'_>,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        game.client_living_attribute_value(living, id)
    }
    fn side_living_equipment_stack<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'__side_g>> {
        game.client_living_equipment_stack(living, slot)
    }
    fn side_living_read_target(game: &Self::Game<'_>, living: LivingRef) -> Option<Target> {
        game.client_living_read_target(living)
    }
    fn side_living_target_alive(game: &Self::Game<'_>, id: u32) -> bool {
        game.client_living_target_alive(id)
    }
    fn side_living_use_ticks(game: &Self::Game<'_>, living: LivingRef) -> Option<Option<u32>> {
        game.client_living_use_ticks(living)
    }
    fn side_living_used_item_hand(game: &Self::Game<'_>, living: LivingRef) -> Option<Hand> {
        game.client_living_used_item_hand(living)
    }
    fn side_menu_carried(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<ItemStack>> {
        game.client_menu_carried(menu)
    }
    fn side_menu_kind(game: &Self::Game<'_>, menu: MenuRef) -> Option<Option<String>> {
        game.client_menu_kind(menu)
    }
    fn side_menu_slot(
        game: &Self::Game<'_>,
        menu: MenuRef,
        slot: usize,
    ) -> Option<Option<ItemStack>> {
        game.client_menu_slot(menu, slot)
    }
    fn side_menu_state_id(game: &Self::Game<'_>, menu: MenuRef) -> Option<i32> {
        game.client_menu_state_id(menu)
    }
    fn side_menu_slot_count(game: &Self::Game<'_>, menu: MenuRef) -> Option<usize> {
        game.client_menu_slot_count(menu)
    }
    fn side_pick_section_move(game: &mut Self::Game<'_>, id: u32, to: [f64; 3]) {
        {
            game.client_pick_section_move(id, to);
        }
    }
    fn side_pick_section_remove(game: &mut Self::Game<'_>, id: u32) {
        {
            game.client_pick_section_remove(id);
        }
    }
    fn side_pick_section_sync_moved_id(game: &mut Self::Game<'_>, id: u32) {
        {
            game.client_pick_section_sync_moved_id(id);
        }
    }
    fn side_player(game: &Self::Game<'_>, uuid: u128) -> Option<PlayerRef> {
        game.client_player(uuid)
    }
    fn side_player_client_brand<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<&'__side_g str> {
        game.client_player_client_brand(player)
    }
    fn side_player_collision_flags(
        game: &Self::Game<'_>,
        player: PlayerRef,
    ) -> std::result::Result<Option<CollisionFlags>, LevelReadError> {
        game.client_player_collision_flags(player)
    }
    fn side_player_container_menu(game: &Self::Game<'_>, player: PlayerRef) -> Option<MenuRef> {
        game.client_player_container_menu(player)
    }
    fn side_player_destroy_progress(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<f32> {
        game.client_player_destroy_progress(player, level, pos, state)
    }
    fn side_player_dimension_key(game: &Self::Game<'_>, player: PlayerRef) -> Option<DimensionKey> {
        game.client_player_dimension_key(player)
    }
    fn side_player_has_correct_tool_for_drops(
        game: &mut Self::Game<'_>,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool> {
        game.client_player_has_correct_tool_for_drops(player, level, pos, state)
    }
    fn side_player_read_both<'__side_g>(
        game: &'__side_g Self::Game<'_>,
        player: PlayerRef,
    ) -> Option<PlayerRead<'__side_g>> {
        game.client_player_read_both(player)
    }
    fn side_retire_disabled_instance_groups(game: &Self::Game<'_>, effective: &BTreeSet<String>) {
        game.client_retire_disabled_instance_groups(effective);
        game.client
            .prepared_items
            .borrow_mut()
            .retire_disabled(effective);
        if let Some(renderer) = &game.client.renderer.0 {
            renderer.retire_disabled_mod_meshes(effective);
            renderer.retire_disabled_mod_uniforms(effective);
            renderer.retire_unloaded_mod_storage(effective);
            renderer.retire_unloaded_mod_readbacks(effective);
            renderer.retire_unloaded_mod_render_targets(effective);
        }
    }
    fn side_set_audio_loop_context(game: &mut Self::Game<'_>, audio: Option<LoopContext>) {
        {
            game.client_set_audio_loop_context(audio);
        }
    }
    fn side_signal_block_entity(
        game: &Self::Game<'_>,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<BlockEntityRef> {
        game.client_signal_block_entity(level, pos)
    }
    fn side_signal_block_entity_data(
        game: &Self::Game<'_>,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        game.client_signal_block_entity_data(entity)
    }
    fn side_read_level_capability_site(
        game: &Self::Game<'_>,
        world: &World,
        level: LevelRef,
        at: BlockSide,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite> {
        (level.dimension() == Self::side_native_level_key(game)).then_some(())?;
        let [x, y, z] = at.pos;
        game.client_level_containers(world).capability_site(
            at,
            world.block(x, y, z),
            entity_removed,
        )
    }
    fn side_signal_capability_site(
        game: &Self::Game<'_>,
        at: BlockSide,
    ) -> Option<BlockCapabilitySite> {
        game.client_signal_capability_site(at)
    }
    fn side_signal_container_size(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
    ) -> Option<usize> {
        game.client_signal_container_size(world, at)
    }
    fn side_signal_lend_container_stack(
        game: &Self::Game<'_>,
        world: &World,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        game.client_signal_lend_container_stack(world, at, slot, read)
    }
    fn side_tick_uses_server_sections(game: &Self::Game<'_>) -> bool {
        game.client_tick_uses_server_sections()
    }
    fn side_world_table_items<'__side_g>(
        game: &'__side_g Self::Game<'_>,
    ) -> Option<&'__side_g Arc<Items>> {
        game.client_world_table_items()
    }
    fn side_level_ref(game: &Self::Game<'_>, epoch: WorldEpoch, key: DimensionKey) -> LevelRef {
        LevelRef::new(epoch, key)
    }
    fn side_online_player_sender(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerSender> {
        None
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
use sunrise_gameplay_common::game::side_ops::modapi_modding::FunctionRegistry;
use sunrise_gameplay_common::game::side_ops::modapi_modding::SideOps;
use sunrise_items::items::ItemStack;
use sunrise_items::items::Items;
use sunrise_level::world::World;
use sunrise_sim::lan::PlayerSender;
use sunrise_sim::mobs::finalize::EquipmentRead;
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::registry::BlockStateId;

use sunrise_gameplay_common::game::native_entity_base::BuiltinKind;
