//! The mode is the last decimal digit of the parent entity id. Refusals are written into
//! the first child's yaw as digits: 0 ok, 1 Expired, 2 Ownership, 3 Capacity, 4 Invalid,
//! 5 Budget, 9 anything else.

use mod_api::{
    BabyEntitySpawnEvent, Flow, Identifier, MobSplitEvent, PendingMobSpec, ServerHost,
    UnspawnedMobError,
};

fn code<T>(result: Result<T, UnspawnedMobError>) -> f32 {
    match result {
        Ok(_) => 0.0,
        Err(UnspawnedMobError::Unsupported) => 1.0,
        Err(UnspawnedMobError::Invalid) => 4.0,
        Err(_) => 9.0,
    }
}

fn spec(
    kind: &str,
    dim: &str,
    at: [f64; 3],
    yaw: f32,
    size: Option<u32>,
    baby: bool,
) -> PendingMobSpec {
    let mut spec = PendingMobSpec::new(
        Identifier::parse(kind).unwrap(),
        Identifier::parse(dim).unwrap(),
        at,
        yaw,
        baby,
    );
    spec.size = size;
    spec
}

/// `mob_split`: mode 0 changes nothing, 1 cancels, 2 edits A, removes B and appends D, 3 replaces C
/// with E and reports a second append of E in A's yaw, 4 tries invalid input, 5 reads a removed child.
pub fn mob_split(host: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
    let dim = event.dimension;
    match event.parent % 10 {
        1 => return Flow::Cancel,
        2 => {
            event.children.get_mut(0).unwrap().set_yaw(90.0).unwrap();
            let removed = event.children.remove(1).unwrap();
            assert_eq!(removed.kind().unwrap().to_string(), "minecraft:slime");
            drop(removed);
            let d = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:slime",
                        dim,
                        [3.0, 70.0, 4.0],
                        12.0,
                        Some(2),
                        false,
                    ),
                )
                .unwrap();
            event.children.push(d).unwrap();
            let _dropped = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:slime",
                        dim,
                        [9.0, 70.0, 9.0],
                        0.0,
                        Some(2),
                        false,
                    ),
                )
                .unwrap();
        }
        3 => {
            let e = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:magma_cube",
                        dim,
                        [8.0, 65.0, 9.0],
                        33.0,
                        Some(2),
                        false,
                    ),
                )
                .unwrap();
            let prev = event.children.replace(2, e).unwrap();
            assert_eq!(prev.kind().unwrap().to_string(), "minecraft:slime");
            drop(prev);
            let temp = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:slime",
                        dim,
                        [1.0, 65.0, 1.0],
                        5.0,
                        Some(2),
                        false,
                    ),
                )
                .unwrap();
            event.children.insert(0, temp).unwrap();
            let back = event.children.remove(0).unwrap();
            assert_eq!(back.position(), Ok([1.0, 65.0, 1.0]));
            drop(back);
        }
        4 => {
            let slime = |at: [f64; 3], d: &str| spec("minecraft:slime", d, at, 0.0, Some(2), false);
            let mut digits = vec![
                code(
                    event
                        .children
                        .construct(host, slime([f64::NAN, 64.0, 0.0], dim)),
                ),
                code(event.children.construct(
                    host,
                    spec(
                        "minecraft:not_a_mob",
                        dim,
                        [0.0, 64.0, 0.0],
                        0.0,
                        None,
                        false,
                    ),
                )),
                code(
                    event
                        .children
                        .construct(host, slime([0.0, 64.0, 0.0], "minecraft:the_end")),
                ),
            ];
            let oob_insert = event
                .children
                .construct(host, slime([0.0, 64.0, 0.0], dim))
                .unwrap();
            digits.push(code(event.children.insert(99, oob_insert)));
            let oob_replace = event
                .children
                .construct(host, slime([0.0, 64.0, 0.0], dim))
                .unwrap();
            digits.push(code(event.children.replace(99, oob_replace)));
            while event.children.len() < 70 {
                let more = event
                    .children
                    .construct(host, slime([0.0, 64.0, 0.0], dim))
                    .unwrap();
                event.children.push(more).unwrap();
            }
            digits.push(code(event.children.get_mut(0).unwrap().set_position([
                f64::NAN,
                0.0,
                0.0,
            ])));
            let packed = digits.iter().rev().fold(0.0, |acc, d| acc * 10.0 + d);
            event.children.get_mut(0).unwrap().set_yaw(packed).unwrap();
        }
        5 => event.children.clear(),
        _ => {}
    }
    Flow::Continue
}

/// `baby_entity_spawn`: mode 0 changes nothing, 1 cancels, 2 replaces the baby with a pig baby whose
/// yaw reports a second baby (`Capacity` is 3), 3 removes the baby, 4 offers a grown replacement
/// and reports its refusal (`Invalid` is 4) in the kept baby's yaw.
pub fn baby_entity_spawn(host: &mut dyn ServerHost, event: &mut BabyEntitySpawnEvent<'_>) -> Flow {
    let dim = event.dimension;
    let pig = |baby| spec("minecraft:pig", dim, [0.5, 64.0, 0.0], 0.0, None, baby);
    match event.parent % 10 {
        1 => return Flow::Cancel,
        2 => {
            let mut replacement = event.child.construct(host, pig(true)).unwrap();
            replacement.set_yaw(77.0).unwrap();
            replacement.add_tag("replaced_pig").unwrap();
            let prev = event.child.replace(replacement).unwrap();
            assert_eq!(prev.unwrap().kind().unwrap().to_string(), "minecraft:cow");
            let _dropped = event.child.construct(host, pig(true)).unwrap();
        }
        3 => event.child.clear(),
        4 => {
            let mut grown = event.child.construct(host, pig(false)).unwrap();
            grown.add_tag("grown_pig").unwrap();
            event.child.replace(grown).unwrap();
        }
        _ => {}
    }
    Flow::Continue
}
