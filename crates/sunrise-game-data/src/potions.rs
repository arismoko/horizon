//! The `MobEffects` and `Potions` registries (code in vanilla, ported as
//! tables): each effect's colour and category, each potion's effects, and the
//! potion colour mix. sunrise-client re-exports them from `brewing`.
use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
};
use sunrise_world::attributes::Operation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Beneficial,
    Harmful,
    Neutral,
}
/// One `MobEffect.AttributeTemplate`: (attribute, modifier id, amount at
/// amplifier 0, operation).
pub type EffectModifier = (&'static str, &'static str, f64, Operation);
/// A `MobEffect`: id (vanilla's without namespace, a mod's `<ns>:<path>`),
/// colour, category, instantaneous.
#[derive(Clone, Copy, Debug)]
pub struct EffectDef {
    pub id: &'static str,
    /// Position in the process-owned effect registry, resolved when an instance is created.
    pub index: usize,
    /// The content mod that declared this entry; none for vanilla effects.
    pub owner: Option<&'static str>,
    pub color: u32,
    pub category: Category,
    pub instant: bool,
    /// `IMobEffectExtension.getSortOrder`: where the effect sits in the HUD and inventory
    /// lists. The colour, unless a content mod declared an order.
    pub sort_order: i32,
    /// The attribute modifiers a content mod declared for its effect
    /// (`MobEffect.addAttributeModifier`). Vanilla's effects keep theirs in
    /// the tables of the code that applies them, so none here.
    pub modifiers: &'static [EffectModifier],
}
/// The colours resource packs give effects (Polytone's `effect.<id>`), by the table's id.
static PACK_COLORS: RwLock<Option<HashMap<String, u32>>> = RwLock::new(None);
/// Takes the packs' effect colours (none: vanilla's).
pub fn set_pack_colors(colors: Vec<(String, u32)>) {
    let colors = (!colors.is_empty()).then(|| colors.into_iter().collect());
    *PACK_COLORS.write().unwrap_or_else(|e| e.into_inner()) = colors;
}
impl EffectDef {
    /// `MobEffect.getColor` as the packs show it: potions, tipped arrows and effect particles.
    pub fn shown_color(&self) -> u32 {
        let pack = PACK_COLORS.read().unwrap_or_else(|e| e.into_inner());
        pack.as_ref()
            .and_then(|colors| colors.get(self.id))
            .map_or(self.color, |c| c & 0xFF_FFFF)
    }
}
const fn fx(index: usize, id: &'static str, color: u32, category: Category) -> EffectDef {
    EffectDef {
        index,
        id,
        owner: None,
        color,
        category,
        instant: false,
        sort_order: color as i32,
        modifiers: &[],
    }
}
use Category::{Beneficial as B, Harmful as H, Neutral as N};
/// `MobEffects` in registration order.
pub const EFFECTS: &[EffectDef] = &[
    fx(0, "speed", 3402751, B),
    fx(1, "slowness", 9154528, H),
    fx(2, "haste", 14270531, B),
    fx(3, "mining_fatigue", 4866583, H),
    fx(4, "strength", 16762624, B),
    EffectDef {
        instant: true,
        ..fx(5, "instant_health", 16262179, B)
    },
    EffectDef {
        instant: true,
        ..fx(6, "instant_damage", 11101546, H)
    },
    fx(7, "jump_boost", 16646020, B),
    fx(8, "nausea", 5578058, H),
    fx(9, "regeneration", 13458603, B),
    fx(10, "resistance", 9520880, B),
    fx(11, "fire_resistance", 16750848, B),
    fx(12, "water_breathing", 10017472, B),
    fx(13, "invisibility", 16185078, B),
    fx(14, "blindness", 2039587, H),
    fx(15, "night_vision", 12779366, B),
    fx(16, "hunger", 5797459, H),
    fx(17, "weakness", 4738376, H),
    fx(18, "poison", 8889187, H),
    fx(19, "wither", 7561558, H),
    fx(20, "health_boost", 16284963, B),
    fx(21, "absorption", 2445989, B),
    EffectDef {
        instant: true,
        ..fx(22, "saturation", 16262179, B)
    },
    fx(23, "glowing", 9740385, N),
    fx(24, "levitation", 13565951, H),
    fx(25, "luck", 5882118, B),
    fx(26, "unluck", 12624973, H),
    fx(27, "slow_falling", 15978425, B),
    fx(28, "conduit_power", 1950417, B),
    fx(29, "dolphins_grace", 8954814, B),
    fx(30, "bad_omen", 745784, N),
    fx(31, "hero_of_the_village", 4521796, B),
    fx(32, "darkness", 2696993, H),
    fx(33, "trial_omen", 1484454, N),
    fx(34, "raid_omen", 14565464, N),
    fx(35, "wind_charged", 12438015, H),
    fx(36, "weaving", 7891290, H),
    fx(37, "oozing", 10092451, H),
    fx(38, "infested", 9214860, H),
    fx(39, "breath_of_the_nautilus", 65518, B),
];
/// The effects content mods declare, registered after vanilla's.
static MOD_EFFECTS: OnceLock<Vec<EffectDef>> = OnceLock::new();
/// A content mod's effect, as its declaration gives it.
pub struct ModEffect {
    /// `<ns>:<path>`.
    pub id: String,
    pub color: u32,
    pub category: Category,
    /// The declared sort order, none for the colour.
    pub sort_order: Option<i32>,
    /// The mod that declared it.
    pub owner: String,
    /// Its attribute modifiers: (attribute, modifier id, amount at amplifier
    /// 0, operation), at most one per attribute.
    pub modifiers: Vec<(String, String, f64, Operation)>,
}
impl ModEffect {
    /// An effect with no sort order and no attribute modifiers.
    pub fn new(id: &str, color: u32, category: Category, owner: &str) -> Self {
        Self {
            id: id.to_owned(),
            color,
            category,
            sort_order: None,
            owner: owner.to_owned(),
            modifiers: Vec::new(),
        }
    }
}
/// Registers the content mods' effects in registry order.
/// The list is set once per process: false when it already was, and the
/// effects stay those of the first call until the game restarts. Inside an
/// [`isolate_mod_effects`] scope it is that scope's list instead.
pub fn register_mod_effects(effects: impl IntoIterator<Item = ModEffect>) -> bool {
    #[cfg(any(test, feature = "test-effect-registry"))]
    if let Some(registered) = SCOPED.with(|scoped| scoped.get()) {
        if registered.is_some() {
            return false;
        }
        let defs: &'static [EffectDef] = Box::leak(mod_effect_defs(effects).into_boxed_slice());
        SCOPED.with(|scoped| scoped.set(Some(Some(defs))));
        return true;
    }
    if MOD_EFFECTS.get().is_some() {
        return false;
    }
    MOD_EFFECTS.set(mod_effect_defs(effects)).is_ok()
}
#[cfg(any(test, feature = "test-effect-registry"))]
thread_local! {
    /// This thread's own effect registry while a test fixture's
    /// [`isolate_mod_effects`] scope lasts: `Some(None)` before its registration.
    static SCOPED: std::cell::Cell<Option<Option<&'static [EffectDef]>>> =
        const { std::cell::Cell::new(None) };
}
/// Test fixtures: until the guard drops, this thread registers and reads its
/// own content mod effects, empty at first, and leaves the process's alone.
#[cfg(any(test, feature = "test-effect-registry"))]
pub fn isolate_mod_effects() -> impl Drop {
    struct Restore(
        Option<Option<&'static [EffectDef]>>,
        std::marker::PhantomData<std::rc::Rc<()>>,
    );
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPED.with(|scoped| scoped.set(self.0));
        }
    }
    Restore(
        SCOPED.with(|scoped| scoped.replace(Some(None))),
        std::marker::PhantomData,
    )
}
/// Register the content effects shared by the simulation and potion-tooltip fixtures.
#[cfg(feature = "test-effect-registry")]
pub fn register_test_mod_effects() -> impl Drop {
    let scope = isolate_mod_effects();
    let modifier = |attribute: &str, id: &str, amount, operation| {
        (attribute.to_owned(), id.to_owned(), amount, operation)
    };
    assert!(register_mod_effects([
        ModEffect::new("testfx:glow", 1, Category::Harmful, "testfx"),
        ModEffect {
            modifiers: vec![
                modifier(
                    "minecraft:movement_speed",
                    "testfx:effect/stun_speed",
                    -0.25,
                    Operation::AddMultipliedTotal,
                ),
                modifier(
                    "minecraft:attack_damage",
                    "testfx:effect/stun_damage",
                    1.5,
                    Operation::AddValue,
                ),
                modifier(
                    "minecraft:armor",
                    "testfx:effect/stun_armor",
                    2.0,
                    Operation::AddValue,
                ),
                modifier(
                    "minecraft:scale",
                    "testfx:effect/stun_scale",
                    0.5,
                    Operation::AddMultipliedBase,
                ),
                modifier(
                    "minecraft:luck",
                    "testfx:effect/stun_luck",
                    1.0,
                    Operation::AddValue,
                ),
                modifier(
                    "minecraft:knockback_resistance",
                    "testfx:effect/stun_knockback",
                    0.5,
                    Operation::AddValue,
                ),
            ],
            ..ModEffect::new("testfx:stun", 2, Category::Harmful, "testfx")
        },
        ModEffect {
            modifiers: vec![modifier(
                "minecraft:max_absorption",
                "testfx:effect/shield",
                -2.0,
                Operation::AddValue,
            )],
            ..ModEffect::new("testfx:shield", 3, Category::Harmful, "testfx")
        },
        ModEffect {
            modifiers: vec![modifier(
                "minecraft:movement_speed",
                "minecraft:baby",
                0.1,
                Operation::AddMultipliedBase,
            )],
            ..ModEffect::new("testfx:grown", 4, Category::Neutral, "testfx")
        },
        ModEffect {
            modifiers: vec![modifier(
                "minecraft:movement_speed",
                "testfx:effect/still",
                0.0,
                Operation::AddValue,
            )],
            ..ModEffect::new("testfx:still", 5, Category::Neutral, "testfx")
        },
    ]));
    scope
}

