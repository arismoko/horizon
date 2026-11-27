//! The `NativeClientHooks` the library's tests install. The callbacks are process-wide and
//! install once, so every test that needs them calls [`install`]. Each answers only under a
//! setting its own tests write: `rules_mod` for the menu, entity and block hooks, and
//! `item_parts` for the item hooks ([`answering`]), so every other test sees vanilla's answers.
use super::NativeClientHooks;
use crate::items::ItemStack;
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_world::native_load_context::{
    DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide,
};

/// What the installed `collision_shape` answers a query on this thread.
pub(crate) type CollisionAnswer = Box<dyn Fn(&mod_api::CollisionQuery) -> Option<Vec<[f64; 6]>>>;

thread_local! {
    /// The server-side jumps the installed `living_jump` heard on this thread. Per thread,
    /// because every test whose mob or player jumps on a server loop context reaches it once
    /// the callbacks are installed; a test that counts sets it to zero first.
    pub(crate) static JUMPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The asks the installed `menu_still_valid` heard on this thread, per thread for the same
    /// reason.
    pub(crate) static STILL_VALID_ASKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The position the installed `menu_still_valid` last heard on this thread.
    pub(crate) static STILL_VALID_POS: std::cell::Cell<Option<[i32; 3]>> =
        const { std::cell::Cell::new(None) };
    /// How many times this thread asked the installed `collision_shape`.
    pub(crate) static COLLISION_ASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The answer the installed `collision_shape` gives this thread; none leaves the shape be.
    pub(crate) static COLLISION_ANSWER: std::cell::RefCell<Option<CollisionAnswer>> =
        const { std::cell::RefCell::new(None) };
    /// The item hooks this thread asked: the hook, the stack's item, and the answer it arrived
    /// with.
    static ITEM_ASKS: std::cell::RefCell<Vec<(&'static str, String, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The jumps the installed `living_jump` heard on this thread while a [`record_jumps`]
    /// guard lives: the entity and its velocity. `None` when no test records.
    static JUMPED: std::cell::RefCell<Option<Vec<(u32, [f64; 3])>>> =
        const { std::cell::RefCell::new(None) };
}

/// Records this thread's jumps until it is dropped; recorded jumps leave [`JUMPS`] alone.
pub(crate) struct JumpRecording;

impl Drop for JumpRecording {
    fn drop(&mut self) {
        JUMPED.set(None);
    }
}

/// Starts recording the jumps the installed `living_jump` hears on this thread.
pub(crate) fn record_jumps() -> JumpRecording {
    JUMPED.set(Some(Vec::new()));
    JumpRecording
}

/// The jumps recorded on this thread since the last call, oldest first.
pub(crate) fn jumps() -> Vec<(u32, [f64; 3])> {
    JUMPED.with_borrow_mut(|jumps| jumps.as_mut().map(std::mem::take).unwrap_or_default())
}

/// The item hooks this thread asked since the last call, oldest first: the hook, the stack's
/// item, and the answer it arrived with.
pub(crate) fn item_asks() -> Vec<(&'static str, String, bool)> {
    ITEM_ASKS.with_borrow_mut(std::mem::take)
}

/// A server loop context whose `item_parts` setting `hook` holds `answers`: comma-separated
/// `item=true` or `item=false` pairs, which the installed item hooks answer for those items.
pub(crate) fn answering(hook: &str, answers: &str) -> LoopContext {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let settings = std::env::temp_dir().join(format!(
        "sunrise-item-parts-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = settings.join("item_parts").join(hook);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, answers).unwrap();
    let inputs = std::sync::Arc::new(LoadInputs::new(Default::default(), settings));
    LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
        .loop_context(LoopSide::Server, ["item_parts"])
}

/// Record the ask, then answer what the `item_parts` setting `hook` gives `stack`'s item, if
/// anything.
fn item_part(context: &LoopContext, hook: &'static str, stack: &ItemStack, answer: &mut bool) {
    ITEM_ASKS.with_borrow_mut(|asks| asks.push((hook, stack.item.to_string(), *answer)));
    let Some(answers) = context.setting("item_parts", hook) else {
        return;
    };
    let given = answers.split(',').find_map(|pair| {
        let (item, given) = pair.split_once('=')?;
        (item == &*stack.item).then_some(given == "true")
    });
    if let Some(given) = given {
        *answer = given;
    }
}

/// A server loop context with no settings.
pub(crate) fn loop_context() -> LoopContext {
    let inputs = std::sync::Arc::new(LoadInputs::new(Default::default(), std::env::temp_dir()));
    LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
        .loop_context(LoopSide::Server, [] as [&str; 0])
}

/// Install the test callbacks, once for the process.
pub(crate) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let installed = super::install(NativeClientHooks {
            menu_still_valid: |context, player, menu, pos, eye, _, valid| {
                STILL_VALID_ASKED.set(STILL_VALID_ASKED.get() + 1);
                STILL_VALID_POS.set(pos);
                if menu == "test:rules" && player == 2 && eye[2] == 0.0 {
                    *valid = context.setting("rules_mod", "stack").is_some();
                }
            },
            // A part answers on the vanilla chest menu too: the game must
            // still never ask it, because it is not a mod menu type.
            menu_still_valid_listens: |menu| matches!(menu, "test:rules" | "minecraft:generic_9x3"),
            living_heal: |context, _, amount, cancel| {
                if context.setting("rules_mod", "stack").is_some() {
                    *cancel = true;
                    *amount += 1.0;
                }
            },
            living_breathe_heard: || false,
            has_mods: || false,
            player_name_format: |context, _, displayname| {
                if let Some(value) = context.setting("rules_mod", "stack") {
                    *displayname = crate::text::Text::literal(value);
                }
            },
            player_tab_list_name_format: |context, _, _, display_name| {
                *display_name = context
                    .setting("rules_mod", "stack")
                    .map(crate::text::Text::literal);
            },
            living_drown: |context, _, _, drowning, damage, bubbles, _| {
                if context.setting("rules_mod", "stack").is_some() {
                    *drowning = true;
                    *damage += 1.0;
                    *bubbles = 0;
                }
            },
            living_fall: |context, _, _, distance, multiplier, _| {
                if context.setting("rules_mod", "stack").is_some() {
                    *distance += 1.0;
                    *multiplier *= 2.0;
                }
            },
            living_jump: |context, entity, _, velocity| {
                let recorded = JUMPED.with_borrow_mut(|jumps| {
                    jumps
                        .as_mut()
                        .map(|jumps| jumps.push((entity, velocity)))
                        .is_some()
                });
                if !recorded && context.side() == LoopSide::Server {
                    JUMPS.set(JUMPS.get() + 1);
                }
            },
            living_jump_power: |context, _, power| {
                *power += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
            },
            living_speed: |context, _, speed| {
                *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 2.0);
            },
            living_flying_speed: |context, _, speed| {
                *speed += context.setting("rules_mod", "stack").map_or(0.0, |_| 3.0);
            },
            block_jump_factor: |context, _, factor| {
                *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 4.0);
            },
            block_speed_factor: |context, _, factor| {
                *factor += context.setting("rules_mod", "stack").map_or(0.0, |_| 5.0);
            },
            is_ladder: |context, _, _, _, ladder| {
                *ladder |= context.setting("rules_mod", "stack").is_some();
            },
            // Keeps the game's reach: the menu tests read vanilla's `stillValid` distance.
            block_interaction_range: |_, _, _, _, _| {},
            makes_open_trapdoor_above_climbable: |context, _, _, _, climbable| {
                *climbable |= context.setting("rules_mod", "stack").is_some();
            },
            friction: |level, entity, _, _, friction| {
                if level.context.setting("rules_mod", "stack").is_some() {
                    *friction = match entity.map(|(_, kind)| kind) {
                        Some("minecraft:player") => 0.98,
                        Some("minecraft:cow") => f32::NAN,
                        _ => 3.0,
                    };
                }
                true
            },
            collision_shape: |_, _, _, query, shape| {
                COLLISION_ASKS.with(|asks| asks.set(asks.get() + 1));
                COLLISION_ANSWER.with_borrow(|answer| {
                    *shape = answer
                        .as_ref()
                        .and_then(|answer| answer(query))
                        .map(|boxes| {
                            mod_api::__private::voxel_shape_into_core(
                                mod_api::VoxelShape::try_from_boxes(&boxes).unwrap(),
                            )
                        });
                });
            },
            player_stays_on_ground_surface: |context, _, staying| {
                *staying |= context.setting("rules_mod", "stack").is_some();
            },
            ender_entity_teleport: |context, _, _, _, target, cancel| {
                if context.setting("rules_mod", "stack").is_some() {
                    target[1] += 1.0;
                    *cancel = true;
                }
            },
            fluid_flow: |context, _, _, flow| {
                flow[0] += context.setting("rules_mod", "stack").map_or(0.0, |_| 1.0);
            },
            sound_type: |level, _, _, entity, cause, own| {
                (level.context.setting("rules_mod", "stack").is_some()
                    && entity.is_some_and(|(_, kind)| kind == "minecraft:player")
                    && cause == mod_api::BlockSoundCause::Step)
                    .then(|| mod_api::BlockSoundType {
                        step: mod_api::SoundEventKey::parse("rules_mod:block.rules.step").unwrap(),
                        volume: 2.0,
                        ..own.clone()
                    })
            },
            can_grindstone_repair: |context, stack, can_repair| {
                item_part(context, "can_grindstone_repair", stack, can_repair);
            },
            is_combine_repairable: |context, stack, repairable| {
                item_part(context, "is_combine_repairable", stack, repairable);
            },
            can_fit_inside_container_items: |context, stack, fits| {
                item_part(context, "can_fit_inside_container_items", stack, fits);
            },
            item_name: |_, _, _| {},
        });
        assert!(
            installed,
            "the library's callbacks are installed only here in its tests"
        );
    });
}
