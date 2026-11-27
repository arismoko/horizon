//! Fixed-depth recursion guard for block-state survival Part dispatch and public reads.
use std::cell::RefCell;
use sunrise_core::position::BlockPos;
use sunrise_level::world::World;
use sunrise_world::registry::BlockStateId;

const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    world: usize,
    pos: [i32; 3],
    state: u32,
}

#[derive(Clone, Copy)]
enum EntryKind {
    Query { seen_hooks: u8 },
    Part,
}

#[derive(Clone, Copy)]
struct Entry {
    key: Key,
    kind: EntryKind,
}

struct Stack {
    entries: [Option<Entry>; MAX_DEPTH],
    depth: usize,
}

thread_local! {
    static ACTIVE: RefCell<Stack> = const { RefCell::new(Stack {
        entries: [None; MAX_DEPTH],
        depth: 0,
    }) };
}

#[derive(Clone, Copy)]
pub enum PartHook {
    CanSurvive = 1,
    CanSustainPlant = 2,
    MayPlaceOn = 4,
}

/// A live guard frame. Calls nest in order, so dropping always releases the top slot.
pub struct Guard {
    index: usize,
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with_borrow_mut(|stack| {
            debug_assert_eq!(stack.depth, self.index + 1);
            stack.entries[self.index] = None;
            stack.depth = self.index;
        });
    }
}

fn key(world: *const World, pos: BlockPos, state: BlockStateId) -> Key {
    Key {
        world: world as usize,
        pos: [pos.x, pos.y, pos.z],
        state: state.0,
    }
}

/// Refuse the same query while it is active, and reserve one frame for its Part callback at
/// the fixed nesting bound. The API maps refusal to `LevelReadError::Unavailable`.
pub fn enter_query(world: *const World, pos: BlockPos, state: BlockStateId) -> Option<Guard> {
    let key = key(world, pos, state);
    ACTIVE.with_borrow_mut(|stack| {
        if stack.depth >= MAX_DEPTH - 1
            || stack.entries[..stack.depth]
                .iter()
                .flatten()
                .any(|entry| entry.key == key)
        {
            return None;
        }
        let index = stack.depth;
        stack.entries[index] = Some(Entry {
            key,
            kind: EntryKind::Query { seen_hooks: 0 },
        });
        stack.depth += 1;
        Some(Guard { index })
    })
}

/// Guard one Part callback. A query frame permits each expected survival hook once, while a
/// callback frame prevents any nested survival callback for the same candidate.
pub fn enter_part(
    world: *const World,
    pos: BlockPos,
    state: BlockStateId,
    hook: PartHook,
) -> Option<Guard> {
    let key = key(world, pos, state);
    let bit = hook as u8;
    ACTIVE.with_borrow_mut(|stack| {
        if stack.depth == MAX_DEPTH
            || stack.entries[..stack.depth]
                .iter()
                .flatten()
                .any(|entry| entry.key == key && matches!(entry.kind, EntryKind::Part))
        {
            return None;
        }
        let query_index = stack.entries[..stack.depth].iter().position(|entry| {
            entry.is_some_and(|entry| {
                entry.key == key && matches!(entry.kind, EntryKind::Query { .. })
            })
        });
        if let Some(index) = query_index {
            let Some(Entry {
                kind: EntryKind::Query { seen_hooks },
                ..
            }) = stack.entries[index]
            else {
                unreachable!("query index points to a query frame")
            };
            if seen_hooks & bit != 0 {
                return None;
            }
            stack.entries[index] = Some(Entry {
                key,
                kind: EntryKind::Query {
                    seen_hooks: seen_hooks | bit,
                },
            });
        }
        let index = stack.depth;
        stack.entries[index] = Some(Entry {
            key,
            kind: EntryKind::Part,
        });
        stack.depth += 1;
        Some(Guard { index })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_queries_callbacks_and_excessive_nesting_are_refused() {
        let world = std::ptr::NonNull::<World>::dangling().as_ptr();
        let state = BlockStateId(1);
        let pos = BlockPos::new(4, 64, 8);
        let query = enter_query(world, pos, state).unwrap();
        assert!(enter_query(world, pos, state).is_none());

        let part = enter_part(world, pos, state, PartHook::CanSurvive).unwrap();
        assert!(enter_part(world, pos, state, PartHook::CanSurvive).is_none());
        assert!(enter_part(world, pos, state, PartHook::CanSustainPlant).is_none());
        drop(part);
        // One survival evaluation may proceed from the state hook to the soil and plant hooks.
        assert!(enter_part(world, pos, state, PartHook::CanSustainPlant).is_some());
        drop(query);

        let mut nested = Vec::with_capacity(MAX_DEPTH - 1);
        for depth in 0..MAX_DEPTH - 1 {
            nested.push(
                enter_query(world, BlockPos::new(depth as i32, 64, 8), state)
                    .expect("the configured nesting depth fits the guard"),
            );
        }
        // A query may use the penultimate frame, but another query cannot consume the frame
        // reserved for its Part callback.
        assert!(enter_query(world, BlockPos::new(-1, 64, 8), state).is_none());
        let last_pos = BlockPos::new((MAX_DEPTH - 2) as i32, 64, 8);
        let callback = enter_part(world, last_pos, state, PartHook::CanSurvive)
            .expect("a survival callback can use the final guard frame");
        assert_eq!(ACTIVE.with_borrow(|stack| stack.depth), MAX_DEPTH);
        drop(callback);
        while nested.pop().is_some() {}
    }
}
