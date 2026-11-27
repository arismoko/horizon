//! Two-arm side dispatchers owned by lane `mobs-rest`.
use crate::game::minecarts;
use crate::game::{
    boats_common::Boat, collision_source::Mover, commands_common::Target,
    entity_common::InsideSources,
};
use std::cell::Ref;
use std::collections::HashMap;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    entities::ItemEntity,
    entity_solids::{Candidate, Solids},
    motion::BodyRestitution,
};
use sunrise_sim::{join_seq::JoinList, mobs::Mob};
use sunrise_world::ground_support::GroundState;
use sunrise_world::registry::BlockStateId;

pub trait MobsRestSideGameExt<'__sg>: crate::game::SideGame<'__sg> {
    #[inline(always)]
    fn side_remote_general_type_slot(&self, id: u32) -> Option<usize> {
        <Self::State as SideOps>::side_remote_general_type_slot(self.side_state(), id)
    }

    #[inline(always)]
    fn side_move_guest_entity_id(&self, conn: u32) -> Option<u32> {
        <Self::State as SideOps>::side_move_guest_entity_id(self.side_state(), conn)
    }

    fn side_boat_client_controlled(&self, id: u32) -> bool {
        <Self::State as SideOps>::side_boat_client_controlled(self, id)
    }

    #[inline(always)]
    fn side_cart_rail_direction(
        &self,
        cart: Option<u32>,
        state: BlockStateId,
        pos: BlockPos,
        text: &str,
        vanilla: minecarts::Shape,
    ) -> minecarts::Shape {
        <Self::State as SideOps>::side_cart_rail_direction(self, cart, state, pos, text, vanilla)
    }

    fn side_passenger_riding_position(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        <Self::State as SideOps>::side_passenger_riding_position(self, vehicle, me, own_seat, index)
    }

    fn side_position_riders(&mut self) {
        <Self::State as SideOps>::side_position_riders(self)
    }

    fn side_general_passenger_position(&self, vehicle: u32, rider: u32) -> Option<Vec3> {
        <Self::State as SideOps>::side_general_passenger_position(self, vehicle, rider)
    }

    fn side_play_player_bound_sound(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        <Self::State as SideOps>::side_play_player_bound_sound(
            self,
            event,
            source,
            volume,
            pitch,
            except_actor,
        )
    }

    fn side_player_equipped(&mut self, slot: &str, new: Option<&ItemStack>) {
        <Self::State as SideOps>::side_player_equipped(self, slot, new)
    }

    fn side_boat_can_simulate_movement(&self, id: u32) -> bool {
        <Self::State as SideOps>::side_boat_can_simulate_movement(self, id)
    }

    fn side_boat_has_room(&self, i: usize) -> bool {
        <Self::State as SideOps>::side_boat_has_room(self, i)
    }

    fn side_player_boat_seat(&self, i: usize) -> Vec3 {
        <Self::State as SideOps>::side_player_boat_seat(self, i)
    }

    fn side_tick_boat_leash(&mut self, b: &mut Boat) {
        <Self::State as SideOps>::side_tick_boat_leash(self, b)
    }

    fn side_finish_body_restitution(
        &mut self,
        entity: (u32, &str),
        feet: Vec3,
        response: BodyRestitution,
    ) -> Vec3 {
        <Self::State as SideOps>::side_finish_body_restitution(self, entity, feet, response)
    }

