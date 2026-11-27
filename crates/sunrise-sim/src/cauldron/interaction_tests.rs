use super::*;
struct Hand {
    stack: Option<ItemStack>,
    dropped: Vec<ItemStack>,
}
impl Player for Hand {
    fn creative(&self) -> bool {
        false
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.stack
    }
    fn add(&mut self, _: &mut ItemStack) -> bool {
        false
    }
    fn contains(&self, _: &ItemStack) -> bool {
        false
    }
    fn drop_stack(&mut self, stack: ItemStack) {
        self.dropped.push(stack);
    }
}
#[test]
fn cauldron_no_mod_water_bucket_and_bottle_keep_vanilla_values() {
    let mut hand = Hand {
        stack: Some(ItemStack::new("minecraft:water_bucket", 1)),
        dropped: vec![],
    };
    let result = interact(&state("minecraft:cauldron", None), false, false, &mut hand);
    assert_eq!(result.outcome, Outcome::Success);
    assert_eq!(
        result.block,
        Some(state("minecraft:water_cauldron", Some(3)))
    );
    assert_eq!(hand.stack, Some(ItemStack::new("minecraft:bucket", 1)));
    assert_eq!(result.sound, Some("minecraft:item.bucket.empty"));
    assert_eq!(result.fluid_event, Some("minecraft:fluid_place"));
    assert!(hand.dropped.is_empty());
    hand.stack = Some(ItemStack::new("minecraft:glass_bottle", 1));
    let result = interact(
        &state("minecraft:water_cauldron", Some(3)),
        false,
        false,
        &mut hand,
    );
    assert_eq!(result.outcome, Outcome::Success);
    assert_eq!(
        result.block,
        Some(state("minecraft:water_cauldron", Some(2)))
    );
    assert_eq!(
        hand.stack,
        Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"))
    );
    assert_eq!(result.sound, Some("minecraft:item.bottle.fill"));
    assert_eq!(result.fluid_event, Some("minecraft:fluid_pickup"));
    assert!(hand.dropped.is_empty());
}
#[test]
fn washing_a_raid_banner_removes_just_its_last_layer() {
    let banner = crate::mobs::raider::ominous_banner();
    let mut hand = Hand {
        stack: Some(banner.clone()),
        dropped: vec![],
    };
    let result = interact(
        &state("minecraft:water_cauldron", Some(3)),
        false,
        false,
        &mut hand,
    );
    assert_eq!(
        result.block,
        Some(state("minecraft:water_cauldron", Some(2)))
    );
    assert!(hand.dropped.is_empty());
    let washed = hand.stack.unwrap();
    assert_eq!(washed.banner_layers(), &banner.banner_layers()[..7]);
    let original = crate::item_data::encode_extra_components(&banner).unwrap();
    let washed = crate::item_data::encode_extra_components(&washed).unwrap();
    for id in [
        "minecraft:item_name",
        "minecraft:rarity",
        "minecraft:tooltip_display",
    ] {
        assert_eq!(washed.get(id), original.get(id));
    }
}

#[test]
fn cauldron_prebound_entries_overwrite_in_place_and_keep_tag_priority() {
    use interactions::{Dispatcher, Entry};
    let mut table = Dispatcher::<&str>::default();
    assert_eq!(
        table.register(
            "minecraft:cauldron".into(),
            "minecraft:water_bucket".into(),
            "mod_water",
        ),
        None
    );
    assert!(matches!(
        table.select(
            "minecraft:cauldron",
            "minecraft:water_bucket",
            false,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "mod_water"
    ));
    assert_eq!(
        table.register(
            "minecraft:water_cauldron".into(),
            "minecraft:bucket".into(),
            "mod_bucket",
        ),
        None
    );
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:bucket",
            false,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "mod_bucket"
    ));
    assert_eq!(
        table.register(
            "minecraft:water_cauldron".into(),
            "minecraft:leather_helmet".into(),
            "first",
        ),
        None
    );
    assert_eq!(
        table.register(
            "minecraft:water_cauldron".into(),
            "minecraft:leather_helmet".into(),
            "second",
        ),
        Some("first")
    );
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            true,
            |_| false
        ),
        Some(Entry::Dye)
    ));
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            false,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "second"
    ));
    table.register_tag(
        "minecraft:water_cauldron".into(),
        "minecraft:cauldron_can_remove_dye".into(),
        "custom_dye",
    );
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            true,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "custom_dye"
    ));
    table.register_tag(
        "minecraft:water_cauldron".into(),
        "minecraft:cauldron_can_remove_dye".into(),
        "custom_dye_v2",
    );
    assert!(matches!(
        table.select(
            "minecraft:water_cauldron",
            "minecraft:leather_helmet",
            true,
            |_| false
        ),
        Some(Entry::Mod(owner)) if *owner == "custom_dye_v2"
    ));
}
#[test]
fn cauldron_tag_entries_precede_exact_ones_and_overwrite_in_place() {
    use interactions::{Dispatcher, Entry};
    fn pick<'a>(
        table: &Dispatcher<&'a str>,
        item: &str,
        in_tag: impl Fn(&str) -> bool,
    ) -> Option<&'a str> {
        match table.select("minecraft:water_cauldron", item, false, in_tag) {
            Some(Entry::Mod(owner)) => Some(*owner),
            _ => None,
        }
    }
    let mut table = Dispatcher::<&str>::default();
    let water = || "minecraft:water_cauldron".to_owned();
    table.register(water(), "minecraft:milk_bucket".into(), "exact");
    table.register_tag(water(), "c:buckets/milk".into(), "milk");
    table.register_tag(water(), "c:buckets".into(), "buckets");
    // Membership is asked at use time: matching tags precede exact item entries.
    let milk = |tag: &str| matches!(tag, "c:buckets/milk" | "c:buckets");
    assert_eq!(pick(&table, "minecraft:milk_bucket", milk), Some("milk"));
    assert_eq!(
        pick(&table, "minecraft:milk_bucket", |_| false),
        Some("exact")
    );
    assert_eq!(
        pick(&table, "othermod:goat_milk_bucket", milk),
        Some("milk")
    );
    assert!(
        table
            .select(
                "minecraft:water_cauldron",
                "othermod:goat_milk_bucket",
                false,
                |_| false
            )
            .is_none()
    );
    // Overwriting an existing tag replaces it at its original position before later tags
    // and returns the displaced handler.
    assert_eq!(
        table.register_tag(water(), "c:buckets/milk".into(), "milk_v2"),
        Some("milk")
    );
    assert_eq!(
        pick(&table, "othermod:goat_milk_bucket", milk),
        Some("milk_v2")
    );
    assert_eq!(
        table.register_tag(water(), "c:buckets/milk".into(), "milk_v3"),
        Some("milk_v2")
    );
    assert_eq!(
        pick(&table, "othermod:goat_milk_bucket", milk),
        Some("milk_v3")
    );
    assert_eq!(
        pick(&table, "othermod:oil_bucket", |tag| tag == "c:buckets"),
        Some("buckets")
    );
    // A tag entry on a custom block declares that block.
    table.register_tag("test:syrup_cauldron".into(), "c:buckets".into(), "custom");
    assert!(table.contains_block("test:syrup_cauldron"));
    assert!(matches!(
        table.select("test:syrup_cauldron", "othermod:goat_milk_bucket", false, milk),
        Some(Entry::Mod(owner)) if *owner == "custom"
    ));
}
