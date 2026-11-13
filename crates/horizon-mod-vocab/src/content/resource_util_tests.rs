use super::*;
use crate::testing::FakeHost;
use std::{cell::RefCell, rc::Rc};

/// A mod's own resource, which the game knows nothing of.
#[derive(Clone, Debug, PartialEq)]
struct Chemical(&'static str);

/// Tanks of up to 100 of a chemical each, shared with the test to read back.
#[derive(Clone)]
struct Tanks(Rc<RefCell<Vec<Option<(Chemical, u32)>>>>);

impl Tanks {
    fn new(held: Vec<Option<(Chemical, u32)>>) -> Self {
        Self(Rc::new(RefCell::new(held)))
    }
    fn held(&self) -> Vec<Option<(Chemical, u32)>> {
        self.0.borrow().clone()
    }
}

impl ResourceHandler<Chemical> for Tanks {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.0.borrow().len()
    }
    fn resource(&self, _: &mut dyn ServerHost, index: usize) -> Option<Chemical> {
        self.0.borrow().get(index)?.as_ref().map(|(c, _)| c.clone())
    }
    fn amount(&self, _: &mut dyn ServerHost, index: usize) -> u32 {
        self.0
            .borrow()
            .get(index)
            .cloned()
            .flatten()
            .map_or(0, |(_, a)| a)
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &Chemical) -> u32 {
        100
    }
    fn insert(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        chemical: &Chemical,
        amount: u32,
    ) -> u32 {
        let mut tanks = self.0.borrow_mut();
        let Some(tank) = tanks.get_mut(index) else {
            return 0;
        };
        let held = match tank {
            Some((other, _)) if other != chemical => return 0,
            Some((_, held)) => *held,
            None => 0,
        };
        let went = amount.min(100 - held);
        if went > 0 {
            *tank = Some((chemical.clone(), held + went));
        }
        went
    }
    fn extract(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        chemical: &Chemical,
        amount: u32,
    ) -> u32 {
        let mut tanks = self.0.borrow_mut();
        let Some(tank) = tanks.get_mut(index) else {
            return 0;
        };
        let Some((held_chemical, held)) = tank.clone().filter(|(c, _)| c == chemical) else {
            return 0;
        };
        let came = amount.min(held);
        *tank = (held > came).then(|| (held_chemical, held - came));
        came
    }
}

const HYDROGEN: Chemical = Chemical("test:hydrogen");

#[test]
fn a_combined_handler_numbers_both_and_fills_the_second_when_the_first_is_full() {
    let full = Some((HYDROGEN, 100));
    let first = Tanks::new(vec![full.clone(), full.clone()]);
    let second = Tanks::new(vec![None, None]);
    let combined = CombinedResourceHandler::new(vec![
        Box::new(first.clone()) as Box<dyn ResourceHandler<Chemical>>,
        Box::new(second.clone()),
    ]);
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    assert_eq!(combined.size(host), 4);
    assert_eq!(combined.amount(host, 1), 100);
    assert_eq!(combined.resource(host, 2), None);
    let went =
        host.transaction(|host, tx| Ok::<_, ()>(combined.insert_any(host, tx, &HYDROGEN, 150)));
    assert_eq!(went, Ok(150));
    assert_eq!(first.held(), vec![full.clone(), full.clone()]);
    assert_eq!(second.held(), vec![full, Some((HYDROGEN, 50))]);
}

#[test]
fn insert_stacking_tops_up_a_holding_index_before_an_empty_one() {
    let tanks = Tanks::new(vec![None, Some((HYDROGEN, 10))]);
    let handler: &dyn ResourceHandler<Chemical> = &tanks;
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let went =
        host.transaction(|host, tx| Ok::<_, ()>(handler.insert_stacking(host, tx, &HYDROGEN, 95)));
    assert_eq!(went, Ok(95));
    // The default `insert_any` would have filled index 0 first.
    assert_eq!(
        tanks.held(),
        vec![Some((HYDROGEN, 5)), Some((HYDROGEN, 100))]
    );
}

#[test]
fn the_comparator_reads_0_when_empty_15_when_full_and_vanillas_steps_between() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let signal = |host: &mut dyn ServerHost, held| {
        (&Tanks::new(held) as &dyn ResourceHandler<Chemical>).comparator_signal(host)
    };
    assert_eq!(signal(&mut *host, vec![None, None]), 0);
    assert_eq!(
        signal(
            &mut *host,
            vec![Some((HYDROGEN, 100)), Some((HYDROGEN, 100))]
        ),
        15
    );
    // One unit in one of two tanks: any content gives at least 1.
    assert_eq!(signal(&mut *host, vec![Some((HYDROGEN, 1)), None]), 1);
    // Half full: floor(0.5 * 14) + 1.
    assert_eq!(signal(&mut *host, vec![Some((HYDROGEN, 100)), None]), 8);
}

