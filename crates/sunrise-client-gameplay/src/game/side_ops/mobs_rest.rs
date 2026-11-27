#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_indexed_mob_rider_slots<'a>(
        &'a self,
        mobs: &JoinList<Mob>,
        id: u32,
    ) -> Option<Ref<'a, [usize]>> {
        None
    }
    fn side_cart_rail_direction(
        game: &Self::Game<'_>,
        cart: Option<u32>,
        state: BlockStateId,
        pos: BlockPos,
        text: &str,
        vanilla: minecarts::Shape,
    ) -> minecarts::Shape {
        game.client_cart_rail_direction(cart, state, pos, text, vanilla)
    }
    fn side_remote_general_type_slot(&self, id: u32) -> Option<usize> {
        self.remote
            .as_ref()?
            .general
            .rows
            .get(&(id as i32))
            .map(|row| row.type_slot)
    }
    fn side_move_guest_entity_id(&self, conn: u32) -> Option<u32> {
        None
    }
    fn side_boat_client_controlled(game: &Self::Game<'_>, id: u32) -> bool {
        game.controls_boat(id)
    }
    fn side_remote_general_rotation(&self, id: u32) -> Option<[f32; 2]> {
        self.remote
            .as_ref()?
            .general
            .rows
            .get(&(id as i32))
            .map(|row| row.rotation)
    }

    fn side_passenger_riding_position(
        game: &Self::Game<'_>,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        game.client_passenger_riding_position(vehicle, me, own_seat, index)
    }
    fn side_position_riders(game: &mut Self::Game<'_>) {
        {
            game.client_position_riders();
        }
    }
    fn side_general_passenger_position(
        game: &Self::Game<'_>,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3> {
        game.client_general_passenger_position(vehicle, rider)
    }
    fn side_play_player_bound_sound(
        game: &mut Self::Game<'_>,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        {
            game.client_play_player_bound_sound(event, source, volume, pitch, except_actor);
        }
    }
    fn side_player_equipped(game: &mut Self::Game<'_>, slot: &str, new: Option<&ItemStack>) {
        {
            game.client_player_equipped(slot, new);
        }
    }
    fn side_boat_can_simulate_movement(game: &Self::Game<'_>, id: u32) -> bool {
        game.client_boat_can_simulate_movement(id)
    }
    fn side_boat_has_room(game: &Self::Game<'_>, i: usize) -> bool {
        game.client_boat_has_room(i)
    }
    fn side_player_boat_seat(game: &Self::Game<'_>, i: usize) -> Vec3 {
        game.client_player_boat_seat(i)
    }
    fn side_tick_boat_leash(game: &mut Self::Game<'_>, b: &mut Boat) {
        {
            game.client_tick_boat_leash(b);
        }
    }
    fn side_finish_body_restitution(
        game: &mut Self::Game<'_>,
        entity: (u32, &str),
        feet: Vec3,
        response: BodyRestitution,
    ) -> Vec3 {
        game.client_finish_body_restitution(entity, feet, response)
    }
    fn side_body_step_on(
        game: &mut Self::Game<'_>,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        game.client_body_step_on(entity, footprint, ground, velocity)
    }
    fn side_item_fall_on(game: &mut Self::Game<'_>, id: u32, on: BlockPos, distance: f64) {
        {
            game.client_item_fall_on(id, on, distance);
        }
    }
    fn side_item_post_move_contacts(
        game: &mut Self::Game<'_>,
        item: &mut ItemEntity,
        client: bool,
        sources: sunrise_gameplay_common::game::entity_common::InsideSources<'_>,
    ) -> bool {
        game.client_item_post_move_contacts(item, client, sources)
    }
    fn side_hurt_lent_item_entity(
        _game: &mut Self::Game<'_>,
        _item: &mut ItemEntity,
        _amount: f32,
        _damage_type: &str,
    ) {
    }
    fn side_queue_boat_contact_hurts(
        _game: &mut Self::Game<'_>,
        _id: u32,
        _hurts: Vec<(&'static str, f32)>,
    ) {
    }
    fn side_apply_boat_contact_hurts(_game: &mut Self::Game<'_>, _id: u32) -> bool {
        false
    }
    fn side_boat_collision_root(game: &Self::Game<'_>, id: u32, target: Target) -> Option<u32> {
        game.client_boat_collision_root(id, target)
    }
    fn side_vehicle_solids(
        game: &Self::Game<'_>,
        mover: Mover<'_>,
        solids: &Solids,
        each: &mut dyn FnMut(&Candidate, Option<usize>),
    ) {
        {
            game.client_vehicle_solids(mover, solids, each);
        }
    }
    fn side_resident_dragon_part_index(game: &Self::Game<'_>, id: u32) -> Option<usize> {
        game.client_resident_dragon_part_index(id)
    }
    fn side_entity_in(game: &Self::Game<'_>, room: Aabb) -> bool {
        game.client_entity_in(room)
    }
    fn side_pick_knot_fence(game: &Self::Game<'_>, id: u32) -> Option<[i32; 3]> {
        game.client_pick_knot_fence(id)
    }
    fn side_entity_is_taken(game: &Self::Game<'_>, id: u32) -> bool {
        game.client_entity_is_taken(id)
    }
    fn side_own_passenger_index(game: &Self::Game<'_>, mob_id: u32) -> usize {
        game.client_own_passenger_index(mob_id)
    }
    fn side_game_side_fields(
        game: &Self::Game<'_>,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        game.client_game_side_fields()
    }
}
use std::cell::Ref;
use std::collections::HashMap;
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::minecarts;
use sunrise_gameplay_common::game::side_ops::mobs_rest::SideOps;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemEntity;
use sunrise_sim::entity_solids::Candidate;
use sunrise_sim::entity_solids::Solids;
use sunrise_sim::join_seq::JoinList;
use sunrise_sim::mobs::Mob;
use sunrise_sim::motion::BodyRestitution;
use sunrise_world::ground_support::GroundState;
use sunrise_world::registry::BlockStateId;
