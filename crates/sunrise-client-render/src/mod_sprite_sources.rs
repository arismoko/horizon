//! The atlas sprite source types native mods register (NeoForge's `RegisterSpriteSourcesEvent`),
//! bound once at start so a resource load invokes each type's provider directly when listing an
//! atlas definition.
use crate::atlas_sources::{ModSources, is_builtin_type};
use mod_api::{
    __private::game::{CallbackOwner, CallbackValue, SpriteSourceRegistration},
    Answered, Identifier, ResourceReader, SpriteSourceFn, SpriteSourceInput, SpriteSourceOutput,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};
use sunrise_core::nbt::Tag;

struct BoundSpriteSource {
    list: CallbackValue<Arc<SpriteSourceFn>>,
}

type SpriteSourceTable = BTreeMap<String, Arc<BoundSpriteSource>>;

static SOURCES: RwLock<SpriteSourceTable> = RwLock::new(BTreeMap::new());

#[cfg(any(test, feature = "test-support"))]
pub fn has_registered() -> bool {
    !SOURCES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty()
}

fn bind_into(
    table: &RwLock<SpriteSourceTable>,
    sources: Vec<Answered<SpriteSourceRegistration>>,
) -> Vec<String> {
    let mut kept = Vec::new();
    let mut seen = BTreeSet::new();
    // A discarded capture can run mod code on drop; never hold the table lock then.
    let mut rejected = Vec::new();
    let mut displaced = Vec::new();
    let mut bound = table
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for Answered { answer, mod_id, .. } in sources {
        let id = answer.id;
        let Some(owner) = CallbackOwner::new(mod_id) else {
            rejected.push(CallbackValue::new(
                CallbackOwner::for_mod(mod_id),
                "sprite source",
                "drop_provider",
                answer.list,
            ));
            log::warn!("Sprite source type {id} has no installed owner {mod_id}");
            continue;
        };
        let key = id.to_string();
        let entry = Arc::new(BoundSpriteSource {
            list: CallbackValue::new(owner, "sprite source", "drop_provider", answer.list),
        });
        if let Some(earlier) = bound.insert(key.clone(), entry) {
            let earlier_mod = earlier.list.owner().id();
            displaced.push(earlier);
            log::warn!(
                "Sunrise: sprite source type {key} registered by {earlier_mod} is replaced by {mod_id}'s registration"
            );
        } else if is_builtin_type(&key) {
            log::warn!(
                "Sunrise: sprite source type {key} registered by the game is replaced by {mod_id}'s registration"
            );
        }
        if seen.insert(key.clone()) {
            kept.push(key);
        }
    }
    drop(bound);
    drop(displaced);
    drop(rejected);
    kept
}

/// Binds the sprite source providers native mods registered once at start, before resources load.
/// Any valid `Identifier` (including `minecraft:` and empty-path ids) is accepted; a later
/// registration for the same id replaces the earlier one (last-writer-wins) with a warning naming
/// both mods, and overriding a built-in vanilla source type logs a warning naming the mod.
/// Returns the unique ids kept in this batch, in first-registration order.
pub fn register(sources: Vec<Answered<SpriteSourceRegistration>>) -> Vec<String> {
    bind_into(&SOURCES, sources)
}

/// A load-local table, or `None` to read the installed providers.
struct SourceReader<'a>(Option<&'a SpriteSourceTable>);

impl SourceReader<'_> {
    fn read<T>(&self, read: impl FnOnce(&SpriteSourceTable) -> T) -> T {
        match self.0 {
            Some(table) => read(table),
            None => read(
                &SOURCES
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
        }
    }
}

