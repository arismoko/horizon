//! Section entry order (`EntitySection` after `Callback.onMove`) and the
//! admission of an entity given its id after it spawned. Crossings are told
//! as they happen, by the movement boundary; these tests tell them directly.
use super::*;
use crate::join_seq::residency::Kind;

const HOME: Owner = Owner {
    dimension: DimensionKey::OVERWORLD,
    kind: Kind::Mob,
};

fn emit(journal: &Journal, change: Change) {
    journal.push(Event {
        owner: HOME,
        change,
    });
}

/// Admit `id` now, as a loaded entity joins the lookup.
fn admit(journal: &Journal, id: u32) -> Identity {
    let identity = Identity::new(id, JoinSeq::next()).unwrap();
    emit(journal, Change::Install(identity));
    identity
}

/// The ids of one section, in the order that section keeps them.
fn section(table: &mut EntityTable, ids: &[u32]) -> Vec<u32> {
    let entry = table.section_entry_order();
    let mut ids: Vec<_> = ids.iter().map(|&id| (entry(id).unwrap(), id)).collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

#[test]
fn a_move_right_after_joining_enters_the_end_of_the_section() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    // A joins section X, B joins section Y, then A moves into Y before
    // anything else happens: vanilla's Y holds B, then A.
    admit(&journal, 1);
    admit(&journal, 2);
    table.entered_section(1);
    assert_eq!(section(&mut table, &[1, 2]), vec![2, 1]);
    assert_eq!(table.level_entities(HOME.dimension), vec![1, 2]);
}

#[test]
fn a_move_out_and_back_is_two_crossings() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    // 1 and 2 are in X. Within one tick 1 steps into Y and back into X:
    // vanilla removes it from X and adds it again, so X holds 2, then 1.
    admit(&journal, 1);
    admit(&journal, 2);
    table.entered_section(1);
    table.entered_section(1);
    assert_eq!(section(&mut table, &[1, 2]), vec![2, 1]);
}

#[test]
fn crossings_enter_in_the_order_they_happen_not_in_lookup_order() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    // 5 joined the lookup before 9, and 7 is already in Y. 9 moves into Y,
    // then 5 rides in after it (a passenger moves right after its vehicle).
    admit(&journal, 5);
    admit(&journal, 7);
    admit(&journal, 9);
    table.entered_section(9);
    table.entered_section(5);
    assert_eq!(section(&mut table, &[5, 7, 9]), vec![7, 9, 5]);
    assert_eq!(table.level_entities(HOME.dimension), vec![5, 7, 9]);
}

#[test]
fn a_put_back_keeps_its_place_and_a_readmission_enters_at_the_end() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    let first = admit(&journal, 9);
    admit(&journal, 5);
    admit(&journal, 7);
    table.entered_section(5);
    assert_eq!(section(&mut table, &[5, 7, 9]), vec![9, 7, 5]);
    // A temporary extraction is not a move.
    emit(&journal, Change::Extract(first));
    emit(&journal, Change::PutBack(first));
    assert_eq!(section(&mut table, &[5, 7, 9]), vec![9, 7, 5]);
    // Removal and re-admission is a new entity at the end (`addEntity`).
    emit(&journal, Change::Remove(first));
    emit(&journal, Change::Install(first));
    assert_eq!(section(&mut table, &[5, 7, 9]), vec![7, 5, 9]);
    assert_eq!(table.level_entities(HOME.dimension), vec![5, 7, 9]);
    // A crossing told for an id that is no member changes nothing.
    table.entered_section(0);
    table.entered_section(42);
    assert_eq!(section(&mut table, &[5, 7, 9]), vec![7, 5, 9]);
}

#[test]
fn an_id_given_after_spawning_admits_the_entity_where_it_spawned() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    // An arrow joins its storage with no id yet (`addFreshEntity`), then a
    // chunk loads a zombie, then the arrow gets its id.
    let arrow = Identity::new(30, JoinSeq::next()).unwrap();
    admit(&journal, 20);
    emit(&journal, Change::Spawned(arrow));
    assert_eq!(table.level_entities(HOME.dimension), vec![30, 20]);
    assert_eq!(section(&mut table, &[20, 30]), vec![30, 20]);
    // Only a new member: an id reported again keeps the admission it has.
    emit(&journal, Change::Remove(arrow));
    admit(&journal, 21);
    emit(&journal, Change::Install(arrow));
    emit(&journal, Change::Spawned(arrow));
    assert_eq!(table.level_entities(HOME.dimension), vec![20, 21, 30]);
}