    fn side_body_step_on(
        &mut self,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite> {
        <Self::State as SideOps>::side_body_step_on(self, entity, footprint, ground, velocity)
    }

    fn side_item_fall_on(&mut self, id: u32, on: BlockPos, distance: f64) {
        <Self::State as SideOps>::side_item_fall_on(self, id, on, distance)
    }

    fn side_item_post_move_contacts(
        &mut self,
        item: &mut ItemEntity,
        client: bool,
        sources: InsideSources<'_>,
    ) -> bool {
        <Self::State as SideOps>::side_item_post_move_contacts(self, item, client, sources)
    }

    fn side_hurt_lent_item_entity(
        &mut self,
        item: &mut ItemEntity,
        amount: f32,
        damage_type: &str,
    ) {
        <Self::State as SideOps>::side_hurt_lent_item_entity(self, item, amount, damage_type);
    }

    fn side_queue_boat_contact_hurts(&mut self, id: u32, hurts: Vec<(&'static str, f32)>) {
        <Self::State as SideOps>::side_queue_boat_contact_hurts(self, id, hurts);
    }

    fn side_apply_boat_contact_hurts(&mut self, id: u32) -> bool {
        <Self::State as SideOps>::side_apply_boat_contact_hurts(self, id)
    }

    fn side_boat_collision_root(&self, id: u32, target: Target) -> Option<u32> {
        <Self::State as SideOps>::side_boat_collision_root(self, id, target)
    }

    fn side_vehicle_solids(
        &self,
        mover: Mover<'_>,
        solids: &Solids,
        each: &mut dyn FnMut(&Candidate, Option<usize>),
    ) {
        <Self::State as SideOps>::side_vehicle_solids(self, mover, solids, each)
    }

    fn side_resident_dragon_part_index(&self, id: u32) -> Option<usize> {
        <Self::State as SideOps>::side_resident_dragon_part_index(self, id)
    }

    fn side_entity_in(&self, room: Aabb) -> bool {
        <Self::State as SideOps>::side_entity_in(self, room)
    }

    fn side_pick_knot_fence(&self, id: u32) -> Option<[i32; 3]> {
        <Self::State as SideOps>::side_pick_knot_fence(self, id)
    }

    fn side_entity_is_taken(&self, id: u32) -> bool {
        <Self::State as SideOps>::side_entity_is_taken(self, id)
    }

    fn side_own_passenger_index(&self, mob_id: u32) -> usize {
        <Self::State as SideOps>::side_own_passenger_index(self, mob_id)
    }

    #[inline(always)]
    fn side_game_side_fields(&self) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        <Self::State as SideOps>::side_game_side_fields(self)
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> MobsRestSideGameExt<'__sg> for T {}

// Generated by Shape G codemod from side_* role dispatchers.
pub trait SideOps: Sized {
    type Game<'g>;

    fn side_indexed_mob_rider_slots<'a>(
        &'a self,
        mobs: &JoinList<Mob>,
        id: u32,
    ) -> Option<Ref<'a, [usize]>>;
    fn side_cart_rail_direction(
        game: &Self::Game<'_>,
        cart: Option<u32>,
        state: BlockStateId,
        pos: BlockPos,
        text: &str,
        vanilla: minecarts::Shape,
    ) -> minecarts::Shape;
    fn side_remote_general_type_slot(&self, id: u32) -> Option<usize>;
    fn side_move_guest_entity_id(&self, conn: u32) -> Option<u32>;
    fn side_boat_client_controlled(game: &Self::Game<'_>, id: u32) -> bool;
    fn side_remote_general_rotation(&self, id: u32) -> Option<[f32; 2]>;

    fn side_passenger_riding_position(
        game: &Self::Game<'_>,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3>;
    fn side_position_riders(game: &mut Self::Game<'_>);
    fn side_general_passenger_position(
        game: &Self::Game<'_>,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3>;
    fn side_play_player_bound_sound(
        game: &mut Self::Game<'_>,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    );
    fn side_player_equipped(game: &mut Self::Game<'_>, slot: &str, new: Option<&ItemStack>);
    fn side_boat_can_simulate_movement(game: &Self::Game<'_>, id: u32) -> bool;
    fn side_boat_has_room(game: &Self::Game<'_>, i: usize) -> bool;
    fn side_player_boat_seat(game: &Self::Game<'_>, i: usize) -> Vec3;
    fn side_tick_boat_leash(game: &mut Self::Game<'_>, b: &mut Boat);
    fn side_finish_body_restitution(
        game: &mut Self::Game<'_>,
        entity: (u32, &str),
        feet: Vec3,
        response: BodyRestitution,
    ) -> Vec3;
    fn side_body_step_on(
        game: &mut Self::Game<'_>,
        entity: (u32, &str),
        footprint: (Vec3, Aabb),
        ground: &GroundState,
        velocity: &mut Vec3,
    ) -> Vec<mod_api::EntityWrite>;
    fn side_item_fall_on(game: &mut Self::Game<'_>, id: u32, on: BlockPos, distance: f64);
    fn side_item_post_move_contacts(
        game: &mut Self::Game<'_>,
        item: &mut ItemEntity,
        client: bool,
        sources: InsideSources<'_>,
    ) -> bool;
    fn side_hurt_lent_item_entity(
        game: &mut Self::Game<'_>,
        item: &mut ItemEntity,
        amount: f32,
        damage_type: &str,
    );
    fn side_queue_boat_contact_hurts(
        game: &mut Self::Game<'_>,
        id: u32,
        hurts: Vec<(&'static str, f32)>,
    );
    fn side_apply_boat_contact_hurts(game: &mut Self::Game<'_>, id: u32) -> bool;
    fn side_boat_collision_root(game: &Self::Game<'_>, id: u32, target: Target) -> Option<u32>;
    fn side_vehicle_solids(
        game: &Self::Game<'_>,
        mover: Mover<'_>,
        solids: &Solids,
        each: &mut dyn FnMut(&Candidate, Option<usize>),
    );
    fn side_resident_dragon_part_index(game: &Self::Game<'_>, id: u32) -> Option<usize>;
    fn side_entity_in(game: &Self::Game<'_>, room: Aabb) -> bool;
    fn side_pick_knot_fence(game: &Self::Game<'_>, id: u32) -> Option<[i32; 3]>;
    fn side_entity_is_taken(game: &Self::Game<'_>, id: u32) -> bool;
    fn side_own_passenger_index(game: &Self::Game<'_>, mob_id: u32) -> usize;
    fn side_game_side_fields(
        game: &Self::Game<'_>,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool);
}
