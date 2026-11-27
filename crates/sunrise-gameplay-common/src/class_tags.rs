//! World-local generated class membership, shared by attachment and typed reads.
use std::collections::{HashMap, HashSet};
use sunrise_items::blocks::BlockFacts;
use sunrise_items::items::Items;
use sunrise_world::level::classes::{Class, is_generated_tag};

fn prepare(
    datapack: &HashMap<String, HashSet<String>>,
    facts: &BlockFacts,
) -> HashMap<String, HashSet<String>> {
    merge(
        datapack,
        facts.states.iter().map(|state| {
            (
                state.name.as_ref(),
                state.classes.iter().map(|name| name.as_ref()),
            )
        }),
    )
}

pub fn merge<'a>(
    datapack: &HashMap<String, HashSet<String>>,
    states: impl IntoIterator<Item = (&'a str, impl IntoIterator<Item = &'a str>)>,
) -> HashMap<String, HashSet<String>> {
    let mut tags = datapack.clone();
    tags.retain(|tag, _| {
        if is_generated_tag(tag) {
            log::warn!("Ignoring reserved generated block tag {tag}");
            false
        } else {
            true
        }
    });
    for class in Class::ALL {
        tags.insert(class.tag().to_owned(), HashSet::new());
    }
    let mut seen = HashSet::new();
    for (block, chain) in states {
        if !seen.insert(block) {
            continue;
        }
        for name in chain {
            if let Some(class) = Class::from_java(name) {
                tags.get_mut(class.tag()).unwrap().insert(block.to_owned());
            }
        }
    }
    tags
}

/// Install generated membership into both views of the same tag table.
pub fn install(items: &mut Items, facts: &BlockFacts) {
    let generated = prepare(&items.block_tags, facts);
    items
        .block_tag_order
        .retain(|tag, _| !is_generated_tag(tag));
    for (tag, members) in &generated {
        if is_generated_tag(tag) {
            let mut members: Vec<_> = members.iter().cloned().collect();
            members.sort_unstable();
            items.block_tag_order.insert(tag.clone(), members);
        }
    }
    items.block_tags = generated;
    // The mob code's flags come from this map, and from these states.
    items.rebuild_block_tag_flags(sunrise_items::items::block_tag_state_names(facts));
}

#[cfg(test)]
mod tests {
    use super::merge;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn class_tags_include_mod_blocks_ancestors_and_deduplicate_states() {
        let datapack = HashMap::from([
            (
                "minecraft:fences".into(),
                HashSet::from(["minecraft:oak_fence".into()]),
            ),
            (
                "sunrise:class/fence".into(),
                HashSet::from(["demo:stone".into()]),
            ),
        ]);
        let mut states = vec![("minecraft:oak_fence", vec!["FenceBlock", "Block"])];
        states.extend((0..4).map(|_| ("demo:rail", vec!["FenceBlock", "Block"])));
        states.push(("demo:stone", vec!["Block"]));
        let tags = merge(&datapack, states.clone());
        assert_eq!(
            tags["sunrise:class/fence"],
            HashSet::from(["minecraft:oak_fence".into(), "demo:rail".into()])
        );
        assert_eq!(tags["sunrise:class/fence"].len(), 2);
        assert!(tags["sunrise:class/block"].contains("demo:rail"));
        assert!(!tags["sunrise:class/wall"].contains("demo:rail"));
        assert!(!tags["minecraft:fences"].contains("demo:rail"));
        let reloaded = merge(
            &HashMap::from([("minecraft:fences".into(), HashSet::new())]),
            states,
        );
        assert_eq!(reloaded["sunrise:class/fence"], tags["sunrise:class/fence"]);
        assert!(reloaded["minecraft:fences"].is_empty());
        let second = merge(&HashMap::new(), [("demo:stone", ["Block"])]);
        assert_eq!(second["sunrise:class/fence"].len(), 0);
    }
}
