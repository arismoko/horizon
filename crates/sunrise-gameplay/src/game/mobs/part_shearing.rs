//! Shears and entity parts (NeoForge's `IShearable`, #3351): a player's shears and a dispenser's
//! ask the mob's entity parts first (`EntityPart::is_shearable`), and a part that says yes
//! shears it (`EntityPart::on_sheared`) in place of the species' own shearing.
use super::Hand;
use mod_api::PlayerRef;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::{
    api_values::{TryIntoEngine, project_stack, stack_read_metadata},
    item_snapshot::ItemSnapshotGameExt1 as _,
    native_hooks::{is_shearable::GameHooks as _, on_sheared::GameHooks as _},
};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Species, Who};

impl crate::game::ServerGame<'_> {
    /// `IShearable.isShearable` as live mob `index`'s entity parts answer it: the answering part's
    /// registration slot and its answer, the first part to answer winning; none when no part
    /// answers. A type with no subscribed part costs one table lookup. Server-side.
    pub(in crate::game) fn part_shearable(
        &mut self,
        index: usize,
        player: Option<PlayerRef>,
        shears: &ItemStack,
    ) -> Option<((usize, usize), bool)> {
        if !self.common.mobs[index].alive() {
            return None;
        }
        let kind = std::sync::Arc::clone(&self.common.mobs[index].kind);
        let kind = &kind.id;
        if !mod_api::__private::has_scoped_hook(mod_api::HookId::IsShearable)
            || !mod_api::__private::entity_part_hooked(
                mod_api::HookId::IsShearable,
                "is_shearable",
                kind,
            )
        {
            return None;
        }
        let entity = self.entity(self.common.mobs[index].id)?;
        let metadata = stack_read_metadata(shears).ok()?;
        let project = |metadata| project_stack(shears, metadata).ok();
        let shears_view = mod_api::ItemStackView::resident(metadata, &project);
        let mut shearable = None;
        let slot = self.native_is_shearable(entity, player, shears_view, &mut shearable, kind)?;
        shearable.map(|answer| (slot, answer))
    }

    /// `IShearable.onSheared` on mob `index`'s entity part `part` (as [`Self::part_shearable`]
    /// named it), then `spawnShearedDrop` for each drop it returns: one block above where the mob
    /// stands afterwards, nudged by the mob's own random as `Sheep.shear` nudges its wool.
    /// A panicking callback is quarantined and does not count as a successful shearing.
    pub(in crate::game) fn part_shear(
        &mut self,
        index: usize,
        part: (usize, usize),
        player: Option<PlayerRef>,
        shears: &ItemStack,
    ) -> Result<(), ()> {
        let (id, at, kind) = (
            self.common.mobs[index].id,
            self.common.mobs[index].position,
            std::sync::Arc::clone(&self.common.mobs[index].kind),
        );
        let (Some(entity), Ok(metadata)) = (self.entity(id), stack_read_metadata(shears)) else {
            return Ok(());
        };
        let project = |metadata| project_stack(shears, metadata).ok();
        let shears_view = mod_api::ItemStackView::resident(metadata, &project);
        let boundary_items = self.stack_boundary_items();
        let mut drops = Vec::new();
        self.native_on_sheared(entity, player, shears_view, &mut drops, part, &kind.id)
            .map_err(|_| ())?;
        // Invalid drops are skipped individually; the part's valid ones still fall.
        let accepted: Vec<ItemStack> = boundary_items.map_or_else(Vec::new, |items| {
            drops
                .iter()
                .filter_map(|drop| drop.try_into_engine(&*items).ok())
                .collect()
        });
        // The part may have moved or removed the mob: its drops fall where it is now.
        let index = self
            .common
            .mobs
            .iter()
            .position(|m| m.id == id && !m.removed);
        let at = index.map_or(at, |i| self.common.mobs[i].position);
        for stack in accepted {
            // `IShearable.spawnShearedDrop`: each species' own height; a mooshroom's drop has
            // no pickup delay and the rest fall with `Sheep.shear`'s push, drawn from the mob's
            // random only for an item that actually joined the level.
            let species = index.map(|i| {
                (
                    self.common.mobs[i].species(),
                    self.common.mobs[i].eye_height(),
                    f64::from(self.common.mobs[i].height()),
                )
            });
            match species {
                Some((Species::SnowGolem, eye, _)) => {
                    self.spawn_at_location(at, eye, stack);
                }
                Some((Species::Bogged, _, height)) => {
                    self.spawn_at_location(at, height, stack);
                }
                Some((Species::Mooshroom, _, height)) => {
                    self.spawn_item_own(stack, [at.x, at.y + height, at.z], 0);
                }
                _ => {
                    self.spawn_at_location(at, 1.0, stack);
                    let Some(i) = index else { continue };
                    if self.just_spawned_item().is_none() {
                        continue;
                    }
                    let r = &mut self.common.mobs[i].random;
                    let push = [
                        f64::from((r.next_float() - r.next_float()) * 0.1),
                        f64::from(r.next_float() * 0.05),
                        f64::from((r.next_float() - r.next_float()) * 0.1),
                    ];
                    if let Some(item) = self.just_spawned_item() {
                        let v = item.velocity;
                        item.velocity = Vec3::new(v.x + push[0], v.y + push[1], v.z + push[2]);
                    }
                }
            }
        }
        Ok(())
    }

    /// Shears in the acting player's hand on mob `index` (`ShearsItem.interactLivingEntity`):
    /// the entity parts' answer, else the species' own (`Sheep`, `MushroomCow`, `SnowGolem`,
    /// `Bogged`), and a shearable mob fires `player_shear_entity`. `Some(consumed)` ends the
    /// interaction: a part's refusal (false), the hook's cancel or a part's shearing (true);
    /// none leaves the shearing to the species' own interaction.
    pub fn player_shear(&mut self, mut index: usize) -> Option<bool> {
        let shears = self.common.held().cloned()?;
        let player = self.native_acting_player();
        let part = self.part_shearable(index, player, &shears);
        if part.is_some_and(|(_, shearable)| !shearable) {
            return Some(false);
        }
        let mob = &self.common.mobs[index];
        let (id, at) = (mob.id, mob.position);
        let shearable = part.is_some()
            || match mob.species() {
                Species::Sheep => !mob.extra.sheared && !mob.is_baby(),
                Species::Mooshroom => !mob.is_baby(),
                Species::SnowGolem | Species::Bogged => mob.alive() && !mob.extra.sheared,
                _ => false,
            };
        if shearable
            && let (Some(uuid), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            let mut cancel = false;
            let off_hand = self.common.hand == Hand::Off;
            self.native_player_shear_entity(uuid, &name, id as i32, off_hand, &mut cancel);
            if cancel {
                return Some(true);
            }
            // The listener had the game in its hands: find the mob again by
            // its id, and it must still be there, not removed, with the same
            // shears in hand. Otherwise the click is spent and nothing is
            // sheared.
            match self.common.mobs.iter().position(|m| m.id == id) {
                Some(now)
                    if !self.common.mobs[now].removed
                        && Self::same_item_and_count(self.common.held(), Some(&shears)) =>
                {
                    index = now;
                }
                _ => return Some(true),
            }
        }
        let (part, _) = part?;
        if self.part_shear(index, part, player, &shears).is_err() {
            return Some(false);
        }
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        self.swing_arm();
        Some(true)
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

#[cfg(test)]
use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_sim::mobs::Mob;

    /// A cow that shears into one leather.
    struct LeatherCow;
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for LeatherCow {
        fn is_shearable(
            &self,
            _: mod_api::HostSide,
            event: &mut mod_api::IsShearableEvent<'_>,
        ) -> Option<bool> {
            assert!(event.player.is_none(), "a dispenser has no player");
            assert_eq!(event.shears.count(), 1);
            assert_eq!(event.shears.damage(), 7);
            Some(true)
        }
        fn on_sheared(&self, _: mod_api::HostSide, event: &mut mod_api::OnShearedEvent<'_>) {
            assert_eq!(event.shears.damage(), 7);
            let leather = mod_api::__private::item_key("minecraft:leather").unwrap();
            event
                .drops
                .push(mod_api::ItemStack::new(leather, 1).unwrap());
        }
    }
    /// A sheep that refuses the shears.
    struct WoollySheep;
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for WoollySheep {
        fn is_shearable(
            &self,
            _: mod_api::HostSide,
            _: &mut mod_api::IsShearableEvent<'_>,
        ) -> Option<bool> {
            Some(false)
        }
    }
    struct ShearMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ShearMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:cow", LeatherCow);
            r.entity_part("minecraft:sheep", WoollySheep);
        }
    }

    /// A dispenser's shears ask the mob's entity parts: a cow whose part says yes drops what
    /// the part returns, and a woolly sheep whose part says no keeps its wool.
    #[test]
    #[ignore = "daily"]
    fn entity_parts_decide_and_perform_a_dispensers_shearing() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::server_side::part_shearing::tests::entity_parts_decide_and_perform_a_dispensers_shearing",
            "SUNRISE_PART_SHEARING_CHILD",
            &["valid"],
        ) {
            return;
        }
        let entries = vec![mod_api::ModEntry::new("shearing", Box::new(ShearMod))];
        mod_platform::__private::install(entries.into());
        sunrise_gameplay_common::game::api_values::install_parts(&[mod_api::ModEntry::new(
            "shearing",
            Box::new(ShearMod),
        )])
        .unwrap();
        with_game(|game| {
            game.resolve_parts().unwrap();
            // The mobs below take ids 1 and 2 by hand: reserve them in the game's allocator, or the
            // first drop is given id 1 too.
            game.server.next_entity = game.server.next_entity.max(2);
            crate::game::native_entity_refs::tests::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );
            let mut add = |id: u32, kind: &str, x: i32| {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let at = Vec3::new(f64::from(x) + 0.5, 64.0, 0.5);
                game.mobs.push(Mob::new(id, kind, at, 0.0));
            };
            add(1, "minecraft:cow", 0);
            add(2, "minecraft:sheep", 4);

            let mut shears = ItemStack::new("minecraft:shears", 1);
            shears.damage = 7;
            assert!(
                game.shear_entity((0, 64, 0), &shears).unwrap(),
                "the cow is sheared"
            );
            let leather: Vec<_> = game
                .dropped
                .iter()
                .filter(|d| &*d.stack.item == "minecraft:leather")
                .map(|d| d.stack.count)
                .collect();
            assert_eq!(leather, [1]);

            assert!(
                !game.shear_entity((4, 64, 0), &shears).unwrap(),
                "the sheep refuses"
            );
            assert!(!game.mobs.iter().any(|m| m.id == 2 && m.extra.sheared));
            assert!(!game.dropped.iter().any(|d| d.stack.item.ends_with("_wool")));
        });
    }
}
