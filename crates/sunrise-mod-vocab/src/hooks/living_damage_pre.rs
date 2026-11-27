//! The `living_damage_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingDamageEvent.Pre`: `LivingEntity.actuallyHurt` of entity `entity` of
    /// type `kind` (`minecraft:player`, `minecraft:zombie`) by `source` (named as
    /// `native_living_incoming_damage` names it) has taken armor and magic, and the absorption
    /// hearts are next: `container` is a live mutable borrow of the hit's damage container;
    /// it holds the damage it started with, what it is now (`new_damage`, which may be 0) and
    /// each reduction so far (armor, enchantments, mob effects). Change its
    /// `new_damage` to change what is inflicted before absorption, not what `lastHurt` remembers
    /// for the cooldown. Set its post-attack invulnerability ticks to change the cooldown itself,
    /// or use `add_reduction_modifier` for subsequent stages (such as `Reduction::Absorption`).
    /// The setters refuse negative damage, NaN damage and negative ticks without changing the hit.
    /// Pre fires for players, mobs and the ender dragon; a wolf whose armor takes the hit and
    /// armor stands do not fire it. The host is the hit's (`HitHost`): its blocks, random and own
    /// attachments, level sounds, and the own random of the entities the hit lends; the hurt entity
    /// is in the middle of its hit.
    #[notes = r#"## Combat hook ownership

The six combat hooks (`living_incoming_damage`, `damage_pre`, `damage_post`, `living_knockback`, `living_use_totem`, `living_shield_block`) are lent to each hit by the game that runs it, not installed once per process. A mod sees no difference: order, arguments and cancel rules are unchanged.

**Migration (`living_use_totem`, `living_shield_block`, now version 2):** both now take the hit's level host as their first argument, as `living_incoming_damage` does: its blocks, random and own attachments, with no writes beyond them. Add the parameter (`level: &mut dyn LevelRandomHost`); the event, its fields, its order and its cancel rules are unchanged. `living_shield_block` fires inside `hurtServer` before the armor and enchantment reductions and sees the hit's `damage` as it came in; `living_use_totem` fires after health reached 0 and before `die`. Tick-time effect: one extra pointer in the hit and the same `listened` early-outs. Diverges from upstream: the two hooks gain the level host that upstream's catalog rows did not have.

**Attacker and held stacks (`living_knockback`, the weapon of `living_incoming_damage`):** the attacker's id and the stacks it holds come from the hit that started the attack, not from process state, so a hit nested inside another (a thorns reply, a guest's attack during the host's) names its own attacker and nothing is left behind afterwards. No event field, order, random draw or cancel rule changes, so neither hook's version does. The game copies the held stacks only while a mod listens; without one the hit carries the attacker id and two empty slots, no allocation.

**Sounds and an entity's random (`living_incoming_damage`, `living_damage_pre`, `living_damage_post`, `living_shield_block`, `living_use_totem`, now one version up):** these five take the hit's host as `&mut dyn HitHost` (a `LevelRandomHost` that also plays level sounds and draws from an entity's own random), so a listener can play a hit sound with a pitch drawn from the attacker. `host.play_sound_at(sound, at, source, volume, pitch)` is `Level.playSound(null, x, y, z, ...)` and `host.play_sound_from(entity, sound, source, volume, pitch)` is `Level.playSound(null, entity, ...)`: every player in range hears it, the entity's own player too, and a cancelled hit still plays what was queued. The sound's seed is drawn at the call from the level's sound-seed stream, never from the level random. `entity.with_random(host, |rng| rng.next_float())` lends the entity's actual `RandomSource`, in listener order, and the stream keeps the draws. A hit lends the entity its source names as its causing entity where the game has it at hand (see the causers paragraph below for the sites); the attacking mob's own random is the one its AI draws from. Any other entity, a stale handle, and a client answer `None` (`false` for a sound). **Migration:** change the host parameter's type from `&mut dyn LevelRandomHost` to `&mut dyn HitHost`; `with_random`, attachments and the event are unchanged. `entity_invulnerability_check` is asked on both sides and as a query, so its host stays read-only and it gets none of this. Tick-time effect: one pointer in the hit; with no listener nothing is looked up, drawn or allocated.

**The victim and the causing entity (`living_incoming_damage` version 5, `living_damage_pre` and `living_damage_post` version 4):** `host.victim()` and `host.causing()` lend a read-only `LivingView` of the hit's living victim and of the causing entity the hit already lends, when it is the one the source names (`LivingIncomingDamageEvent.getEntity`, `DamageSource.getEntity`). Through the same host, the attachments of those two holders now read where they answered `MissingHolder` before (`has`, `get_existing`), and their writes answer `AttachmentError::ReadOnlyHolder` instead, writing nothing. **Migration:** a listener that relied on `MissingHolder` for the victim's or causer's holder handles the values or `ReadOnlyHolder`; nothing else changes. `living_shield_block` and `living_use_totem` lend no view and keep their version. Tick-time effect: a few references copied into the hit; with no listener nothing is looked up or built.

**Causers at every damage site (`living_incoming_damage` version 6, `living_damage_pre` and `living_damage_post` version 5, `living_shield_block` and `living_use_totem` version 4):** the hit lends its causing entity wherever the game has it at hand, not only at a melee hit: Thorns' wearer, the shooter or owner of a projectile, a firework rocket, evoker fangs, a wither skull, a thrown trident or a wind charge, an explosion's igniter or owner, and a pufferfish stinging a mob inside its own tick. A guest that is not in place is lent from its hub record, read-only, with its own attachment map. When the victim caused its own hit (a player's own TNT hurting that player), the source names the victim and `causing()` answers `None`: the listener reads it through `victim()`. So `host.causing()`, `entity.with_random(host, ...)` and `host.play_sound_from(entity, ...)` answer at those sites, and the causer's attachment holder is read-only there. **Migration:** a listener that relied on `None` from those calls at such a site handles the lent entity; nothing else changes. Tick-time effect: with no listener nothing is looked up; with one, a hit looks its causer up once."#]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "hit",
        neoforge = "LivingDamageEvent.Pre",
        version = 5,
        target = "sunrise_gameplay_common::game::native_hooks::living_damage_pre::HitLevel<'_>::native_living_damage_pre"
    )]
    fn living_damage_pre(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: mod_api::DamageContainer<'_>,
    );
}
