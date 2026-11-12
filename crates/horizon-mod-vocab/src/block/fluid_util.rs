//! Fluid interaction helpers, as NeoForge 26.3's `FluidUtil` (arismoko/horizon#3382): the common
//! case of a player right-clicking a tank with a bucket or any fluid container, in one call, with
//! the bucket's sound and game event. Pure vocabulary over [`FLUIDS`], [`ITEM_FLUIDS`] at
//! [`ItemPlace::PlayerHand`] and the move step of [`move_resources`](crate::move_resources): nothing changes until a mod calls them.
use crate::content::resource::{Ends, move_once};
use crate::{
    Capability, Direction, FLUIDS, FluidResource, GameEventContext, Hand, ITEM_FLUIDS, ItemPlace,
    LevelRef, PlayerRef, ResourceHandler, ServerHost, SoundSource,
};

/// `FluidUtil.interactWithFluidHandler` for a block: `player`'s `hand` against the tanks of the
/// block at `pos` in `level` as `side` reaches them ([`FLUIDS`]), as
/// [`interact_with_handler`] does. False where the block has no tanks.
///
/// A tank block calls it from its `use_item_on` part and answers a success when it is true:
///
/// ```no_run
/// use mod_api::{BlockPart, Direction, InteractionResult, ServerHost, UseItemOnEvent, fluid_util};
/// struct Tank;
/// #[mod_api::horizon_part]
/// impl BlockPart for Tank {
///     fn use_item_on(
///         &self,
///         host: &mut dyn ServerHost,
///         event: &mut UseItemOnEvent<'_>,
///     ) -> Option<InteractionResult> {
///         let click = event.click;
///         let player = host.player(click.player)?;
///         let level = player.level(host)?;
///         let side = Direction::ALL.into_iter().find(|d| d.normal() == click.hit.face);
///         fluid_util::interact_with_fluid_handler(host, player, click.hand, level, click.pos, side)
///             .then_some(InteractionResult::Success)
///     }
/// }
/// ```
pub fn interact_with_fluid_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> bool {
    if player.level(host) != Some(level) {
        return false;
    }
    let Some(handler) = level.capability(host, &FLUIDS, pos, side) else {
        return false;
    };
    interact_with_handler(host, player, hand, Some(pos), &handler)
}

/// `FluidUtil.interactWithFluidHandler` for any handler: fills the fluid container `player`
/// holds in `hand` ([`ITEM_FLUIDS`] at [`ItemPlace::PlayerHand`]) from `handler` and, when
/// nothing goes in, empties it into `handler`. Either way it moves the first fluid that moves,
/// as much of it as fits, in a transaction of its own: a stack of more than one container has
/// one split off, changed, where the place puts it, and a creative player keeps the hand's
/// stack, as [`ItemPlace::PlayerHand`] writes. Once something moved it plays the fill or empty
/// sound and game event at the centre of `pos`, or half a block above the player's feet
/// where `pos` is none, as
/// [`trigger_sound_and_game_event`]. True when something moved; false, with nothing changed,
/// where the hand holds no fluid container or nothing moves either way.
///
/// The sound and game event go out as it returns: call it outside a transaction the caller
/// may still abort.
pub fn interact_with_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    pos: Option<[i32; 3]>,
    handler: &Capability<dyn ResourceHandler<FluidResource>>,
) -> bool {
    let Some(level) = player.level(host) else {
        return false;
    };
    let place = ItemPlace::PlayerHand {
        player: player.uuid(),
        hand,
    };
    let Some(item) = place.capability(host, &ITEM_FLUIDS) else {
        return false;
    };
    let moved = match move_first(host, handler, &item) {
        Some(fluid) => (fluid, true),
        None => match move_first(host, &item, handler) {
            Some(fluid) => (fluid, false),
            None => return false,
        },
    };
    let (fluid, pickup) = moved;
    let at = match pos {
        Some([x, y, z]) => Some([x, y, z].map(|c| f64::from(c) + 0.5)),
        None => player
            .entity()
            .position(host)
            .map(|[x, y, z]| [x, y + 0.5, z]),
    };
    if let Some(at) = at {
        trigger_sound_and_game_event(host, &fluid, level, at, Some(player), pickup);
    }
    true
}

/// `FluidUtil.triggerSoundAndGameEvent`: what a bucket plays as `fluid` goes in or out of a
/// container at `at` in `level`. `pickup` is true when the fluid came out of the handler into the
/// container (`minecraft:fluid_pickup` and the fluid's
/// [`fluid_pickup_sound`](crate::prelude::TagsHost::fluid_pickup_sound)) and false when it went
/// in (`minecraft:fluid_place` and its
/// [`fluid_empty_sound`](crate::prelude::TagsHost::fluid_empty_sound)). The sound plays under
/// the block slider at full volume and pitch for every player near, the acting one included,
/// since the server decides the move; the game event names `player` as its source when given.
/// A fluid with no world form, a tank-only one, has no sound, and only its game event goes out.
pub fn trigger_sound_and_game_event(
    host: &mut dyn ServerHost,
    fluid: &FluidResource,
    level: LevelRef,
    at: [f64; 3],
    player: Option<PlayerRef>,
    pickup: bool,
) {
    let id = fluid
        .fluid()
        .key()
        .and_then(crate::content::registry_keys::fluid_id);
    let sound = id.and_then(|id| match pickup {
        true => host.fluid_pickup_sound(id),
        false => host.fluid_empty_sound(id),
    });
    if let Some(sound) = sound {
        level.play_sound(host, sound, at, SoundSource::Blocks, 1.0, 1.0);
    }
    let event = match pickup {
        true => "minecraft:fluid_pickup",
        false => "minecraft:fluid_place",
    };
    let context = GameEventContext {
        source_entity: player.map(PlayerRef::entity),
        affected_state: None,
    };
    level.game_event(host, event, at, context);
}

/// One move of the first fluid of `from`, index by index, that `to` takes any of, as much of it
/// as `to` takes in one go (NeoForge's `ResourceHandlerUtil.moveFirst`), all or nothing in a
/// transaction of its own: that fluid, or none, with nothing changed, where none moves or a
/// handler answers more than it was asked. One go, not [`move_resources`](crate::move_resources)' repeats: a stack of
/// buckets has one filled, as vanilla's click fills one.
fn move_first(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ResourceHandler<FluidResource>>,
    to: &Capability<dyn ResourceHandler<FluidResource>>,
) -> Option<FluidResource> {
    for index in 0..from.size(host) {
        let Some(fluid) = from.resource(host, index) else {
            continue;
        };
        let want = from.amount(host, index);
        let ends = Ends {
            from,
            to,
            at: &index,
        };
        let moved = move_once(
            host,
            ends,
            want,
            |host, tx, want| from.extract(host, tx, index, &fluid, want),
            |host, tx, out| to.insert_any(host, tx, &fluid, out),
        );
        if moved.is_ok_and(|moved| moved > 0) {
            return Some(fluid);
        }
    }
    None
}
