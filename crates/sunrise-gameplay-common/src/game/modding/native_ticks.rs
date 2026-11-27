//! Entities ticking for native mods (`native_entity_tick_pre`, `native_entity_tick_post`:
//! NeoForge's `EntityTickEvent.Pre` and `.Post`), on the server and on a client. Every observed
//! entity is bracketed individually around its own tick (`Pre(A)`, tick `A`, `Post(A)`, `Pre(B)`,
//! ...), matching NeoForge's `ServerLevel.tickNonPassenger` timing, with zero preparation when
//! neither hook is subscribed.
//!
//! Each entity's position writes are filed for `entity_entering_section`
//! (`entity_sections.rs`) while its tick is still running.
use std::sync::Arc;
use sunrise_sim::mobs::MobType;

/// The item entities' type, as the hooks name it.
pub const ITEM: &str = "minecraft:item";
/// The players' type, as the hooks name it.
pub const PLAYER: &str = "minecraft:player";

/// The type of an entity in a round, as the hooks name it.
pub enum TickKind {
    Fixed(&'static str),
    Mob(Arc<MobType>),
}

impl TickKind {
    pub fn id(&self) -> &str {
        match self {
            Self::Fixed(id) => id,
            Self::Mob(kind) => &kind.id,
        }
    }
}

/// The index in a family's list of the entity `is` picks out, looked for from `from` on first:
/// the list keeps its order, though a mod's hook may have taken entities out of it.
pub fn place<T>(list: &[T], from: usize, is: impl Fn(&T) -> bool) -> Option<usize> {
    let from = from.min(list.len());
    list[from..]
        .iter()
        .position(&is)
        .map(|i| from + i)
        .or_else(|| list[..from].iter().position(is))
}

/// Where slot `slot`'s entity of a family's loop is in its list now: the slot itself until a hook
/// told during the loop changed the list (`places`, by id), then by its id (`slots`), if it is
/// still there.
pub fn slot_place(
    places: &Option<sunrise_level::fast_hash::FxHashMap<u32, usize>>,
    slots: &[u32],
    slot: usize,
) -> Option<usize> {
    match places {
        None => Some(slot),
        Some(places) => places.get(&slots[slot]).copied(),
    }
}

/// Where each entity of `ids` is, by id: a family's loop finds its rows with it once a hook told
/// during the loop changed the list.
pub fn places_of(
    ids: impl Iterator<Item = u32>,
) -> sunrise_level::fast_hash::FxHashMap<u32, usize> {
    ids.enumerate().map(|(i, id)| (id, i)).collect()
}

/// Reusable kind-name scratch buffer kept between ticks so `native_other_tick_alone` allocates
/// nothing when telling an entity whose row owns its type string.
#[derive(Default)]
pub struct TickBuffers {
    pub name: String,
}

/// Whether the tick hooks are heard on this side in this tick, checked once per tick
/// (`native_tick_told`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Told {
    /// Whether an enabled mod subscribes to `entity_tick_pre`.
    pub pre: bool,
    /// Whether an enabled mod subscribes to `entity_tick_post`.
    pub post: bool,
}

impl Told {
    /// Reads the current enabled hook subscriptions for `entity_tick_pre` and `entity_tick_post`.
    pub fn from_subscriptions() -> Self {
        let pre = mod_api::__private::enabled_listened(mod_api::HookId::EntityTickPre);
        let post = mod_api::__private::enabled_listened(mod_api::HookId::EntityTickPost);
        Self { pre, post }
    }
    /// Whether an enabled mod subscribes to `entity_tick_pre`.
    pub fn pre(&self) -> bool {
        self.pre
    }
    /// Whether an enabled mod subscribes to `entity_tick_post`.
    pub fn post(&self) -> bool {
        self.post
    }
    /// Whether either tick hook is listened to on this side.
    pub fn listening(&self) -> bool {
        self.pre || self.post
    }
}

/// Per-tick entity-hook state for one family's loop.
#[derive(Default)]
pub struct TickRound {
    /// Whether `entity_tick_pre` / `entity_tick_post` are subscribed.
    pub told: Told,
    /// Whether `entity_entering_section` is listened to on this server: each entity's position
    /// write is filed and told while its tick is still running.
    pub sections: bool,
    /// The entities a Pre canceled.
    pub canceled: Vec<u32>,
    /// The items whose part ended their tick (`on_entity_item_update`): they are not
    /// canceled, so they still hear their Post.
    pub halted: Vec<u32>,
}

impl TickRound {
    /// A round that holds no entities, for one told alone.
    pub fn with_told(told: Told) -> Self {
        Self {
            told,
            ..Self::default()
        }
    }
    /// Whether a mod canceled the tick of entity `id`.
    pub fn canceled(&self, id: u32) -> bool {
        !self.canceled.is_empty() && self.canceled.contains(&id)
    }
    /// Whether a part ended the tick of item `id` (`on_entity_item_update`).
    pub fn halted(&self, id: u32) -> bool {
        !self.halted.is_empty() && self.halted.contains(&id)
    }
    /// A part ended the tick of item `id`.
    pub fn halt(&mut self, id: u32) {
        self.halted.push(id);
    }
    /// Whether `entity_entering_section` is listened to: each entity's position updates are filed.
    pub fn sections(&self) -> bool {
        self.sections
    }
    /// Whether `entity_tick_pre` is subscribed on this side.
    pub fn pre(&self) -> bool {
        self.told.pre()
    }
    /// Whether `entity_tick_post` is subscribed on this side.
    pub fn post(&self) -> bool {
        self.told.post()
    }
    /// Whether either entity tick hook (`entity_tick_pre` or `entity_tick_post`) is subscribed.
    pub fn told_listening(&self) -> bool {
        self.told.listening()
    }
    /// Whether a mod hears entity ticks or section crossings in this round.
    pub fn listening(&self) -> bool {
        self.told.listening() || self.sections
    }
}