impl ModSources for SourceReader<'_> {
    fn is_registered(&self, kind: &str) -> bool {
        self.read(|table| table.contains_key(kind))
    }
    fn ask(
        &self,
        atlas: &Identifier,
        kind: &str,
        config: &Tag,
        resources: &Arc<dyn ResourceReader>,
    ) -> Vec<SpriteSourceOutput> {
        // Keep the entry, not the registry's read guard, while invoking the provider.
        let Some(entry) = self.read(|table| table.get(kind).cloned()) else {
            return Vec::new();
        };

        let input = SpriteSourceInput::new(atlas, config, resources.as_ref());
        let owner = entry.list.owner();
        match owner.call("sprite source", "sprite_source", || {
            (entry.list.value())(&input)
        }) {
            Ok(Ok(outputs)) => outputs,
            Ok(Err(reason)) => {
                log::warn!(
                    "atlas {atlas}: mod {} sprite source {kind} failed: {reason}",
                    owner.id()
                );
                Vec::new()
            }
            Err(error) => {
                log::warn!("atlas {atlas}: sprite source {kind}: {error}");
                Vec::new()
            }
        }
    }
}

/// The native mods' bound sprite source providers, invoked on the resource loader as registrations.
pub struct Registered;
impl ModSources for Registered {
    fn is_registered(&self, kind: &str) -> bool {
        SourceReader(None).is_registered(kind)
    }
    fn ask(
        &self,
        atlas: &Identifier,
        kind: &str,
        config: &Tag,
        resources: &Arc<dyn ResourceReader>,
    ) -> Vec<SpriteSourceOutput> {
        SourceReader(None).ask(atlas, kind, config, resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas_sources::{
        Listing, SpriteOrigin, SpriteSource, decode, list, test_mods::Mods,
    };
    use mod_api::{
        __private::{HookPanicReport, game::RegistrationContextNew, take_hook_panics},
        RegistrationContext,
    };
    use std::sync::{
        Weak,
        atomic::{AtomicUsize, Ordering},
    };

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    fn textures() -> Vec<String> {
        [
            "srctest:item/apple",
            "srctest:item/gem",
            "srctest:item/stick",
        ]
        .map(String::from)
        .to_vec()
    }
    fn pixels(sprite: &str, width: u32, height: u32, rgba: Vec<u8>) -> SpriteSourceOutput {
        SpriteSourceOutput::Pixels {
            sprite: id(sprite),
            width,
            height,
            rgba,
        }
    }
    /// A definition of a directory source, the registered source, then a single file.
    fn definition() -> Vec<SpriteSource> {
        decode(
            r#"{"sources":[
                {"type":"directory","source":"item","prefix":"item/"},
                {"type":"srctest:own","shade":7},
                {"type":"single","resource":"srctest:item/stick","sprite":"srctest:late"}
            ]}"#,
            &Mods::new(&["srctest:own"], |_, _, _, _| Vec::new()),
        )
        .unwrap()
    }

    #[test]
    fn registration_accepts_minecraft_namespace_overrides_builtins_and_last_writer_wins() {
        struct PanicOnDrop {
            table: Weak<RwLock<SpriteSourceTable>>,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                let table = self.table.upgrade().unwrap();
                assert!(table.try_read().is_ok(), "sprite table is still locked");
                self.drops.fetch_add(1, Ordering::Relaxed);
                panic!("sprite capture destructor");
            }
        }
        struct Earlier {
            table: Weak<RwLock<SpriteSourceTable>>,
            displaced_drops: Arc<AtomicUsize>,
            rejected_drops: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Earlier {
            fn register_sprite_sources(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
            ) {
                let displaced = PanicOnDrop {
                    table: self.table.clone(),
                    drops: Arc::clone(&self.displaced_drops),
                };
                event.register(id("srcreg:shared"), move |_| {
                    let _ = &displaced;
                    Ok(vec![pixels("srcreg:from_earlier", 1, 1, vec![1; 4])])
                });
                event.register(id("minecraft:single"), |_| {
                    Ok(vec![pixels("srcreg:single_earlier", 1, 1, vec![2; 4])])
                });
                event.register(id("srcreg:unrelated"), |_| {
                    Ok(vec![pixels("srcreg:unrelated", 1, 1, vec![6; 4])])
                });
                let rejected = PanicOnDrop {
                    table: self.table.clone(),
                    drops: Arc::clone(&self.rejected_drops),
                };
                event.register(id("srcreg:unattributed"), move |_| {
                    let _ = &rejected;
                    Ok(Vec::new())
                });
            }
        }
        struct Later;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Later {
            fn register_sprite_sources(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
            ) {
                event.register(id("srcreg:shared"), |_| {
                    Ok(vec![pixels("srcreg:from_later", 1, 1, vec![3; 4])])
                });
                event.register(id("minecraft:single"), |_| {
                    Ok(vec![pixels("srcreg:single_override", 1, 1, vec![4; 4])])
                });
                event.register(id("minecraft:custom"), |_| {
                    Ok(vec![pixels("srcreg:mc_custom", 1, 1, vec![5; 4])])
                });
            }
        }

