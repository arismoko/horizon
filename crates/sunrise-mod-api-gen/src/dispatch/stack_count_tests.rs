use super::*;

#[test]
#[ignore = "daily"]
fn only_container_loot_filled_changes_when_stack_count_marker_is_removed() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let mut catalog = Catalog::load(&root).unwrap();
            let generate = |catalog: &Catalog| {
                emit(
                    catalog.hooks.iter().filter(|hook| !is_render(hook)),
                    &catalog.draw_contexts,
                    true,
                )
                .unwrap()
            };
            let marked = generate(&catalog);
            assert_eq!(
                catalog
                    .hooks
                    .iter_mut()
                    .find(|hook| hook.name == "container_loot_filled")
                    .unwrap()
                    .stack_count
                    .take(),
                Some(crate::catalog::StackCount::Trim)
            );
            let ordinary = generate(&catalog);
            assert_eq!(marked.traits, ordinary.traits);
            assert_eq!(marked.dispatch, ordinary.dispatch);
            assert_eq!(marked.render_dispatch, ordinary.render_dispatch);
            assert_eq!(marked.part_traits, ordinary.part_traits);
            assert_eq!(marked.hook_ids, ordinary.hook_ids);
            assert_eq!(marked.hook_constants, ordinary.hook_constants);
            assert_eq!(marked.cancel_hooks, ordinary.cancel_hooks);
            assert_eq!(marked.hook_count, ordinary.hook_count);
            let fills = |sources: &HookSources| {
                sources
                    .fills
                    .iter()
                    .map(|fill| (fill.native.clone(), fill.call.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(fills(&marked), fills(&ordinary));
            let marked = render_fire(&marked.fire);
            let ordinary = render_fire(&ordinary.fire);
            let marked_lines: Vec<_> = marked.lines().collect();
            let ordinary_lines: Vec<_> = ordinary.lines().collect();
            assert_eq!(marked_lines.len(), ordinary_lines.len());
            let mut changes = 0;
            for (marked, ordinary) in marked_lines.iter().zip(&ordinary_lines) {
                if marked != ordinary {
                    changes += 1;
                    assert!(marked.contains("fn fire_container_loot_filled("));
                    assert!(marked.contains("try_into_engine_preserving_count"));
                    assert!(!ordinary.contains("try_into_engine_preserving_count"));
                }
            }
            assert_eq!(changes, 1);
            assert!(!marked.contains("native_hooks::container_loot_filled"));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn count_policy_applies_only_to_the_named_output() {
    let stacks: Type = syn::parse_quote!(&mut Vec<ItemStack>);
    let output = ("drops".into(), &stacks);
    let crossing = stack_crossing_routed(
        &[("extra".into(), &stacks)],
        Some(&output),
        true,
        false,
        "AnyCollector",
        false,
        None,
        false,
        &Default::default(),
        Some("drops"),
    )
    .unwrap();
    let (extra, drops) = crossing
        .before
        .split_once("let mut __journal_drops")
        .unwrap();
    assert!(extra.contains("value.try_into_engine(&**items)"));
    assert!(!extra.contains("value.try_into_engine_preserving_count"));
    assert!(drops.contains("value.try_into_engine_preserving_count(&**items)"));
}

#[test]
fn declared_stack_collectors_use_the_same_count_conversion() {
    let hook = Hook {
        name: "any_collector".into(),
        contract: Contract::Collect,
        out: Some("drops".into()),
        call: vec![("drops".into(), "&mut Vec<ItemStack>".into())],
        stack_count: Some(crate::catalog::StackCount::Trim),
        ..Hook::sample(
            "sunrise_client::game::native_hooks::any::Game<'_>::native_any_collector",
            Side::Server,
        )
    };
    let sources = emit(std::iter::once(&hook), &[], true).unwrap();
    let fire = render_fire(&sources.fire);
    assert!(fire.contains("try_into_engine_preserving_count"));
    assert!(fire.contains("let AnyCollectorCall { drops } = call;"));
    syn::parse_file(&fire).unwrap();
}