pub trait NativeTicksGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// A round for a family about to tick, on the server or on a client, with zero preparation
    /// when neither `entity_tick_pre`, `entity_tick_post` nor `entity_entering_section` is
    /// subscribed.
    fn native_tick_round(&mut self) -> TickRound {
        let told = self.native_tick_told();
        let sections = self.side_tick_uses_server_sections();
        TickRound {
            told,
            sections,
            canceled: Vec::new(),
            halted: Vec::new(),
        }
    }

    /// Whether the tick hooks are heard on this side.
    fn native_tick_told(&self) -> Told {
        Told::from_subscriptions()
    }

    /// `native_entity_tick_pre` for one entity about to tick: returns whether its tick runs.
    fn native_tick_pre_one(&mut self, round: &mut TickRound, id: u32, kind: &str) -> bool {
        if !round.told.pre {
            return true;
        }
        let mut canceled = false;
        self.native_entity_tick_pre(self.dim.id(), id, kind, &mut canceled);
        if canceled {
            round.canceled.push(id);
        }
        !canceled
    }

    /// `native_entity_tick_post` for one entity, right after its tick, its move filed and told.
    fn native_tick_post_one(&mut self, round: &TickRound, id: u32, kind: &str) {
        if round.told.post {
            self.native_entity_tick_post(self.dim.id(), id, kind);
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeTicksGameExt1<'__sg> for G {}

use crate::game::native_hooks::base::GameHooks as _;

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{EntityTickPostEvent, EntityTickPreEvent, HostSide, ModEntry, Mods};

    /// Canceled and halted entity ids are tracked independently and an unsubscribed round
    /// performs no listening.
    #[test]
    fn a_round_tracks_canceled_and_halted_entities() {
        let mut r = TickRound {
            told: Told {
                pre: true,
                post: true,
            },
            ..TickRound::default()
        };
        r.canceled.push(3);
        r.halt(5);
        assert!(r.canceled(3) && !r.canceled(1));
        assert!(r.halted(5) && !r.halted(3));
        assert!(r.told_listening() && r.listening());

        let quiet = TickRound::default();
        assert!(!quiet.told_listening() && !quiet.listening() && !quiet.canceled(1));
    }

    /// Subscriptions to `entity_tick_pre` and `entity_tick_post` drive `Told` directly without
    /// a separate opt-in poll hook, and disabling the subscribing mod drops preparation back to
    /// zero.
    #[test]
    #[ignore = "daily"]
    fn told_follows_enabled_pre_and_post_subscriptions() {
        if crate::game::test_fixtures::run_fresh_cases(
            "game::native_ticks::tests::told_follows_enabled_pre_and_post_subscriptions",
            "SUNRISE_ENTITY_TICK_SUBSCRIPTION_CASE",
            &["disable"],
        ) {
            return;
        }

        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}

        struct PreOnly;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PreOnly {
            fn entity_tick_pre(
                &self,
                _: HostSide<'_>,
                _: &mut EntityTickPreEvent<'_>,
            ) -> mod_api::Flow {
                mod_api::Flow::Continue
            }
        }

        struct PostOnly;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PostOnly {
            fn entity_tick_post(&self, _: HostSide<'_>, _: &mut EntityTickPostEvent<'_>) {}
        }

        let quiet: Mods = vec![ModEntry::new("quiet", Box::new(Quiet))].into();
        mod_platform::testing::with_mods(quiet, || {
            let told = Told::from_subscriptions();
            assert!(!told.pre && !told.post && !told.listening());
        });

        let pre: Mods = vec![ModEntry::new("pre_only", Box::new(PreOnly))].into();
        mod_platform::testing::with_mods(pre, || {
            let told = Told::from_subscriptions();
            assert!(told.pre && !told.post && told.listening());
            assert!(mod_api::__private::set_mod_enabled("pre_only", false));
            let disabled = Told::from_subscriptions();
            assert!(!disabled.pre && !disabled.post && !disabled.listening());
        });

        let post: Mods = vec![ModEntry::new("post_only", Box::new(PostOnly))].into();
        mod_platform::testing::with_mods(post, || {
            let told = Told::from_subscriptions();
            assert!(!told.pre && told.post && told.listening());
        });
    }

    /// An entity is found where it is now, from a hint on, and before the hint when an
    /// earlier one left the list.
    #[test]
    fn place_finds_an_entity_after_the_list_changed() {
        let list = [10, 20, 30, 40];
        assert_eq!(place(&list, 1, |&id| id == 30), Some(2));
        assert_eq!(place(&list, 3, |&id| id == 20), Some(1));
        assert_eq!(place(&list, 9, |&id| id == 40), Some(3));
        assert_eq!(place(&list, 0, |&id| id == 50), None);
    }
}