/// A handler of `size` indices each holding `u32::MAX` of hydrogen in a capacity of 1: amounts
/// and sizes past anything sane, which no query may panic on.
struct Oversized(usize);

impl ResourceHandler<Chemical> for Oversized {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.0
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
        Some(HYDROGEN)
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        u32::MAX
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &Chemical) -> u32 {
        1
    }
    fn insert(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &Chemical,
        _: u32,
    ) -> u32 {
        0
    }
    fn extract(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &Chemical,
        _: u32,
    ) -> u32 {
        0
    }
}

#[test]
fn oversized_amounts_and_sizes_cap_instead_of_overflowing() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let full = &Oversized(1) as &dyn ResourceHandler<Chemical>;
    assert_eq!(full.comparator_signal(host), 15);
    let combined = CombinedResourceHandler::new(vec![
        Box::new(Oversized(usize::MAX)) as Box<dyn ResourceHandler<Chemical>>,
        Box::new(Oversized(2)),
    ]);
    assert_eq!(combined.size(host), usize::MAX);
}

/// One index that holds nothing yet answers one more than it was asked: a mod bug.
struct Liar;

impl<R> ResourceHandler<R> for Liar {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<R> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &R) -> u32 {
        u32::MAX
    }
    fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, amount: u32) -> u32 {
        amount.saturating_add(1)
    }
    fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, _: u32) -> u32 {
        0
    }
}

/// `handler` as a capability `owner`'s provider answered with.
fn capability<R: 'static>(
    handler: impl ResourceHandler<R> + 'static,
    owner: &'static str,
) -> Capability<dyn ResourceHandler<R>> {
    Capability {
        handler: Box::new(handler),
        owner: Some(owner),
    }
}

#[test]
fn a_part_answering_more_than_asked_shows_through_every_total() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let tank_then_liar = || {
        CombinedResourceHandler::new(vec![
            Box::new(Tanks::new(vec![None])) as Box<dyn ResourceHandler<Chemical>>,
            Box::new(Liar),
        ])
    };
    let insert = |host: &mut dyn ServerHost, handler: &dyn ResourceHandler<Chemical>, amount| {
        host.transaction(|host, tx| Ok::<_, ()>(handler.insert_any(host, tx, &HYDROGEN, amount)))
    };
    // The tank takes 100; the liar, asked for 50, answers 51: the total is over 150.
    assert_eq!(insert(&mut *host, &tank_then_liar(), 150), Ok(151));
    // Asked `u32::MAX`, no total can be over it: it answers 0, never `u32::MAX`.
    assert_eq!(insert(&mut *host, &tank_then_liar(), u32::MAX), Ok(0));
    // The default `insert_any` and `insert_stacking` add up their indices the same way.
    let indices = CombinedResourceHandler::new(vec![
        Box::new(Tanks::new(vec![Some((HYDROGEN, 90))])) as Box<dyn ResourceHandler<Chemical>>,
        Box::new(Liar),
    ]);
    let indices = &indices as &dyn ResourceHandler<Chemical>;
    let stacking =
        host.transaction(|host, tx| Ok::<_, ()>(indices.insert_stacking(host, tx, &HYDROGEN, 20)));
    assert_eq!(stacking, Ok(21));

    // A move out of an endless source asks `u32::MAX`: the saturated total used to read as all
    // of it going in, and the lying move committed.
    let from = capability(InfiniteResourceHandler::new(HYDROGEN), "test:creative");
    let to = capability(tank_then_liar(), "test:liar");
    let moved = host.transaction(|host, tx| {
        crate::move_resources(host, &from, &to, |_: &Chemical| true, u32::MAX, tx)
    });
    assert_eq!(moved, Ok(0));
}

