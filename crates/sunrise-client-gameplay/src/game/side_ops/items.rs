#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_redirectable_projectile_tag(&self, kind: &str) -> Option<bool> {
        let remote = self.remote.as_ref()?;
        let tags = remote.registries.tags.get("minecraft:entity_type")?;
        let ids = remote.builtin.get("minecraft:entity_type")?;
        Some(
            tags.get("minecraft:redirectable_projectile")
                .is_some_and(|members| ids.id(kind).is_some_and(|id| members.contains(&id))),
        )
    }
    fn side_remote_owns_bobber_target(&self, id: u32) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.entity_id as u32 == id)
    }
    fn side_command_hook_rotation(game: &Self::Game<'_>, id: u32) -> Option<[f32; 2]> {
        client_fishing_hook_by_id(game, id).map(|hook| [hook.yaw, hook.pitch])
    }
    fn side_fishing_hook_on_ground(game: &Self::Game<'_>, id: u32) -> Option<bool> {
        client_fishing_hook_by_id(game, id).map(|hook| hook.ground.on_ground)
    }

    fn side_emit_consume(
        game: &mut Self::Game<'_>,
        consumable: &ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        {
            game.client_emit_consume(consumable, stack, count);
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
            game.client_move_bobber_response(id, position, ground, wanted, moved, velocity);
        }
    }
    fn side_consume_held_slot(game: &mut Self::Game<'_>, amount: u32) {
        {
            game.client_consume_held_slot(amount);
        }
    }
    fn side_on_start_using_item(game: &mut Self::Game<'_>, stack: &ItemStack) {
        {
            game.client_on_start_using_item(stack);
        }
    }
    fn side_apply_use_cooldown(game: &mut Self::Game<'_>, stack: &ItemStack) {
        {
            game.client_apply_use_cooldown(stack);
        }
    }
    fn side_item_pick(game: &Self::Game<'_>) -> Option<Hit> {
        game.client_item_pick()
    }
    fn side_bucket_liquid_user_entity_id(game: &Self::Game<'_>) -> Option<u32> {
        game.client_bucket_liquid_user_entity_id()
    }
    fn side_in_rain(game: &mut Self::Game<'_>, at: Vec3, top: f64) -> bool {
        game.client_in_rain(at, top)
    }
    fn side_projectile_player_uuid(game: &mut Self::Game<'_>) -> Option<u128> {
        game.client_projectile_player_uuid()
    }
    fn side_acting_player_ref(game: &Self::Game<'_>) -> Option<mod_api::PlayerRef> {
        game.client_acting_player_ref()
    }
    fn side_destroy_progress_player_ref(game: &Self::Game<'_>) -> Option<mod_api::PlayerRef> {
        game.client_acting_player_ref()
    }
    fn side_current_level_ref(game: &Self::Game<'_>) -> Option<mod_api::LevelRef> {
        game.client_current_level_ref()
    }
}

#[cfg(feature = "client")]
fn client_fishing_hook_by_id<'a>(
    game: &'a crate::game::ClientGame<'_>,
    id: u32,
) -> Option<&'a sunrise_gameplay_common::game::fishing_common::Hook> {
    use sunrise_sim::join_seq::{
        Joined as _,
        residency::{
            Kind, Source,
            table::{EntityLookup, SourceEntity},
        },
    };
    let found = game
        .resident_entities
        .table
        .borrow_mut()
        .server_entity_lookup(id, |at| {
            if at.owner.kind != Kind::Hook || !matches!(at.source, Source::Hook { caster: None }) {
                return None;
            }
            let hook = game.hook.as_ref()?;
            if hook.caster_generation != game.resident_entities.local_player.join()
                || hook.join_seq() != at.join
                || hook.caster.is_some()
            {
                return None;
            }
            Some(SourceEntity {
                id: hook.id(),
                value: hook,
            })
        });
    match found {
        EntityLookup::Found { value, .. } => Some(value),
        EntityLookup::Taken | EntityLookup::Missing => None,
    }
}
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Hit;
use sunrise_gameplay_common::game::item_use_common::ItemConsumable;
use sunrise_gameplay_common::game::side_ops::items::SideOps;
use sunrise_items::items::ItemStack;
