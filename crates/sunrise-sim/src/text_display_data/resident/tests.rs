use super::*;

#[test]
fn item_metadata_encodes_the_stack_only_when_sent() {
    let stack = Arc::new(ItemStack {
        item: "minecraft:diamond".into(),
        count: 3,
        damage: 0,
        extra: None,
    });
    let display = DisplayState::new(DisplayContents::Item {
        stack: Some(stack.clone()),
        context: ItemDisplayContext::Head,
    });
    let mut metadata = Vec::new();
    display.append_metadata(
        None,
        false,
        |borrowed| {
            assert!(std::ptr::eq(borrowed, stack.as_ref()));
            Some(Stack::new(42, 3))
        },
        &mut metadata,
    );
    assert_eq!(
        metadata,
        [
            (
                23,
                serializer::ITEM_STACK,
                Value::Item(Some(Stack::new(42, 3)))
            ),
            (24, serializer::BYTE, Value::Byte(5)),
        ],
    );

    metadata.clear();
    let mut changed = display.clone();
    changed.contents = DisplayContents::Item {
        stack: Some(stack),
        context: ItemDisplayContext::Gui,
    };
    changed.append_metadata(
        Some(&display),
        false,
        |_| panic!("unchanged stack"),
        &mut metadata,
    );
    assert_eq!(metadata, [(24, serializer::BYTE, Value::Byte(6))]);

    metadata.clear();
    let mut emptied = changed.clone();
    emptied.contents = DisplayContents::Item {
        stack: None,
        context: ItemDisplayContext::Gui,
    };
    emptied.append_metadata(Some(&changed), false, |_| panic!("no stack"), &mut metadata);
    assert_eq!(metadata, [(23, serializer::ITEM_STACK, Value::Item(None))]);
}

#[test]
fn item_metadata_omits_nonempty_stack_when_encoder_is_unavailable() {
    let stack = Arc::new(ItemStack {
        item: "minecraft:diamond".into(),
        count: 3,
        damage: 0,
        extra: None,
    });
    let display = DisplayState::new(DisplayContents::Item {
        stack: Some(stack),
        context: ItemDisplayContext::Head,
    });
    let mut metadata = Vec::new();
    display.append_metadata(None, false, |_| None, &mut metadata);
    assert_eq!(metadata, [(24, serializer::BYTE, Value::Byte(5))]);
}