/// A host with `ids` registered, stacking to 64, and their resources.
fn items_host(names: &[&str]) -> (FakeHost, Vec<ItemResource>) {
    use crate::prelude::ItemsHost;
    let mut host = FakeHost::new();
    let ids: Vec<_> = names
        .iter()
        .map(|id| crate::Identifier::parse(id).unwrap())
        .collect();
    host.registries.resolve_items(ids.clone()).unwrap();
    for name in names {
        host.registries
            .item_component(name, "minecraft:max_stack_size", Tag::Int(64));
    }
    let resources = ids
        .iter()
        .map(|id| ItemResource::of(&crate::ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
        .collect();
    (host, resources)
}

/// A chest at `pos` holding `slots`, in the journaled level, so an abort puts it back.
fn chest(host: &mut FakeHost, pos: [i32; 3], slots: &[Option<(&ItemResource, u32)>]) {
    let entity = host.level.block_entity(pos).with_items(slots.len());
    for (slot, held) in slots.iter().enumerate() {
        entity.items.as_mut().unwrap().slots[slot] = held.map(|(r, n)| r.to_stack(n).unwrap());
    }
}

fn items_at(host: &mut FakeHost, pos: [i32; 3]) -> Capability<dyn ResourceHandler<ItemResource>> {
    let level = host.level.level();
    let server: &mut dyn ServerHost = host;
    level.capability(server, &crate::ITEMS, pos, None).unwrap()
}

/// How many each slot of the chest at `pos` holds.
fn counts(host: &FakeHost, pos: [i32; 3]) -> Vec<u32> {
    let entity = host.level.block_entity_at(pos).unwrap();
    let slots = &entity.items.as_ref().unwrap().slots;
    slots
        .iter()
        .map(|s| s.as_ref().map_or(0, |s| s.count()))
        .collect()
}

const A: [i32; 3] = [0, 64, 0];
const B: [i32; 3] = [1, 64, 0];

#[test]
fn move_first_moves_one_resource_and_an_abort_puts_it_back() {
    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(&mut fake, A, &[Some((dirt, 3)), Some((stone, 5))]);
    chest(&mut fake, B, &[None, Some((stone, 10))]);
    let (from, to) = (items_at(&mut fake, A), items_at(&mut fake, B));
    let only_stone = |r: &ItemResource| r == stone;

    // Aborted: both chests as they were.
    let host: &mut dyn ServerHost = &mut fake;
    let aborted: Result<(), _> =
        host.transaction(|host, tx| Err(move_first_stacking(host, &from, &to, only_stone, 64, tx)));
    assert_eq!(aborted, Err(Ok(Some((stone.clone(), 5)))));
    assert_eq!(counts(&fake, A), [3, 5]);
    assert_eq!(counts(&fake, B), [0, 10]);

    // Committed, stacking: onto the 10 stone, not into the empty slot.
    let host: &mut dyn ServerHost = &mut fake;
    let moved =
        host.transaction(|host, tx| move_first_stacking(host, &from, &to, only_stone, 64, tx));
    assert_eq!(moved, Ok(Some((stone.clone(), 5))));
    assert_eq!(counts(&fake, A), [3, 0]);
    assert_eq!(counts(&fake, B), [0, 15]);

    // `move_first` moves the first resource that goes, the dirt, and only it.
    let host: &mut dyn ServerHost = &mut fake;
    let moved = host.transaction(|host, tx| move_first(host, &from, &to, |_| true, 2, tx));
    assert_eq!(moved, Ok(Some((dirt.clone(), 2))));
    assert_eq!(counts(&fake, A), [1, 0]);
    assert_eq!(counts(&fake, B), [2, 15]);
}

#[test]
fn find_extractable_probes_without_taking_anything() {
    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(&mut fake, A, &[Some((dirt, 3)), Some((stone, 5))]);
    let items = items_at(&mut fake, A);
    let host: &mut dyn ServerHost = &mut fake;
    let found = host.transaction(|host, tx| {
        Ok::<_, ()>(items.find_extractable(host, tx, |r: &ItemResource| r == stone))
    });
    assert_eq!(found, Ok(Some(stone.clone())));
    assert_eq!(counts(&fake, A), [3, 5]);
}

#[test]
fn an_overanswer_after_an_honest_part_undoes_the_whole_move() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    let stone = &r[0];
    chest(&mut fake, A, &[Some((stone, 5))]);
    chest(&mut fake, B, &[Some((stone, 62))]);
    let from = items_at(&mut fake, A);
    let honest = items_at(&mut fake, B).handler;
    let to = capability(
        CombinedResourceHandler::new(vec![honest, Box::new(Liar)]),
        "test:liar",
    );
    let host: &mut dyn ServerHost = &mut fake;
    // B takes 2; the liar, asked for 3, answers 4.
    let moved =
        host.transaction(|host, tx| crate::move_resources(host, &from, &to, |_| true, 5, tx));
    assert_eq!(
        moved,
        Err(Overanswered {
            owner: "test:liar".into()
        })
    );
    assert_eq!(counts(&fake, A), [5]);
    assert_eq!(counts(&fake, B), [62]);
}
