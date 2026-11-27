use crate::game::{Hit, commands_common::Target, remote::movement::MovePacket};
use mod_api::{EntityRef, LevelRef, PlayerAbilities, PlayerRef};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_items::items::ItemStack;
use sunrise_sim::{lan::guest::Guest, motion::PlayerMotion};
use sunrise_world::stepped_interpolation::PosRot;

pub trait PlayerSideGameExt<'__sg>: crate::game::SideGame<'__sg> {
    #[inline(always)]
    fn side_filtered_command_player_target(&self, uuid: [i32; 4]) -> Option<Target> {
        <Self::State as SideOps>::side_filtered_command_player_target(self, uuid)
    }

    // Hands, inventory & equipment (6).
    #[inline(always)]
    fn side_wear_glider(&mut self, points: u32) {
        <Self::State as SideOps>::side_wear_glider(self, points)
    }
    #[inline(always)]
    fn side_on_inventory_item_added(&mut self, stack: &ItemStack) {
        <Self::State as SideOps>::side_on_inventory_item_added(self, stack)
    }
    #[inline(always)]
    fn side_reset_attack_strength(&mut self) {
        <Self::State as SideOps>::side_reset_attack_strength(self)
    }
    #[inline(always)]
    fn side_main_hand_attack_speed(&self) -> f64 {
        <Self::State as SideOps>::side_main_hand_attack_speed(self)
    }
    #[inline(always)]
    fn side_known_player_movement(&self) -> Vec3 {
        <Self::State as SideOps>::side_known_player_movement(self)
    }
    #[inline(always)]
    fn side_apply_equipment(&mut self) {
        <Self::State as SideOps>::side_apply_equipment(self)
    }

    // Game mode & abilities (4).
    #[inline(always)]
    fn side_may_build(&self) -> bool {
        <Self::State as SideOps>::side_may_build(self)
    }
    #[inline(always)]
    fn side_instabuild(&self) -> bool {
        <Self::State as SideOps>::side_instabuild(self)
    }
    #[inline(always)]
    fn side_player_info_game_mode(&self) -> Option<GameMode> {
        <Self::State as SideOps>::side_player_info_game_mode(self)
    }
    #[inline(always)]
    fn side_player_abilities(&self, player: PlayerRef) -> Option<PlayerAbilities> {
        <Self::State as SideOps>::side_player_abilities(self, player)
    }

    // Player identity & residency (6).
    #[inline(always)]
    fn side_own_entity_id(&self) -> i32 {
        <Self::State as SideOps>::side_own_entity_id(self)
    }
    #[inline(always)]
    fn side_own_profile_uuid(&self) -> Option<u128> {
        <Self::State as SideOps>::side_own_profile_uuid(self)
    }
    #[inline(always)]
    fn side_own_profile_name(&self) -> String {
        <Self::State as SideOps>::side_own_profile_name(self)
    }
    #[inline(always)]
    fn side_local_player(&self) -> Option<PlayerRef> {
        <Self::State as SideOps>::side_local_player(self)
    }
    #[inline(always)]
    fn side_remove_local_player_body(&mut self) {
        <Self::State as SideOps>::side_remove_local_player_body(self)
    }
    #[inline(always)]
    fn side_bind_local_player_residency(&mut self) {
        <Self::State as SideOps>::side_bind_local_player_residency(self)
    }