/// The registry rows of the content mods' effects, after vanilla's.
fn mod_effect_defs(effects: impl IntoIterator<Item = ModEffect>) -> Vec<EffectDef> {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    effects
        .into_iter()
        .enumerate()
        .map(|(index, effect)| {
            debug_assert!(effect.id.contains(':'), "{} has no namespace", effect.id);
            let def = fx(
                EFFECTS.len() + index,
                leak(effect.id),
                effect.color,
                effect.category,
            );
            let modifiers: Vec<EffectModifier> = effect
                .modifiers
                .into_iter()
                .map(|(attribute, id, amount, operation)| {
                    (leak(attribute), leak(id), amount, operation)
                })
                .collect();
            EffectDef {
                sort_order: effect.sort_order.unwrap_or(def.sort_order),
                owner: Some(leak(effect.owner)),
                modifiers: Box::leak(modifiers.into_boxed_slice()),
                ..def
            }
        })
        .collect()
}
/// The content mods' effects, in registry order.
pub fn mod_effects() -> &'static [EffectDef] {
    #[cfg(any(test, feature = "test-effect-registry"))]
    if let Some(registered) = SCOPED.with(|scoped| scoped.get()) {
        return registered.unwrap_or(&[]);
    }
    MOD_EFFECTS.get().map_or(&[], Vec::as_slice)
}
/// Every registered effect: vanilla's, then the content mods'.
pub fn all_effects() -> impl Iterator<Item = &'static EffectDef> {
    EFFECTS.iter().chain(mod_effects())
}
pub fn effect(id: &str) -> Option<&'static EffectDef> {
    let short = id.trim_start_matches("minecraft:");
    match EFFECTS.iter().find(|e| e.id == short) {
        Some(e) => Some(e),
        None => mod_effect(id),
    }
}
/// A content mod's effect id (`<ns>:<path>`), not vanilla's with or
/// without `minecraft:`.
#[inline]
pub fn is_mod_effect(id: &str) -> bool {
    !id.starts_with("minecraft:") && id.bytes().any(|b| b == b':')
}
/// Out of line, so vanilla's lookups (every frame, every tick) stay as they were.
#[cold]
#[inline(never)]
fn mod_effect(id: &str) -> Option<&'static EffectDef> {
    mod_effects().iter().find(|e| e.id == id)
}
/// `MobEffect.getDescriptionId` (`Util.makeDescriptionId("effect", key)`):
/// `effect.<namespace>.<path>`, `/` in the path as `.`.
pub fn description_id(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("effect.{namespace}.{}", path.replace('/', "."))
}
/// The effect's sprite in the GUI atlas (`Hud.getMobEffectSprite`: the key
/// with `mob_effect/` before its path): `mob_effect/<path>` for vanilla's,
/// `<ns>:mob_effect/<path>` for a mod's.
pub fn sprite(id: &str) -> String {
    let mut out = String::new();
    push_sprite(&mut out, id);
    out
}
/// [`sprite`], appended to `out`.
pub fn push_sprite(out: &mut String, id: &str) {
    let path = match id.split_once(':') {
        None => id,
        Some(("minecraft", path)) => path,
        Some((namespace, path)) => {
            out.push_str(namespace);
            out.push(':');
            path
        }
    };
    out.push_str("mob_effect/");
    out.push_str(path);
}
/// Namespaced effect ids for mob effect storage; a mod's id already is.
pub fn namespaced(id: &'static str) -> &'static str {
    static IDS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let key = id.strip_prefix("minecraft:").unwrap_or(id);
    IDS.get_or_init(|| {
        EFFECTS
            .iter()
            .map(|e| {
                (
                    e.id,
                    &*Box::leak(format!("minecraft:{}", e.id).into_boxed_str()),
                )
            })
            .collect()
    })
    .get(key)
    .copied()
    .unwrap_or(id)
}
/// A `Potion`: id, name suffix (`item.minecraft.potion.effect.<name>`) and
/// effects as (effect, duration ticks, amplifier).
#[derive(Clone, Copy, Debug)]
pub struct PotionDef {
    pub id: &'static str,
    pub name: &'static str,
    pub effects: &'static [(&'static str, i32, i32)],
}
const fn p(
    id: &'static str,
    name: &'static str,
    effects: &'static [(&'static str, i32, i32)],
) -> PotionDef {
    PotionDef { id, name, effects }
}
/// `Potions` in registration order.
pub const POTIONS: &[PotionDef] = &[
    p("water", "water", &[]),
    p("mundane", "mundane", &[]),
    p("thick", "thick", &[]),
    p("awkward", "awkward", &[]),
    p("night_vision", "night_vision", &[("night_vision", 3600, 0)]),
    p(
        "long_night_vision",
        "night_vision",
        &[("night_vision", 9600, 0)],
    ),
    p("invisibility", "invisibility", &[("invisibility", 3600, 0)]),
    p(
        "long_invisibility",
        "invisibility",
        &[("invisibility", 9600, 0)],
    ),
    p("leaping", "leaping", &[("jump_boost", 3600, 0)]),
    p("long_leaping", "leaping", &[("jump_boost", 9600, 0)]),
    p("strong_leaping", "leaping", &[("jump_boost", 1800, 1)]),
    p(
        "fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 3600, 0)],
    ),
    p(
        "long_fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 9600, 0)],
    ),
    p("swiftness", "swiftness", &[("speed", 3600, 0)]),
    p("long_swiftness", "swiftness", &[("speed", 9600, 0)]),
    p("strong_swiftness", "swiftness", &[("speed", 1800, 1)]),
    p("slowness", "slowness", &[("slowness", 1800, 0)]),
    p("long_slowness", "slowness", &[("slowness", 4800, 0)]),
    p("strong_slowness", "slowness", &[("slowness", 400, 3)]),
    p(
        "turtle_master",
        "turtle_master",
        &[("slowness", 400, 3), ("resistance", 400, 2)],
    ),
    p(
        "long_turtle_master",
        "turtle_master",
        &[("slowness", 800, 3), ("resistance", 800, 2)],
    ),
    p(
        "strong_turtle_master",
        "turtle_master",
        &[("slowness", 400, 5), ("resistance", 400, 3)],
    ),
    p(
        "water_breathing",
        "water_breathing",
        &[("water_breathing", 3600, 0)],
    ),
    p(
        "long_water_breathing",
        "water_breathing",
        &[("water_breathing", 9600, 0)],
    ),
    p("healing", "healing", &[("instant_health", 1, 0)]),
    p("strong_healing", "healing", &[("instant_health", 1, 1)]),
    p("harming", "harming", &[("instant_damage", 1, 0)]),
    p("strong_harming", "harming", &[("instant_damage", 1, 1)]),
    p("poison", "poison", &[("poison", 900, 0)]),
    p("long_poison", "poison", &[("poison", 1800, 0)]),
    p("strong_poison", "poison", &[("poison", 432, 1)]),
    p("regeneration", "regeneration", &[("regeneration", 900, 0)]),
    p(
        "long_regeneration",
        "regeneration",
        &[("regeneration", 1800, 0)],
    ),
    p(
        "strong_regeneration",
        "regeneration",
        &[("regeneration", 450, 1)],
    ),
    p("strength", "strength", &[("strength", 3600, 0)]),
    p("long_strength", "strength", &[("strength", 9600, 0)]),
    p("strong_strength", "strength", &[("strength", 1800, 1)]),
    p("weakness", "weakness", &[("weakness", 1800, 0)]),
    p("long_weakness", "weakness", &[("weakness", 4800, 0)]),
    p("luck", "luck", &[("luck", 6000, 0)]),
    p("slow_falling", "slow_falling", &[("slow_falling", 1800, 0)]),
    p(
        "long_slow_falling",
        "slow_falling",
        &[("slow_falling", 4800, 0)],
    ),
    p("wind_charged", "wind_charged", &[("wind_charged", 3600, 0)]),
    p("weaving", "weaving", &[("weaving", 3600, 0)]),
    p("oozing", "oozing", &[("oozing", 3600, 0)]),
    p("infested", "infested", &[("infested", 3600, 0)]),
];
pub fn potion(id: &str) -> Option<&'static PotionDef> {
    let id = id.trim_start_matches("minecraft:");
    POTIONS.iter().find(|p| p.id == id)
}
/// `PotionContents.BASE_POTION_COLOR` (ARGB -13083194) as RGB.
pub const BASE_POTION_COLOR: u32 = 0x385DC6;
/// `PotionContents.getColor`: amplifier-weighted mix of visible effect colours.
pub fn potion_color(id: Option<&str>) -> u32 {
    let Some(def) = id.and_then(potion) else {
        return BASE_POTION_COLOR;
    };
    effects_color(def.effects.iter().map(|(id, _, amp)| (*id, *amp)))
}
/// `PotionContents.getColorOptional` of some effects, `(id, amplifier)`, all visible: the mix of
/// their colours weighted by amplifier + 1, else the base potion colour.
pub fn effects_color<'a>(effects: impl Iterator<Item = (&'a str, i32)>) -> u32 {
    let (mut r, mut g, mut b, mut total) = (0u32, 0u32, 0u32, 0u32);
    for (e, amp) in effects {
        let Some(e) = effect(e) else { continue };
        let w = amp as u32 + 1;
        let color = e.shown_color();
        r += w * (color >> 16 & 255);
        g += w * (color >> 8 & 255);
        b += w * (color & 255);
        total += w;
    }
    match (
        r.checked_div(total),
        g.checked_div(total),
        b.checked_div(total),
    ) {
        (Some(r), Some(g), Some(b)) => (r << 16) | (g << 8) | b,
        _ => BASE_POTION_COLOR,
    }
}
/// Borrow an effect by its completed engine registry index.
pub fn effect_by_index(index: usize) -> Option<&'static EffectDef> {
    if index < EFFECTS.len() {
        EFFECTS.get(index)
    } else {
        mod_effects().get(index - EFFECTS.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_ids_and_sprites_keep_the_namespace() {
        assert_eq!(description_id("speed"), "effect.minecraft.speed");
        assert_eq!(description_id("minecraft:speed"), "effect.minecraft.speed");
        assert_eq!(description_id("ruby:curse/dark"), "effect.ruby.curse.dark");
        assert_eq!(sprite("speed"), "mob_effect/speed");
        assert_eq!(sprite("minecraft:speed"), "mob_effect/speed");
        assert_eq!(sprite("ruby:curse/dark"), "ruby:mob_effect/curse/dark");
    }

    /// `PotionContents.getColorOptional`: each visible effect's colour weighted by its
    /// amplifier + 1, the channels averaged by integer division; none gives the base colour.
    #[test]
    fn effects_mix_their_colours_by_amplifier() {
        let c = |id: &str| effect(id).unwrap().color;
        assert_eq!(effects_color([("poison", 0)].into_iter()), c("poison"));
        let (p, s) = (c("poison"), c("speed"));
        let mix = |shift: u32| (((p >> shift & 255) + 3 * (s >> shift & 255)) / 4) << shift;
        assert_eq!(
            effects_color([("poison", 0), ("speed", 2)].into_iter()),
            mix(16) | mix(8) | mix(0)
        );
        assert_eq!(effects_color(std::iter::empty()), BASE_POTION_COLOR);
    }

    /// The registry is set once, here once per isolated scope.
    #[test]
    fn mod_effects_follow_vanilla_once_registered() {
        let _scope = isolate_mod_effects();
        let stun = (
            "minecraft:movement_speed".to_owned(),
            "ruby:effect/stun".to_owned(),
            -1.0,
            Operation::AddMultipliedTotal,
        );
        assert!(register_mod_effects([
            ModEffect::new("ruby:glow", 0xFF0000, Category::Neutral, "declaring-mod"),
            ModEffect {
                sort_order: Some(-5),
                modifiers: vec![stun],
                ..ModEffect::new("ruby:first", 0x0000FF, Category::Neutral, "declaring-mod")
            },
        ]));
        assert!(!register_mod_effects([ModEffect::new(
            "ruby:late",
            1,
            Category::Harmful,
            "other-mod"
        )]));
        let glow = effect("ruby:glow").unwrap();
        assert_eq!(glow.owner, Some("declaring-mod"));
        assert_eq!(effect("speed").unwrap().owner, None);
        assert_eq!(
            (glow.color, glow.category, glow.instant),
            (0xFF0000, Category::Neutral, false)
        );
        // The HUD and inventory sort by this: the colour unless the mod declared an order.
        assert_eq!(glow.sort_order, 0xFF0000);
        assert_eq!(effect("ruby:first").unwrap().sort_order, -5);
        assert_eq!(effect("speed").unwrap().sort_order, 3402751);
        assert!(effect("glow").is_none() && effect("ruby:late").is_none());
        // The declared modifiers ride on the registered effect.
        assert_eq!(
            effect_by_index(EFFECTS.len() + 1).unwrap().modifiers,
            [(
                "minecraft:movement_speed",
                "ruby:effect/stun",
                -1.0,
                Operation::AddMultipliedTotal
            )]
        );
        assert!(glow.modifiers.is_empty() && effect("speed").unwrap().modifiers.is_empty());
        assert!(is_mod_effect(glow.id) && !is_mod_effect("speed"));
        assert!(!is_mod_effect(namespaced("speed")));
        assert_eq!(namespaced(glow.id), "ruby:glow");
        assert_eq!(effect("minecraft:speed").unwrap().id, "speed");
        let ids: Vec<_> = all_effects().map(|e| e.id).collect();
        assert_eq!(ids.len(), EFFECTS.len() + 2);
        assert_eq!(ids.last(), Some(&"ruby:first"));
    }

    /// Each fixture thread's scope registers its own list, leaves the
    /// process's alone, and ends with the scope.
    #[test]
    fn isolated_mod_effect_registries_are_per_thread_and_scoped() {
        std::thread::scope(|threads| {
            for id in ["ruby:one", "ruby:two"] {
                threads.spawn(move || {
                    {
                        let _scope = isolate_mod_effects();
                        assert!(mod_effects().is_empty());
                        assert!(register_mod_effects([ModEffect::new(
                            id,
                            1,
                            Category::Neutral,
                            "ruby"
                        )]));
                        let ids: Vec<_> = mod_effects().iter().map(|e| e.id).collect();
                        assert_eq!(ids, [id]);
                    }
                    assert!(effect(id).is_none());
                });
            }
        });
    }
}
