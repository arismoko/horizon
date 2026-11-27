#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_redirectable_projectile_tag(&self, kind: &str) -> Option<bool> {
        None
    }
    fn side_remote_owns_bobber_target(&self, id: u32) -> bool {
        false
    }
    fn side_command_hook_rotation(game: &Self::Game<'_>, id: u32) -> Option<[f32; 2]> {
        game.fishing_hook_by_id(id)
            .map(|(hook, _)| [hook.yaw, hook.pitch])
    }
    fn side_fishing_hook_on_ground(game: &Self::Game<'_>, id: u32) -> Option<bool> {
        game.fishing_hook_by_id(id)
            .map(|(hook, _)| hook.ground.on_ground)
    }

    fn side_emit_consume(
        game: &mut Self::Game<'_>,
        consumable: &ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        {
            game.server_emit_consume(consumable, stack, count);
        }
    }
    fn side_move_bobber_response(
        game: &mut Self::Game<'_>,
        id: u32,
        position: Vec3,
        ground: &mut sunrise_world::ground_support::GroundState,
        wanted: Vec3,
        moved: Vec3,
        velocity: &mut Vec3,
    ) {
        {
            game.server_move_bobber_response(id, position, ground, wanted, moved, velocity);
        }
    }
    fn side_consume_held_slot(game: &mut Self::Game<'_>, amount: u32) {
        {
            game.server_consume_held_slot(amount);
        }
    }
    fn side_on_start_using_item(game: &mut Self::Game<'_>, stack: &ItemStack) {
        {
            game.server_on_start_using_item(stack);
        }
    }
    fn side_apply_use_cooldown(game: &mut Self::Game<'_>, stack: &ItemStack) {
        {
            game.server_apply_use_cooldown(stack);
        }
    }
    fn side_item_pick(game: &Self::Game<'_>) -> Option<Hit> {
        game.server_item_pick()
    }
    fn side_bucket_liquid_user_entity_id(game: &Self::Game<'_>) -> Option<u32> {
        game.server_bucket_liquid_user_entity_id()
    }
    fn side_in_rain(game: &mut Self::Game<'_>, at: Vec3, top: f64) -> bool {
        game.server_in_rain(at, top)
    }
    fn side_projectile_player_uuid(game: &mut Self::Game<'_>) -> Option<u128> {
        game.server_projectile_player_uuid()
    }
    fn side_acting_player_ref(game: &Self::Game<'_>) -> Option<mod_api::PlayerRef> {
        game.server_acting_player_ref()
    }
    fn side_destroy_progress_player_ref(game: &Self::Game<'_>) -> Option<mod_api::PlayerRef> {
        game.native_acting_player()
    }
    fn side_current_level_ref(game: &Self::Game<'_>) -> Option<mod_api::LevelRef> {
        game.server_current_level_ref()
    }
}
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Hit;
use sunrise_gameplay_common::game::item_use_common::ItemConsumable;
use sunrise_gameplay_common::game::side_ops::items::SideOps;
use sunrise_items::items::ItemStack;
