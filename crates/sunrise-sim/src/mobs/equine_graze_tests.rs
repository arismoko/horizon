//! `AbstractHorse.aiStep` clears a horse's `EatingHaystack` after 51 ticks.
//! A llama keeps the loaded flag because `Llama.canEatGrass` is false.
use crate::entity_nbt::{mob_from_nbt, mob_to_nbt};
use crate::mobs::world::with_test_world;
use std::sync::Arc;
use sunrise_core::nbt::Tag;

#[test]
#[ignore = "daily"]
fn a_grazing_horse_clears_eating_haystack_after_51_ticks_while_a_llama_keeps_it() {
    assert!(
        with_test_world(&[], |w, types| {
            for (kind, after_51) in [("minecraft:horse", 0), ("minecraft:llama", 1)] {
                let identity = [
                    ("id", Tag::String(kind.into())),
                    (
                        "Pos",
                        Tag::List(
                            [0.5_f64, 64.0, 0.5]
                                .map(|v| Tag::Double(v.to_bits()))
                                .into(),
                        ),
                    ),
                ];
                let raw = Tag::compound(
                    identity
                        .clone()
                        .into_iter()
                        .chain([("EatingHaystack", Tag::Byte(1))]),
                );
                // No old field to copy through: saving must write the live grazing state.
                let retained = Tag::compound(identity);
                let scoped_types = [(kind.into(), Arc::new(types.types[kind].clone()))].into();
                let mut m = mob_from_nbt(&raw, &scoped_types, 1, Some(w.registry)).unwrap();
                m.ground.on_ground = true;
                let mut test_level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                for tick in 1..=51 {
                    crate::mobs::ai::tick(&mut m, w, &mut test_level.level(), &mut events);
                    let expected = match tick {
                        50 => 1,
                        51 => after_51,
                        _ => continue,
                    };
                    let saved = mob_to_nbt(
                        &m,
                        [0; 4],
                        Some(&retained),
                        Some(w.registry),
                        |_| None,
                        w.items.unwrap(),
                    );
                    assert_eq!(
                        saved.get("EatingHaystack"),
                        Some(&Tag::Byte(expected)),
                        "{kind} at tick {tick}"
                    );
                }
            }
        }),
        "vanilla data is missing"
    );
}
