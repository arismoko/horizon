#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_command_guest(&self, conn: u32) -> Option<&Guest> {
        self.lan.as_ref()?.hub.guest(conn)
    }
    fn side_command_guest_motion(&self, conn: u32) -> Option<&PlayerMotion> {
        <Self as SideOps>::side_command_guest(self, conn)?
            .motion
            .as_deref()
    }
    fn side_command_guest_tick_count(&self, conn: u32) -> Option<i32> {
        let guest = <Self as SideOps>::side_command_guest(self, conn)?;
        Some(
            guest
                .motion
                .as_deref()
                .map(|motion| motion.health.tick_count)
                .or(guest.carried_tick_count)
                .unwrap_or(0),
        )
    }
    fn side_other_entity_kind(&self, id: u32) -> Option<&str> {
        self.others
            .0
            .iter()
            .find(|other| other.id == id)
            .map(|other| other.kind.as_str())
    }
    fn side_other_entity_rotation(&self, id: u32) -> Option<[f32; 2]> {
        self.others
            .0
            .iter()
            .find(|other| other.id == id)
            .map(|other| [other.yaw, other.pitch])
    }
    fn side_filtered_command_player_target(
        game: &Self::Game<'_>,
        uuid: [i32; 4],
    ) -> Option<Target> {
        game.player(sunrise_sim::lan::playerdata::ints_uuid(uuid))
            .and_then(|player| game.native_entity_target(player.entity()))
    }
    fn side_remote_player_name(&self, id: u32) -> Option<&str> {
        None
    }

    fn side_wear_glider(game: &mut Self::Game<'_>, points: u32) {
        game.server_wear_glider(points)
    }
    fn side_on_inventory_item_added(game: &mut Self::Game<'_>, stack: &ItemStack) {
        game.server_on_inventory_item_added(stack)
    }
    fn side_reset_attack_strength(game: &mut Self::Game<'_>) {
        game.server_reset_attack_strength()
    }
    fn side_main_hand_attack_speed(game: &Self::Game<'_>) -> f64 {
        game.server_main_hand_attack_speed()
    }
    fn side_known_player_movement(game: &Self::Game<'_>) -> Vec3 {
        game.server_known_player_movement()
    }
    fn side_apply_equipment(game: &mut Self::Game<'_>) {
        game.server_apply_equipment()
    }
    fn side_may_build(game: &Self::Game<'_>) -> bool {
        game.server_may_build()
    }
    fn side_instabuild(game: &Self::Game<'_>) -> bool {
        game.server_instabuild()
    }
    fn side_player_info_game_mode(game: &Self::Game<'_>) -> Option<GameMode> {
        game.server_player_info_game_mode()
    }
    fn side_player_abilities(game: &Self::Game<'_>, player: PlayerRef) -> Option<PlayerAbilities> {
        game.server_player_abilities(player)
    }
    fn side_own_entity_id(game: &Self::Game<'_>) -> i32 {
        game.server_own_entity_id()
    }
    fn side_own_profile_uuid(game: &Self::Game<'_>) -> Option<u128> {
        game.server_own_profile_uuid()
    }
    fn side_own_profile_name(game: &Self::Game<'_>) -> String {
        game.server_own_profile_name()
    }
    fn side_local_player(game: &Self::Game<'_>) -> Option<PlayerRef> {
        game.server_local_player()
    }
    fn side_remove_local_player_body(game: &mut Self::Game<'_>) {
        game.server_remove_local_player_body()
    }
    fn side_bind_local_player_residency(game: &mut Self::Game<'_>) {
        game.server_bind_local_player_residency()
    }
    fn side_aim(game: &Self::Game<'_>) -> Option<([f64; 3], [f64; 3])> {
        game.server_aim()
    }
    fn side_pick(game: &Self::Game<'_>) -> Option<Hit> {
        game.server_pick()
    }
    fn side_pick_unfiltered(game: &Self::Game<'_>) -> Option<Hit> {
        game.server_pick_unfiltered()
    }
    fn side_mob_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])> {
        game.server_mob_hit()
    }
    fn side_block_distance(game: &Self::Game<'_>) -> Option<f64> {
        game.server_block_distance()
    }
    fn side_frame_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.server_frame_hit()
    }
    fn side_boat_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.server_boat_hit()
    }
    fn side_cart_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.server_cart_hit()
    }
    fn side_armor_stand_hit(game: &Self::Game<'_>) -> Option<(usize, f64, [f64; 3])> {
        game.server_armor_stand_hit()
    }
    fn side_painting_hit(game: &Self::Game<'_>) -> Option<(usize, f64)> {
        game.server_painting_hit()
    }
    fn side_dragon_hit(game: &Self::Game<'_>) -> Option<(u32, f64)> {
        game.server_dragon_hit()
    }
    fn side_uses_stored_pick(game: &Self::Game<'_>) -> bool {
        game.server_uses_stored_pick()
    }
    fn side_pick_shares_vehicle(game: &Self::Game<'_>, id: u32) -> bool {
        game.server_pick_shares_vehicle(id)
    }
    fn side_refresh_client_pick(game: &mut Self::Game<'_>, partial: f32) {
        game.server_refresh_client_pick(partial)
    }
    fn side_release_tick_targets(game: &mut Self::Game<'_>) {
        game.server_release_tick_targets()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn side_set_test_forced_hit(game: &mut Self::Game<'_>, hit: Option<Hit>) {
        game.server_set_test_forced_hit(hit)
    }
    fn side_reset_client_item_use(game: &mut Self::Game<'_>) {
        game.server_reset_client_item_use()
    }
    fn side_tick_client_item_use(game: &mut Self::Game<'_>) {
        game.server_tick_client_item_use()
    }
    fn side_client_display_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.server_client_display_position(entity)
    }
    fn side_entity_position(game: &Self::Game<'_>, entity: EntityRef) -> Option<[f64; 3]> {
        game.server_entity_position(entity)
    }
    fn side_client_copy_box(game: &Self::Game<'_>, id: u32, position: Vec3, vanilla: Aabb) -> Aabb {
        game.server_client_copy_box(id, position, vanilla)
    }
    fn side_native_entity_read_target(game: &Self::Game<'_>, entity: EntityRef) -> Option<Target> {
        game.server_native_entity_read_target(entity)
    }
    fn side_level(game: &Self::Game<'_>, dimension: &str) -> Option<LevelRef> {
        game.server_level(dimension)
    }
    fn side_native_current_level(game: &Self::Game<'_>) -> Option<LevelRef> {
        game.server_native_current_level()
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
        game.server_move_remote_interaction(id, position, has_path, rotation, packet, snap_old)
    }
    fn side_move_remote_hook(
        game: &mut Self::Game<'_>,
        id: u32,
        position: [f64; 3],
        on_ground: bool,
    ) -> bool {
        game.server_move_remote_hook(id, position, on_ground)
    }
    fn side_has_remote_text_display(game: &Self::Game<'_>, id: i32) -> bool {
        game.server_has_remote_text_display(id)
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

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
