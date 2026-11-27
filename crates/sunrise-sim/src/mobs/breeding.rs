//! Shared breeding inheritance and parent-stream reward decisions.
use super::{Mob, MobWorld, Species};

pub fn draws_parent_variant(species: Species) -> bool {
    matches!(
        species,
        Species::Pig
            | Species::Cow
            | Species::Chicken
            | Species::Fox
            | Species::Cat
            | Species::Wolf
    )
}

pub fn orb_after_child(species: Species) -> bool {
    species == Species::Fox
}

/// Complete the goal-owned parent stream before any subsequent same-tick work.
pub(super) fn voice_and_experience(
    m: &mut Mob,
    world: &MobWorld,
    inherited: bool,
) -> (
    Option<crate::lan::data::RegistryRef<super::variants::WolfSoundVariant>>,
    super::BreedExperience,
) {
    // Wolf.getBreedOffspring picks its voice after the coat and before
    // finalizeSpawnChildFromBreeding draws XP. Collar mixing uses level RNG.
    // A baby-spawn listener defers both voice and XP until approval.
    let wolf_voice = if m.species() == Species::Wolf && !world.asks_baby_spawn {
        world
            .side
            .variant_tables()
            .and_then(|v| v.wolf_sound.random(&mut m.random))
    } else {
        None
    };
    let experience = if inherited && !world.asks_baby_spawn && !super::orb_after_child(m.species())
    {
        super::BreedExperience::Drawn(super::breeding_experience(m, world.mob_drops))
    } else {
        super::BreedExperience::Deferred
    };
    (wolf_voice, experience)
}