    // Aim, raycast & targeting (16; includes 1 #[cfg(test)]).
    #[inline(always)]
    fn side_aim(&self) -> Option<([f64; 3], [f64; 3])> {
        <Self::State as SideOps>::side_aim(self)
    }
    #[inline(always)]
    fn side_pick(&self) -> Option<Hit> {
        <Self::State as SideOps>::side_pick(self)
    }
    #[inline(always)]
    fn side_pick_unfiltered(&self) -> Option<Hit> {
        <Self::State as SideOps>::side_pick_unfiltered(self)
    }
    #[inline(always)]
    fn side_mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        <Self::State as SideOps>::side_mob_hit(self)
    }
    #[inline(always)]
    fn side_block_distance(&self) -> Option<f64> {
        <Self::State as SideOps>::side_block_distance(self)
    }
    #[inline(always)]
    fn side_frame_hit(&self) -> Option<(usize, f64)> {
        <Self::State as SideOps>::side_frame_hit(self)
    }
    #[inline(always)]
    fn side_boat_hit(&self) -> Option<(usize, f64)> {
        <Self::State as SideOps>::side_boat_hit(self)
    }
    #[inline(always)]
    fn side_cart_hit(&self) -> Option<(usize, f64)> {
        <Self::State as SideOps>::side_cart_hit(self)
    }
    #[inline(always)]
    fn side_armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        <Self::State as SideOps>::side_armor_stand_hit(self)
    }
    #[inline(always)]
    fn side_painting_hit(&self) -> Option<(usize, f64)> {
        <Self::State as SideOps>::side_painting_hit(self)
    }
    #[inline(always)]
    fn side_dragon_hit(&self) -> Option<(u32, f64)> {
        <Self::State as SideOps>::side_dragon_hit(self)
    }
    #[inline(always)]
    fn side_uses_stored_pick(&self) -> bool {
        <Self::State as SideOps>::side_uses_stored_pick(self)
    }
    #[inline(always)]
    fn side_pick_shares_vehicle(&self, id: u32) -> bool {
        <Self::State as SideOps>::side_pick_shares_vehicle(self, id)
    }
    #[inline(always)]
    fn side_refresh_client_pick(&mut self, partial: f32) {
        <Self::State as SideOps>::side_refresh_client_pick(self, partial)
    }
    #[allow(dead_code)]
    #[inline(always)]
    fn side_release_tick_targets(&mut self) {
        <Self::State as SideOps>::side_release_tick_targets(self)
    }
    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    fn side_set_test_forced_hit(&mut self, hit: Option<Hit>) {
        <Self::State as SideOps>::side_set_test_forced_hit(self, hit)
    }

    // Client item-use hooks from common remote handlers (2).
    #[inline(always)]
    fn side_reset_client_item_use(&mut self) {
        <Self::State as SideOps>::side_reset_client_item_use(self)
    }
    #[inline(always)]
    fn side_tick_client_item_use(&mut self) {
        <Self::State as SideOps>::side_tick_client_item_use(self)
    }

    // Dual-role entity & level lookups (6).
    #[inline(always)]
    fn side_client_display_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        <Self::State as SideOps>::side_client_display_position(self, entity)
    }
    #[inline(always)]
    fn side_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        <Self::State as SideOps>::side_entity_position(self, entity)
    }
    #[inline(always)]
    fn side_client_copy_box(&self, id: u32, position: Vec3, vanilla: Aabb) -> Aabb {
        <Self::State as SideOps>::side_client_copy_box(self, id, position, vanilla)
    }
    #[inline(always)]
    fn side_native_entity_read_target(&self, entity: EntityRef) -> Option<Target> {
        <Self::State as SideOps>::side_native_entity_read_target(self, entity)
    }
    #[inline(always)]
    fn side_level(&self, dimension: &str) -> Option<LevelRef> {
        <Self::State as SideOps>::side_level(self, dimension)
    }
    #[inline(always)]
    fn side_native_current_level(&self) -> Option<LevelRef> {
        <Self::State as SideOps>::side_native_current_level(self)
    }

    // Client-only entity motion helpers called by common remote/motion.rs (3).
    #[inline(always)]
    fn side_move_remote_interaction(
        &mut self,
        id: i32,
        position: [f64; 3],
        has_path: bool,
        rotation: Option<(f32, f32)>,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) -> bool {
        <Self::State as SideOps>::side_move_remote_interaction(
            self, id, position, has_path, rotation, packet, snap_old,
        )
    }
    #[inline(always)]
    fn side_move_remote_hook(&mut self, id: u32, position: [f64; 3], on_ground: bool) -> bool {
        <Self::State as SideOps>::side_move_remote_hook(self, id, position, on_ground)
    }
    #[inline(always)]
    fn side_has_remote_text_display(&self, id: i32) -> bool {
        <Self::State as SideOps>::side_has_remote_text_display(self, id)
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> PlayerSideGameExt<'__sg> for T {}

// Generated by Shape G codemod from side_* role dispatchers.
pub trait SideOps: Sized {
    type Game<'g>;

    fn side_command_guest(&self, conn: u32) -> Option<&Guest>;
    fn side_command_guest_motion(&self, conn: u32) -> Option<&PlayerMotion>;
    fn side_command_guest_tick_count(&self, conn: u32) -> Option<i32>;
    fn side_other_entity_kind(&self, id: u32) -> Option<&str>;
    fn side_other_entity_rotation(&self, id: u32) -> Option<[f32; 2]>;
    fn side_filtered_command_player_target(game: &Self::Game<'_>, uuid: [i32; 4])
    -> Option<Target>;
    fn side_remote_player_name(&self, id: u32) -> Option<&str>;

    fn side_wear_glider(game: &mut Self::Game<'_>, points: u32);
    fn side_on_inventory_item_added(game: &mut Self::Game<'_>, stack: &ItemStack);
    fn side_reset_attack_strength(game: &mut Self::Game<'_>);
    fn side_main_hand_attack_speed(game: &Self::Game<'_>) -> f64;
    fn side_known_player_movement(game: &Self::Game<'_>) -> Vec3;
    fn side_apply_equipment(game: &mut Self::Game<'_>);
    fn side_may_build(game: &Self::Game<'_>) -> bool;
    fn side_instabuild(game: &Self::Game<'_>) -> bool;
    fn side_player_info_game_mode(game: &Self::Game<'_>) -> Option<GameMode>;
    fn side_player_abilities(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerAbilities>;
    fn side_own_entity_id(game: &Self::Game<'_>) -> i32;
    fn side_own_profile_uuid(game: &Self::Game<'_>) -> Option<u128>;
    fn side_own_profile_name(game: &Self::Game<'_>) -> String;
    fn side_local_player(game: &Self::Game<'_>) -> Option<PlayerRef>;
    fn side_remove_local_player_body(game: &mut Self::Game<'_>);
    fn side_bind_local_player_residency(game: &mut Self::Game<'_>);
    fn side_aim(game: &Self::Game<'_>) -> Option<([f64; 3], [f64; 3])>;
    fn side_pick(game: &Self::Game<'_>) -> Option<Hit>;
    fn side_pick_unfiltered(game: &Self::Game<'_>) -> Option<Hit>;
    fn side_mob_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])>;
    fn side_block_distance(game: &Self::Game<'_>) -> Option<f64>;
    fn side_frame_hit(game: &Self::Game<'_>) -> Option<(usize, f64)>;
    fn side_boat_hit(game: &Self::Game<'_>) -> Option<(usize, f64)>;
    fn side_cart_hit(game: &Self::Game<'_>) -> Option<(usize, f64)>;
    fn side_armor_stand_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])>;
    fn side_painting_hit(game: &Self::Game<'_>) -> Option<(usize, f64)>;
    fn side_dragon_hit(game: &Self::Game<'_>) -> Option<(u32, f64)>;
    fn side_uses_stored_pick(game: &Self::Game<'_>) -> bool;
    fn side_pick_shares_vehicle(game: &Self::Game<'_>, id: u32) -> bool;
    fn side_refresh_client_pick(game: &mut Self::Game<'_>, partial: f32);
    fn side_release_tick_targets(game: &mut Self::Game<'_>);
    // A default body: this crate can have the feature on while the side crate that implements it does not.
    #[cfg(any(test, feature = "test-support"))]
    fn side_set_test_forced_hit(game: &mut Self::Game<'_>, hit: Option<Hit>) {
        let _ = (game, hit);
    }
    fn side_reset_client_item_use(game: &mut Self::Game<'_>);
    fn side_tick_client_item_use(game: &mut Self::Game<'_>);
    fn side_client_display_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]>;
    fn side_entity_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]>;
    fn side_client_copy_box(game: &Self::Game<'_>, id: u32, position: Vec3, vanilla: Aabb) -> Aabb;
    fn side_native_entity_read_target(game: &Self::Game<'_>, entity: EntityRef) -> Option<Target>;
    fn side_level(game: &Self::Game<'_>, dimension: &str) -> Option<LevelRef>;
    fn side_native_current_level(game: &Self::Game<'_>) -> Option<LevelRef>;
    fn side_move_remote_interaction(
        game: &mut Self::Game<'_>,
        id: i32,
        position: [f64; 3],
        has_path: bool,
        rotation: Option<(f32, f32)>,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) -> bool;
    fn side_move_remote_hook(
        game: &mut Self::Game<'_>,
        id: u32,
        position: [f64; 3],
        on_ground: bool,
    ) -> bool;
    fn side_has_remote_text_display(game: &Self::Game<'_>, id: i32) -> bool;
}