        let table = Arc::new(RwLock::new(SpriteSourceTable::new()));
        let displaced_drops = Arc::new(AtomicUsize::new(0));
        let rejected_drops = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new(
                "earlier_src",
                Box::new(Earlier {
                    table: Arc::downgrade(&table),
                    displaced_drops: Arc::clone(&displaced_drops),
                    rejected_drops: Arc::clone(&rejected_drops),
                }),
            ),
            mod_api::ModEntry::new("later_src", Box::new(Later)),
        ]
        .into();
        let (kept, notices) = mod_platform::testing::with_mods(mods, || {
            let mut registrations = Vec::new();
            mod_api::dispatch_register_sprite_sources(
                &RegistrationContext::new(),
                &mut registrations,
            );
            registrations
                .iter_mut()
                .find(|entry| entry.answer.id == id("srcreg:unattributed"))
                .unwrap()
                .mod_id = "missing_src_owner";
            let kept = bind_into(&table, registrations);
            (kept, take_hook_panics())
        });
        assert_eq!(displaced_drops.load(Ordering::Relaxed), 1);
        assert_eq!(rejected_drops.load(Ordering::Relaxed), 1);
        assert_eq!(notices.len(), 2);
        for mod_id in ["earlier_src", "missing_src_owner"] {
            assert!(notices.contains(&HookPanicReport {
                mod_id,
                hook: "drop_provider",
            }));
        }
        let Ok(table) = Arc::try_unwrap(table) else {
            panic!("unexpected sprite table owner");
        };
        let table = table.into_inner().unwrap();
        assert_eq!(
            kept,
            [
                "srcreg:shared",
                "minecraft:single",
                "srcreg:unrelated",
                "minecraft:custom"
            ]
        );
        let registered = SourceReader(Some(&table));
        assert!(
            registered.is_registered("srcreg:shared")
                && registered.is_registered("minecraft:single")
                && registered.is_registered("minecraft:custom")
                && !registered.is_registered("srcreg:unattributed")
        );
        let source = crate::resource_source::ResourceSource::indexed(
            [
                (
                    "assets/minecraft/atlases/blocks.json".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(
                        br#"{"sources":[
                            {"type":"srcreg:shared"},
                            {"type":"single","resource":"srctest:item/apple"},
                            {"type":"minecraft:custom"},
                            {"type":"srcreg:unrelated"}
                        ]}"#
                        .to_vec(),
                    ),
                ),
                (
                    "assets/srctest/textures/item/apple.png".to_owned(),
                    sunrise_assets::resource_source::Resource::bytes(b"png".to_vec()),
                ),
            ]
            .into(),
        );
        let listing = Listing::build(&source, &SourceReader(Some(&table)));
        assert_eq!(
            listing
                .sprites("minecraft:blocks")
                .keys()
                .collect::<Vec<_>>(),
            [
                "srcreg:from_later",
                "srcreg:mc_custom",
                "srcreg:single_override",
                "srcreg:unrelated"
            ],
            "later registration replaces earlier one and overrides built-in minecraft:single"
        );
    }

    #[test]
    fn panic_disables_only_that_provider_with_notice_and_non_panic_err_does_not_latch() {
        let panic_calls = Arc::new(AtomicUsize::new(0));
        let flaky_calls = Arc::new(AtomicUsize::new(0));
        let healthy_calls = Arc::new(AtomicUsize::new(0));

        struct Panicker(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Panicker {
            fn register_sprite_sources(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
            ) {
                let calls = Arc::clone(&self.0);
                event.register(id("srctest:panics"), move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("sprite source boom");
                });
            }
        }
        struct Flaky(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Flaky {
            fn register_sprite_sources(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
            ) {
                let calls = Arc::clone(&self.0);
                event.register(id("srctest:flaky"), move |_| {
                    if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                        Err("missing palette".into())
                    } else {
                        Ok(vec![pixels("srctest:recovered", 1, 1, vec![7; 4])])
                    }
                });
            }
        }
        struct Healthy(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Healthy {
            fn register_sprite_sources(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
            ) {
                let calls = Arc::clone(&self.0);
                event.register(id("srctest:healthy"), move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(vec![pixels("srctest:ok", 1, 1, vec![8; 4])])
                });
            }
        }

        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new("src_panicker", Box::new(Panicker(Arc::clone(&panic_calls)))),
            mod_api::ModEntry::new("src_flaky", Box::new(Flaky(Arc::clone(&flaky_calls)))),
            mod_api::ModEntry::new("src_healthy", Box::new(Healthy(Arc::clone(&healthy_calls)))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut registrations = Vec::new();
            mod_api::dispatch_register_sprite_sources(
                &RegistrationContext::new(),
                &mut registrations,
            );
            let table = RwLock::new(SpriteSourceTable::new());
            let kept = bind_into(&table, registrations);
            let table = table.into_inner().unwrap();
            assert_eq!(kept, ["srctest:panics", "srctest:flaky", "srctest:healthy"]);

            let registered = SourceReader(Some(&table));
            let atlas_source = |kind: &str| {
                crate::resource_source::ResourceSource::indexed(
                    [(
                        "assets/minecraft/atlases/blocks.json".to_owned(),
                        sunrise_assets::resource_source::Resource::bytes(
                            format!(r#"{{"sources":[{{"type":"{kind}"}}]}}"#).into_bytes(),
                        ),
                    )]
                    .into(),
                )
            };

            let _ = take_hook_panics();
            let first_panic = Listing::build(&atlas_source("srctest:panics"), &registered);
            assert!(first_panic.sprites("minecraft:blocks").is_empty());
            assert_eq!(panic_calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "src_panicker",
                    hook: "sprite_source",
                }],
            );

            // Disabled after panic: a second load does not invoke the panicking callback again.
            let second_panic = Listing::build(&atlas_source("srctest:panics"), &registered);
            assert!(second_panic.sprites("minecraft:blocks").is_empty());
            assert_eq!(panic_calls.load(Ordering::Relaxed), 1);

            // Non-panic Err does not latch or queue a panic notice.
            let first_flaky = Listing::build(&atlas_source("srctest:flaky"), &registered);
            assert!(first_flaky.sprites("minecraft:blocks").is_empty());
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 1);
            assert!(take_hook_panics().is_empty());

            let second_flaky = Listing::build(&atlas_source("srctest:flaky"), &registered);
            assert_eq!(flaky_calls.load(Ordering::Relaxed), 2);
            assert_eq!(
                second_flaky
                    .sprites("minecraft:blocks")
                    .keys()
                    .collect::<Vec<_>>(),
                ["srctest:recovered"]
            );

            // Another mod's provider is unaffected by the panic.
            let healthy = Listing::build(&atlas_source("srctest:healthy"), &registered);
            assert_eq!(healthy_calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                healthy
                    .sprites("minecraft:blocks")
                    .keys()
                    .collect::<Vec<_>>(),
                ["srctest:ok"]
            );
        });
    }

    #[test]
    fn an_unregistered_type_fails_its_definition_by_name() {
        let text = r#"{"sources":[{"type":"directory","source":"item","prefix":""},{"type":"srcfree:nobody"}]}"#;
        assert_eq!(
            decode(text, &Mods::none()).unwrap_err().to_string(),
            "sources[1]: unknown sprite source type srcfree:nobody"
        );
        let registered = Mods::new(&["srcfree:nobody", "srcfree:"], |_, _, _, _| Vec::new());
        assert!(matches!(
            decode(text, &registered).unwrap().as_slice(),
            [_, SpriteSource::Registered { kind, .. }] if kind == "srcfree:nobody"
        ));
        // Leading colon resolves into minecraft:, and valid empty-path ids are accepted.
        let leading_and_empty = r#"{"sources":[
            {"type":":single","resource":":item/stick"},
            {"type":"srcfree:"}
        ]}"#;
        assert!(matches!(
            decode(leading_and_empty, &registered).unwrap().as_slice(),
            [
                SpriteSource::Single { resource, .. },
                SpriteSource::Registered { kind, .. },
            ] if resource == "minecraft:item/stick" && kind == "srcfree:"
        ));
    }

    #[test]
    fn the_source_is_asked_with_its_entry_and_nothing_the_entries_before_it_listed() {
        let mut asked = Vec::new();
        list(&definition(), &textures(), &mut |kind, config| {
            assert!(
                matches!(config, Tag::Compound(_)),
                "an object converts to a compound"
            );
            asked.push((kind.to_owned(), config.get("shade").cloned()));
            Vec::new()
        });
        assert_eq!(
            asked,
            // A JSON number converts to the narrowest tag that holds it.
            [("srctest:own".to_owned(), Some(Tag::Byte(7)))]
        );
    }

    #[test]
    fn outputs_apply_in_order_and_a_later_sprite_replaces_an_earlier_one() {
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                // Replaces the directory's apple, then is replaced by the next output.
                pixels("srctest:item/apple", 1, 1, vec![1, 1, 1, 255]),
                SpriteSourceOutput::Texture {
                    sprite: id("srctest:item/apple"),
                    texture: id("srctest:item/gem"),
                },
                pixels("srctest:new", 1, 2, vec![9; 8]),
                // The single source after the mod's replaces this one.
                pixels("srctest:late", 1, 1, vec![0; 4]),
            ]
        });
        assert_eq!(
            listed.sprites["srctest:item/apple"],
            SpriteOrigin::File("srctest:item/gem".into())
        );
        assert_eq!(
            listed.sprites["srctest:new"],
            SpriteOrigin::Pixels((1, 2, vec![9; 8]))
        );
        assert_eq!(
            listed.sprites["srctest:late"],
            SpriteOrigin::File("srctest:item/stick".into())
        );
        assert_eq!(
            listed.by_mods.iter().collect::<Vec<_>>(),
            ["srctest:item/apple", "srctest:new"],
            "the mod's, but not what a later built-in source took back"
        );
        let generated =
            crate::atlas_sources::generate(&listed.sprites["srctest:new"], &|_, _| None);
        assert_eq!(
            generated,
            Some((1, 2, vec![9; 8])),
            "a mod's pixels need no file"
        );
    }

    #[test]
    fn remove_drops_a_listed_sprite_and_an_unlisted_id_changes_nothing() {
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                SpriteSourceOutput::Remove {
                    sprite: id("srctest:item/gem"),
                },
                SpriteSourceOutput::Remove {
                    sprite: id("srctest:never_listed"),
                },
            ]
        });
        let ids: Vec<&str> = listed.sprites.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            ["srctest:item/apple", "srctest:item/stick", "srctest:late"]
        );
    }

    #[test]
    fn what_cant_be_a_sprite_is_skipped_alone() {
        let side = 32768_u32;
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                pixels("srctest:short", 2, 2, vec![0; 15]),
                pixels("srctest:long", 2, 2, vec![0; 17]),
                pixels("srctest:empty", 0, 4, Vec::new()),
                // The product overflows: refused without a panic.
                pixels("srctest:overflow", u32::MAX, u32::MAX, vec![0; 4]),
                pixels(
                    "srctest:too_wide",
                    side + 1,
                    1,
                    vec![0; (side as usize + 1) * 4],
                ),
                SpriteSourceOutput::Texture {
                    sprite: id("srctest:missing"),
                    texture: id("srctest:item/nothing"),
                },
                pixels("srctest:widest", side, 1, vec![0; side as usize * 4]),
                pixels("srctest:good", 2, 2, vec![0; 16]),
            ]
        });
        let ids: Vec<&str> = listed.sprites.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "srctest:good",
                "srctest:item/apple",
                "srctest:item/gem",
                "srctest:item/stick",
                "srctest:late",
                "srctest:widest"
            ]
        );
    }
}
