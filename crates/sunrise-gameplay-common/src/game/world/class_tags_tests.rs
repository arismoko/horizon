use super::class_tags::merge;
use std::collections::{HashMap, HashSet};

#[test]
fn class_tags_dispatch_and_failed_preparation_preserve_installed_snapshot() {
    use crate::game::api_values::{self, TryIntoApi};
    use mod_api::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_world::registry::{BlockRegistry, BlockStateId};
    struct Counter(Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl BlockPart for Counter {
        fn analog_output(&self, _: &mut dyn LevelRandomHost, _: &mut AnalogOutputEvent) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct ReloadGuard;
    #[mod_api::sunrise_part]
    impl BlockPart for ReloadGuard {}
    struct Listener(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.block_part_tag("sunrise:class/fence", Counter(self.0.clone()));
            // This key is valid in the installed world but absent in the attempted reload.
            let stone = r.block_key("demo:stone");
            r.block_part(stone, ReloadGuard);
        }
    }
    struct Level;
    impl LevelHost for Level {
        fn block_entity_data(&self, _: mod_api::BlockEntityRef) -> Option<mod_api::Compound> {
            None
        }
    }
    impl mod_api::LevelRandomHost for Level {}
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new("demo", Box::new(Listener(calls.clone())))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let registry = BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:oak_fence":0,"demo:rail":1,"demo:stone":5,"minecraft:air":6},"states":[["o",false,false,"","minecraft:oak_fence"],["r0",false,false,"",{"id":"demo:rail","properties":{"variant":"0"}}],["r1",false,false,"",{"id":"demo:rail","properties":{"variant":"1"}}],["r2",false,false,"",{"id":"demo:rail","properties":{"variant":"2"}}],["r3",false,false,"",{"id":"demo:rail","properties":{"variant":"3"}}],["s",false,false,"","demo:stone"],["a",true,false,"","minecraft:air"]]}"#.as_slice()).unwrap();
        api_values::resolve_block_definitions(&registry).unwrap();
        let states = || {
            [
                ("minecraft:oak_fence", vec!["FenceBlock", "Block"]),
                ("demo:rail", vec!["FenceBlock", "Block"]),
                ("demo:stone", vec!["Block"]),
            ]
        };
        let tags = merge(
            &HashMap::from([(
                "minecraft:fences".into(),
                HashSet::from(["minecraft:oak_fence".into()]),
            )]),
            states(),
        );
        let prepare = |tags: &HashMap<String, HashSet<String>>, valid: bool| {
            mod_api::__private::prepare_parts(
                tags,
                &HashMap::new(),
                mod_api::__private::KnownIds {
                    block: &|id| valid || id != "demo:stone",
                    item: &|_| false,
                },
            )
        };
        mod_api::__private::install_resolved_parts(prepare(&tags, true).unwrap());
        let mut items = sunrise_items::items::Items::empty();
        items.set_tags(
            sunrise_items::items::TagRegistry::Block,
            tags.iter()
                .map(|(tag, members)| (tag.clone(), members.iter().cloned().collect()))
                .collect(),
        );
        let class_fence = mod_api::__private::tag_key(
            &mod_api::Identifier::parse("sunrise:class/fence").unwrap(),
        )
        .unwrap()
        .typed();
        let visit = |id| {
            let mut output = 0;
            mod_api::dispatch_analog_output(
                &mut Level,
                LevelDimension::Overworld,
                sunrise_core::position::BlockPos::new(0, 64, 0),
                BlockStateId(id).try_into_api().unwrap(),
                sunrise_core::geometry::Direction::North,
                &mut output,
            );
        };
        visit(0);
        visit(1);
        visit(5);
        assert_eq!(calls.swap(0, Ordering::SeqCst), 2);
        visit(5);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        visit(1);
        assert_eq!(calls.swap(0, Ordering::SeqCst), 1);
        let reloaded = merge(
            &HashMap::from([("minecraft:fences".into(), HashSet::new())]),
            states(),
        );
        mod_api::__private::install_resolved_parts(prepare(&reloaded, true).unwrap());
        visit(1);
        assert_eq!(calls.swap(0, Ordering::SeqCst), 1);
        let attempted = merge(&HashMap::new(), [("demo:stone", ["Block"])]);
        assert!(prepare(&attempted, false).is_err());
        let fence = BlockStateId(1).try_into_api().unwrap().block();
        let oak = BlockStateId(0).try_into_api().unwrap().block();
        assert_eq!(
            [fence, oak]
                .into_iter()
                .filter(|key| crate::game::native_tags::block_in_tag(
                    Some(&items),
                    *key,
                    class_fence
                ) == Some(true))
                .count(),
            2
        );
        visit(0);
        visit(1);
        visit(5);
        assert_eq!(calls.swap(0, Ordering::SeqCst), 2);
        mod_api::__private::install_resolved_parts(prepare(&attempted, true).unwrap());
        visit(1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    });
}
