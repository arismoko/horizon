#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_command_guest(&self, conn: u32) -> Option<&Guest> {
        None
    }
    fn side_command_guest_motion(&self, conn: u32) -> Option<&PlayerMotion> {
        None
    }
    fn side_command_guest_tick_count(&self, conn: u32) -> Option<i32> {
        None
    }
    fn side_other_entity_kind(&self, id: u32) -> Option<&str> {
        None
    }
    fn side_other_entity_rotation(&self, id: u32) -> Option<[f32; 2]> {
        None
    }
    fn side_filtered_command_player_target(
        game: &Self::Game<'_>,
        uuid: [i32; 4],
    ) -> Option<Target> {
        None
    }
    fn side_remote_player_name(&self, id: u32) -> Option<&str> {
        let r = self.remote.as_ref()?;
        r.level_player_name(id as i32, &r.player_uuid(id)?)
    }

    fn side_wear_glider(game: &mut Self::Game<'_>, points: u32) {
        game.client_wear_glider(points)
    }
    fn side_on_inventory_item_added(game: &mut Self::Game<'_>, stack: &ItemStack) {
        game.client_on_inventory_item_added(stack)
    }
    fn side_reset_attack_strength(game: &mut Self::Game<'_>) {
        game.client_reset_attack_strength()
    }
    fn side_main_hand_attack_speed(game: &Self::Game<'_>) -> f64 {
        game.client_main_hand_attack_speed()
    }
    fn side_known_player_movement(game: &Self::Game<'_>) -> Vec3 {
        game.client_known_player_movement()
    }
    fn side_apply_equipment(game: &mut Self::Game<'_>) {
        game.client_apply_equipment()
    }
    fn side_may_build(game: &Self::Game<'_>) -> bool {
        game.client_may_build()
    }
    fn side_instabuild(game: &Self::Game<'_>) -> bool {
        game.client_instabuild()
    }
    fn side_player_info_game_mode(game: &Self::Game<'_>) -> Option<GameMode> {
        game.client_player_info_game_mode()
    }
    fn side_player_abilities(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerAbilities> {
        game.client_player_abilities(player)
    }
    fn side_own_entity_id(game: &Self::Game<'_>) -> i32 {
        game.client_own_entity_id()
    }
    fn side_own_profile_uuid(game: &Self::Game<'_>) -> Option<u128> {
        game.client_own_profile_uuid()
    }
    fn side_own_profile_name(game: &Self::Game<'_>) -> String {
        game.client_own_profile_name()
    }
    fn side_local_player(game: &Self::Game<'_>) -> Option<PlayerRef> {
        game.client_local_player()
    }
    fn side_remove_local_player_body(game: &mut Self::Game<'_>) {
        game.client_remove_local_player_body()
    }
    fn side_bind_local_player_residency(game: &mut Self::Game<'_>) {
        game.client_bind_local_player_residency()
    }
    fn side_aim(game: &Self::Game<'_>) -> Option<([f64; 3], [f64; 3])> {
        game.client_aim()
    }
    fn side_pick(game: &Self::Game<'_>) -> Option<Hit> {
        game.client_pick()
    }
    fn side_pick_unfiltered(game: &Self::Game<'_>) -> Option<Hit> {
        game.client_pick_unfiltered()
    }
    fn side_mob_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])> {
        game.client_mob_hit()
    }
    fn side_block_distance(game: &Self::Game<'_>) -> Option<f64> {
        game.client_block_distance()
    }
    fn side_frame_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.client_frame_hit()
    }
    fn side_boat_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.client_boat_hit()
    }
    fn side_cart_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.client_cart_hit()
    }
    fn side_armor_stand_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])> {
        game.client_armor_stand_hit()
    }
    fn side_painting_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.client_painting_hit()
    }
    fn side_dragon_hit(game: &Self::Game<'_>) -> Option<(u32, f64)> {
        game.client_dragon_hit()
    }
    fn side_uses_stored_pick(game: &Self::Game<'_>) -> bool {
        game.client_uses_stored_pick()
    }
    fn side_pick_shares_vehicle(game: &Self::Game<'_>, id: u32) -> bool {
        game.client_pick_shares_vehicle(id)
    }
    fn side_refresh_client_pick(game: &mut Self::Game<'_>, partial: f32) {
        game.client_refresh_client_pick(partial)
    }
    fn side_release_tick_targets(game: &mut Self::Game<'_>) {
        game.client_release_tick_targets()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn side_set_test_forced_hit(game: &mut Self::Game<'_>, hit: Option<Hit>) {
        game.client_set_test_forced_hit(hit)
    }
    fn side_reset_client_item_use(game: &mut Self::Game<'_>) {
        game.client_reset_client_item_use()
    }
    fn side_tick_client_item_use(game: &mut Self::Game<'_>) {
        game.client_tick_client_item_use()
    }
    fn side_client_display_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.client_client_display_position(entity)
    }
    fn side_entity_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.client_entity_position(entity)
    }
    fn side_client_copy_box(game: &Self::Game<'_>, id: u32, position: Vec3, vanilla: Aabb) -> Aabb {
        game.client_client_copy_box(id, position, vanilla)
    }
    fn side_native_entity_read_target(game: &Self::Game<'_>, entity: EntityRef) -> Option<Target> {
        game.client_native_entity_read_target(entity)
    }
    fn side_level(game: &Self::Game<'_>, dimension: &str) -> Option<LevelRef> {
        game.client_level(dimension)
    }
    fn side_native_current_level(game: &Self::Game<'_>) -> Option<LevelRef> {
        game.client_native_current_level()
    }
    fn side_move_remote_interaction(
        game: &mut Self::Game<'_>,
        id: i32,
        position: [f64; 3],
        has_path: bool,
        rotation: Option<(f32, f32)>,
        packet: MovePacket,
        snap_old: Option<PosRot>,
    ) -> bool {
        game.client_move_remote_interaction(id, position, has_path, rotation, packet, snap_old)
    }
    fn side_move_remote_hook(
        game: &mut Self::Game<'_>,
        id: u32,
        position: [f64; 3],
        on_ground: bool,
    ) -> bool {
        game.client_move_remote_hook(id, position, on_ground)
    }
    fn side_has_remote_text_display(game: &Self::Game<'_>, id: i32) -> bool {
        game.client_has_remote_text_display(id)
    }
}
use mod_api::EntityRef;
use mod_api::LevelRef;
use mod_api::PlayerAbilities;
use mod_api::PlayerRef;
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Hit;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::remote::movement::MovePacket;
use sunrise_gameplay_common::game::side_ops::player::SideOps;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::guest::Guest;
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::stepped_interpolation::PosRot;
