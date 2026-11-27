//! The engine side of `rotate`: the callback every template placement turns its states through,
//! asking the parts of each state's block on the worldgen workers and on the server thread. It
//! reads what the server published (`native_worldgen_parts`), never a game thread's tables.
use super::api_values::block_state::Index;
pub use super::native_worldgen_parts;
use mod_api::StructureMirror;
pub use mod_api::{BlockState, HookId, StructureRotation};
pub use sunrise_world::registry::BlockStateId as EngineState;
use sunrise_world::worldgen::structure::Mirror;
pub use sunrise_world::worldgen::structure::Rotation;

pub fn mirror(
    state: EngineState,
    pos: [i32; 3],
    mirror: Mirror,
    table: EngineState,
) -> EngineState {
    let mirror = match mirror {
        Mirror::None => StructureMirror::None,
        Mirror::LeftRight => StructureMirror::LeftRight,
        Mirror::FrontBack => StructureMirror::FrontBack,
    };
    ask_parts(
        HookId::Mirror,
        "mirror",
        state,
        table,
        |host, asked, mirrored| {
            mod_api::world::mirror::native_mirror(host, asked, pos, mirror, mirrored);
        },
    )
}

pub fn rotate(
    state: EngineState,
    pos: [i32; 3],
    rotation: Rotation,
    table: EngineState,
) -> EngineState {
    let rotation = match rotation {
        Rotation::None => StructureRotation::None,
        Rotation::Clockwise90 => StructureRotation::Clockwise90,
        Rotation::Clockwise180 => StructureRotation::Clockwise180,
        Rotation::Counterclockwise90 => StructureRotation::Counterclockwise90,
    };
    ask_parts(
        HookId::Rotate,
        "rotate",
        state,
        table,
        |host, asked, rotated| {
            mod_api::world::rotation::native_rotate(host, asked, pos, rotation, rotated);
        },
    )
}

fn ask_parts(
    hook: HookId,
    name: &str,
    state: EngineState,
    table: EngineState,
    dispatch: impl FnOnce(&mod_api::OwningWorldgen<'_>, BlockState, &mut BlockState),
) -> EngineState {
    if !mod_api::__private::has_part_hook(hook) {
        return table;
    }
    let Some(index) = native_worldgen_parts::index() else {
        return table;
    };
    if !index.worldgen_listens(hook, state) {
        return table;
    }
    let (Ok(asked), Ok(mut answer)) = (index.to_api(state), index.to_api(table)) else {
        return table;
    };
    if let Err(error) = sunrise_level::crash::contain(|| {
        native_worldgen_parts::with_host(|host| dispatch(host, asked, &mut answer));
    }) {
        log::warn!("{name} callback failed: {error}; the template places the game's state");
        return table;
    }
    placed(&index, hook, name, asked, answer).unwrap_or(table)
}

fn placed(
    index: &Index,
    hook: HookId,
    name: &str,
    state: BlockState,
    answer: BlockState,
) -> Option<EngineState> {
    let refused = |why: &str| {
        log::warn!(
            "{name}: a part of {} answered {why}; the template places the game's state",
            mod_api::__private::worldgen_part_mods(hook, state).join(", ")
        );
        None
    };
    if answer.block() != state.block() {
        return refused("a state of another block");
    }
    match index.to_engine(answer) {
        Ok(placed) => Some(placed),
        Err(error) => refused(&format!(
            "a block state this game does not have ({error:?})"
        )),
    }
}
