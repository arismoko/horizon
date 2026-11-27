//! `AnvilScreen`'s rename box: the 50-character `EditBox`, read-only while
//! the input slot is empty (`AnvilScreen.slotChanged`).
#[cfg(feature = "client")]
use super::{Station, StationUi};
pub(in crate::game) use sunrise_gameplay_common::game::stations_common::NAME_CHARS;

/// `AnvilScreen.slotChanged`: the box shows the input's name, and only an
/// input item makes it editable (`name.setEditable(!itemStack.isEmpty())`).
/// A read-only box still keeps the focus, the caret keys and copy.
#[cfg(feature = "client")]
pub(in crate::game::client_side) fn input_changed(
    station: &mut Station,
    ui: &mut StationUi,
    name: String,
) {
    station.name = name;
    let value = station.name.clone();
    ui.rename.set_value(&value);
    ui.rename.editable = station.inputs[0].is_some();
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::super::{Kind, Station, StationUi, rename_box};
    use super::input_changed;

    #[test]
    fn the_rename_box_is_read_only_without_an_input_item() {
        // AnvilScreen.init: no input yet, so the box is read-only but focused.
        let station_box = rename_box();
        assert!(station_box.focused && !station_box.editable);
        assert!(!station_box.invert_highlighted_text_color);

        // slotChanged with an item: editable, and it shows the item's name.
        let mut station = Station::new_kind(Kind::Anvil, [0; 3]);
        let mut ui = StationUi::new();
        station.inputs[0] = Some(sunrise_items::items::ItemStack::new(
            "minecraft:diamond_sword",
            1,
        ));
        input_changed(&mut station, &mut ui, "Diamond Sword".into());
        assert!(ui.rename.editable);
        assert_eq!(ui.rename.value, "Diamond Sword");

        // The input is taken out again: read-only, and slotChanged's
        // `setValue("")` empties it (the caller passes the empty name).
        station.inputs[0] = None;
        input_changed(&mut station, &mut ui, String::new());
        assert!(!ui.rename.editable);
        assert_eq!(ui.rename.value, "");
        ui.rename.set_value("Diamond Sword");

        // A read-only box still keeps the caret keys; Backspace is consumed
        // without taking a character.
        assert!(
            ui.rename
                .key(winit::keyboard::KeyCode::ArrowLeft, false, false)
        );
        assert!(
            ui.rename
                .key(winit::keyboard::KeyCode::Backspace, false, false)
        );
        assert_eq!(ui.rename.value, "Diamond Sword");
    }
}

// The previews in a server session read the server's enchantments (#8688).
