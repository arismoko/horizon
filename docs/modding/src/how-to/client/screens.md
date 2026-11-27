# Screens, the inventory key and the sign editor

A client mod can close the screen a player has open, open the player's inventory, and read which sign
the sign editor is on. The two writes take the same paths as the keys the player uses, so the server
sees only the packets of a normal client.

```rust
{{#include ../../../examples/src/client_rendering.rs:screen_controls}}
```

`ClientHost::close_screen()` follows the screen's own Escape close. A container screen closes as the
player's container close: the client sends `ServerboundContainerClosePacket` with the menu's id and then
closes its copy of the menu. It never drops a container menu locally alone, which would leave the
server's menu open. A pause menu closes to the game, another screen runs its own close, and the sign
editor sends `ServerboundSignUpdatePacket` with the lines being edited, as Escape does. A written book
or a book and quill closes and sends nothing: no menu close, and no `ServerboundEditBookPacket`, since
Escape saves no pages (only the editor's Done button does). A lectern's book closes the lectern's menu,
so the client sends `ServerboundContainerClosePacket` with that menu's id.

When a screen opens over another, only the screen in front handles the close. A link's or a command's
confirmation answers no, a server's resource pack prompt declines, and a dialog runs its own Escape.
Declining a required pack disconnects instead of returning to the screen underneath. The death
screen's exit confirmation answers no once its Escape delay has elapsed. The death screen itself,
the code of conduct and a dialog that Escape cannot close do not close this way. The screen hooks'
closing notices fire as they do for Escape. It returns `false` when no screen is open or the screen
in front does not close.

`ClientHost::open_inventory()` does what `key.inventory` does: it opens the player's inventory screen
locally and sends nothing, because the player's menu is always open on the server. Riding a vehicle
with its own inventory (a tamed mount or a chest boat), it sends the open-inventory player command
(`ServerboundPlayerCommandPacket`) at the player's next tick instead, as the key's press is. It returns
`false` with a screen already open, for a spectator, in a portal transition, or with no world.

`ClientHost::sign_editor()` returns `Some(SignEditView { pos, front })` while the sign editor is open:
the sign's block position and whether its front side is being edited. It returns `None` with no sign
editor open, or outside a world. A close of the sign editor always sends the sign update, however it
closes; there is no close that skips it, because the vanilla client never omits that packet.

## World preset editors

Bind a `PresetEditor` to a world preset with `register_preset_editors`. The stock frame draws first,
then `PresetEditor::render` adds the editor's paint, followed by `screen_render_post` and toasts.
If the render callback fails, its paint and draft edits are discarded without removing the stock
frame or skipping Post. A panic disables the bound editor callbacks for the session and reports the
mod's name in the log and an error toast.

While `world_preset_editor` is open, keys consumed by the input method during composition do not
reach `PresetEditor::key_pressed` or the editor's Done and Cancel buttons. Committed characters go
through the usual character hooks and `PresetEditor::char_typed`, separately from key presses.

The editor's Done and Cancel buttons take a left click before `PresetEditor::mouse_clicked(x, y,
button, double_click, draft)`. The editor receives the same `i32` GLFW button index as the screen
hooks. Press Pre, the editor, and press Post get the same `double_click`: the last handled press
must have been on the same screen instance with the same button, less than 250 ms earlier.
Synthetic clicks pass `false` and do not change the history.

If Pre allows a press or release, its button Post runs whether the original screen took or declined
that event. Done and Cancel report `world_preset_editor` with `event.was_click_handled = true`, even
when Create World is already open. A release instead reports its own receiving screen and
`event.was_release_handled`. Post listeners may write `*event.result = TriState::True` to force
handled or `*event.result = TriState::False` to force unhandled; `TriState::Default` keeps the
original answer. The final answer decides whether to record a press for double-click detection and
whether the event can reach gameplay mappings with no screen open. A cancelled mouse Pre reaches
neither the editor nor Post, but a cancelled real press counts as handled. Key and character Post
still run only for unhandled input.

The control socket sends `screen_key_released_pre` to the currently open screen, even if it did
not receive the matching press. For example, when Escape closes the preset editor, its release
goes to Create World, including when Escape is sent as a single tap. If the released key was a
modifier, its bit is already cleared in `event.modifiers` unless the other key for the same
modifier is still held.

## Drawing a dimension transition

In `Hooks::register_dimension_transition_screen`, push a `DimensionTransitionScreen` for
each trip you want to cover. Each mod receives an empty batch at client startup.
`Conditional { to, from }` wins over `Incoming(to)`, then `Outgoing(from)`. The first valid
registration for each key wins.

```rust,ignore
use mod_api::{
    ClientHost, DimensionTransition, DimensionTransitionReason, DimensionTransitionScreen,
    DrawContext, RegisterDimensionTransitionScreenEvent, RegistrationContext,
};

fn register_dimension_transition_screen(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterDimensionTransitionScreenEvent<'_>,
) {
    event.screens.push(DimensionTransitionScreen::new(
        DimensionTransition::Incoming("example:moon".into()),
        |_, initial_reason| {
            // Chosen once when the screen opens, not when the tracker restarts.
            let backdrop = match initial_reason {
                DimensionTransitionReason::NetherPortal => 0xFF301010,
                _ => 0xFF101030,
            };
            Box::new(
                move |_: &dyn ClientHost, _reason, progress, draw: &mut dyn DrawContext| {
                    let [w, h] = draw.size();
                    draw.fill([0.0, 0.0, w, h], backdrop);
                    if let Some(progress) = progress {
                        draw.fill([0.0, h - 8.0, w * progress, 8.0], 0xFF6060FF);
                    }
                },
            )
        },
    ));
}
```

The client calls the matching factory once when it opens a new loading screen for travel.
The returned `Box<dyn DimensionTransitionPresentation>` can hold a closure, as above, or an
object implementing the trait. It draws the entire screen under its registering mod's identity.
Its `reason` and optional smoothed integrated-server `progress` are current each frame.
If another load starts while the screen is open, the same presentation stays in place,
even if the reason changes. The tracker still decides when the screen closes. Login and death
respawn do not look up dimension keys; like unmatched travel, they use vanilla when opening
a new loading screen. The vanilla screen also draws when the matching factory or its presentation
is disabled or refused. A failed draw leaves neither its paint nor its requests for blur, local sound,
particle clears or camera changes. Requests pending before that draw stay in place. A phase refusal only
affects that invocation. A draw panic disables drawing for that registration for the session, but does
not stop the factory from being called on the next matching load. The presentation is dropped as its
registering mod when the screen closes or the client disconnects, even if drawing has already been
disabled. A destructor panic is reported with the mod's name and disables the registration's draw
callback, not the factory. Closing still completes.

## Reading open trades

From a client hook in the world, get the menu from `host.local_player()` and
`PlayerRef::container_menu(host)`, then call `menu.merchant_offers(host)`. The result is an owned
snapshot of the latest offers the server sent to the open merchant menu, in display order. Each
`MerchantOffer` has the first payment, an optional second payment, the result stack and its
components, and stock and pricing data. Use `offer.cost_a_count(host)` for the adjusted first
payment and `offer.out_of_stock()` to check availability.

On a Sunrise server, an edit accepted by `trade_with_villager` reaches this snapshot when the
whole result click commits. A listener that only observes a trade does not move the ordinary
server offer refresh earlier. A take is stale if a listener wrote through the `Host` to a player's
inventory, cursor or menu storage (the take read its own player's; a write to any player's counts).
A refused click sends the authoritative offers and contents again to correct the client's prediction.

`Ok(None)` means the handle is no longer current, doesn't belong to the local player, or isn't a
merchant menu. An open merchant menu can return `Ok(Some(vec![]))` before the offers arrive.
`Err(LevelReadError::Unavailable)` means the snapshot couldn't be read. The call doesn't select
an offer, buy anything, or send packets; the server still decides every trade.

## Frame coordinates

A [registered menu screen factory](../world/machine-menu-data.md#the-menus-screen)
gives one opening its own callbacks. Use `with_draw(|host, frame, draw| ...)`,
`with_click(|host, frame, right| ...)`, or
`with_tooltip(|host, frame, slot, lines| ...)` on the returned `MenuScreen`.
Each callback receives a `MenuScreenFrame`. Its `pointer: [i32; 2]` and `gui_size: [i32; 2]`
are the current absolute pointer and window size in scaled GUI pixels. The absolute
pointer truncates toward zero, including outside the panel. `origin: [f32; 2]`
is the panel's top-left; `at: [f32; 2]` is the pointer relative to that origin,
without truncation. Use these values for hover regions and layout after a resize
or panel move. `frame.data` borrows the current menu's live data slots, which may
not have arrived yet. Use `frame.data.get(index)`: a block-backed menu sends
signed-short values at indices 0 through 32,767, and the negotiated blockless
revision-2 payload sends `i32` values at indices 0 through 65,535, both in snapshots and in deltas.
Higher indices stay on the server. Live data is not the opening payload: decode `open.extra` in the
factory and capture only the owned values the callbacks need.

To draw a sprite from synchronized menu data, see
[Draw a progress bar and a fuel gauge](menu-gauges.md).

## Reading the level from a block screen

In the menu screen factory, take the opening's position from `open.pos` and the verified
mod block ID, if present, from `open.block`. Without a matching server payload, only a
verified clicked opener supplies a position. A menu explicitly opened without a position
gets `None`, not the world origin.

Capture the position by value in `MenuScreen::with_draw(move |host, frame, draw| ...)`.
The read-only callback runs beneath the items and labels and can use ordinary level reads
through the client host, for example to inspect the block beneath a machine. The client view
may lag the server; use `frame.data` for authoritative live menu values.

## Reading the hovered stack

In a client screen hook, `host.screen_hovered_stack()` reads the full stack under the pointer,
including its count and components. It reads container slots and Creative catalog slots, including
Search results. It returns `None` when no container or Creative screen is open, the screen isn't
ready, or the pointer is over no slot, an empty slot or the Creative trash. If you only need the
item key, call `ItemStack::item()` on the returned stack.

## What a screen draws beside its panel

`ScreenView::rect` is a container screen's panel (vanilla's `leftPos`, `topPos`, `imageWidth`, `imageHeight`).
`ScreenView::regions` lists what the same screen drew outside it. Each region is a `ScreenRegionKind`
and a `rect`, `[x, y, width, height]` in GUI pixels from the window's top left, taken from the layout that draws it:

| Kind | Present while | Rectangle (vanilla 26.3) |
|---|---|---|
| `RecipeBook` | the recipe book is open beside the window (a window too narrow draws it over the container instead, and it isn't listed then) | its panel, 147 x 166, left of the container's |
| `RecipeBookTabs` | the recipe book drew tabs | the box of the tabs it drew at `xPosTab` (30 pixels left of the book), 35 x 27 a tab, 27 apart, the selected one 2 further left, an animating one as squeezed; a registered book's tab arrows too |
| `CreativeTab` | the creative inventory is open | one per tab of the page shown, 26 x 32, as drawn by `CreativeModeInventoryScreen.extractTabButton` above and below the panel |
| `Effects` | effects show beside the inventory | the effects' box, compact or full, after `render_inventory_mob_effects` moved or resized it |

A region that's hidden, folded or too narrow to show isn't in the list, and a screen with none has an empty list.
The list is what was drawn: `screen_render_post` sees this frame's drawing (none when `screen_render_pre` cancelled
it). Render Pre, Background and Foreground, like input hooks, see the last published draw for that screen.
The hooks that come before a screen's layout (`screen_opening`,
`screen_closing`, `screen_init_post`, `screen_room`, `render_inventory_mob_effects`) see an empty list.
A mod that lays out its own panel beside a screen (a recipe viewer's item list, an overlay) keeps clear of
`rect` and of every region:

```rust,ignore
fn screen_render_post(&self, _host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
    let screen = event.screen;
    let overlaps = |a: [f32; 4], b: [f32; 4]| {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    };
    let taken = |cell: [f32; 4]| overlaps(screen.rect, cell) || screen.regions.iter().any(|r| overlaps(r.rect, cell));
    // ... lay out the panel's cells, skipping each cell for which `taken(cell)` holds.
}
```

The list is collected after the screen draws, and only while a mod listens to a hook that is given the screen's `ScreenView`, and
it is borrowed, so reading it costs nothing. It holds the game's own regions only. What another mod's screen draws beside its window (an upgrade
column, a row of tabs) is that mod's to report: on NeoForge that's JEI's `IGuiContainerHandler.getGuiExtraAreas`, and
here the viewer mod's own library API.

## Replacing a mod container slot's tooltip

A screen callback installed with `MenuScreen::with_tooltip` runs for a hovered filled slot as well
as an empty slot or screen space. `slot` is the **menu index**, including
the player inventory, or `None` outside a mapped slot. It is not the engine's
internal slot encoding. `frame.at` gives GUI pixels from the screen's origin.

On a filled slot, `lines` starts as the item's tooltip after item tooltip
hooks. Edit, replace or clear those lines: the final list replaces the screen's
`getTooltipFromContainerItem` result before the stack's tooltip image and
`RenderTooltipEvent.GatherComponents`. Empty slots and screen space start with
an empty list. Lines are `(Text, u32)` pairs, not strings: preserve their rich
components and colors when adding a hint or editing another line.

The stack's tooltip image and style, and the rule hiding slot tooltips while
carrying an item (except items such as bundles), are unchanged. This is the
screen-side replacement point, not a second item-tooltip hook. NeoForge's
tooltip events do not carry a slot index; the index here is a Sunrise convenience
over the screen override.

## Moving or hiding a mod screen's slots

The [menu screen factory](../world/machine-menu-data.md#the-menus-screen) puts geometry in
`MenuScreenLayout::slots` at the original backing indices. Only
[permanent exclusions](../world/machine-menu-data.md#custom-menu-screens-and-initially-inactive-slots)
may be `None` or omitted from the end of the list. A layout that changes while open (tabs,
a collapsing panel or a scrolling slot grid) moves or hides slots afterwards with
`ClientHost::set_screen_slot`, as a Java screen sets `Slot.x`/`Slot.y` or deactivates a slot:

```rust,ignore
const MENU: &str = "mymod:station";
// A scrolling grid: slot 3 now sits one row up, 18 pixels from the screen's top-left corner.
host.set_screen_slot(MENU, 3, Some([62.0, 18.0]))?;
// This layout doesn't use slot 4.
host.set_screen_slot(MENU, 4, None)?;
// Show it again on another tab with set_screen_slot(MENU, 4, Some([x, y])).
// Where slot 3 is from the next frame.
assert_eq!(host.screen_slot(MENU, 3), Some([62.0, 18.0]));
```

`menu` is the menu type whose screen is open, so a call made for another screen refuses. Any mod can
read, move, activate, or hide slots of the open screen for `menu`. `slot` ranges over
`0..(container_len + player_slots)`, where `container_len` is `MenuScreenLayout::slots.len()` and
`player_slots` is what the menu declared (`MenuScreenOpen::player_slots`, 0 for a menu with no player
inventory). `0..container_len` indexes `MenuScreenLayout::slots` (including slots declared `None` at open, which
start inactive and can be activated later with `Some([x, y])`). The menu's player slots follow in menu order, the
main inventory's three rows and then the hotbar, as the menu appends them
(`AbstractContainerMenu.addStandardInventorySlots`); each starts at its place on the `inventory_x`/`inventory_y`
grid. A position is GUI pixels from the screen's top-left corner and names the item's top-left
corner, as in `MenuScreenLayout::slots`; `None` hides the slot. The call takes `&self`, so a screen hook such as
`screen_render_post` can move slots every frame; a call that repeats a slot's current place changes nothing and
costs no layout work. The change lasts for that opening, for container and player slots alike: closing the
screen and opening it again, or a replacement even when the server reuses the same window id without a close
packet, starts from the declared layout with every slot shown. `ClientHost::screen_slot(menu, slot)` reads where a slot will be from the next frame, `None` when it is
hidden.

From the next frame a moved slot is a vanilla slot at that position, which is what the screen reads live:

- the hovered slot is the first shown slot in menu order whose 18 by 18 box (one pixel up and left of the item
  square, one beyond it) holds the pointer, so a container slot over the player's inventory or hotbar wins;
- the 24 by 24 highlight is drawn 4 pixels up and left of it;
- its item, count and decorations, and its empty-slot icon, are drawn there;
- the recipe book's ghost items are drawn there.

A hidden slot is none of these, and it is not a drag target or swapped by a number key either: it is not hovered, drawn or listed until the mod puts it somewhere again. A move
or a hide never touches the recipe book: a bound book keeps its catalogue and its placement. The recipe book and
`screen_room` still move the whole image, and every slot with it; a slot's own position never changes for them. A
position outside the image is allowed and is not clamped, and a slot there takes the click instead of making it a
click outside the screen.

The call is client-side only: nothing is sent, and server-side slot rules never change. A locally hidden
slot keeps its item and a shift-click may still fill it, as a deactivated Java slot (`Slot.isActive()`)
does. The server never consults it, so a shift-click from another slot can still fill a hidden player or
container slot: that is the menu's own quick-move answer, and the client adds no filter. Slots declared `None` at
open start inactive and can be activated later with `Some([x, y])`.

Each refusal changes nothing and is named by `ScreenSlotRefusal`:

| Refusal | When |
|---|---|
| `NoScreen` | No mod screen is shown, or the shown one belongs to another menu type |
| `NoSuchSlot` | `slot` is `container_len + player_slots` or more (a menu with no player inventory has no player slots) |
| `BadPosition` | An `x` or `y` that is NaN or infinite |
| `Unavailable` | No game backend: screens are the client's |

A hidden player slot keeps its item. The inventory label stays where the screen puts it; a screen that hides the
player's inventory sets `own_labels` to draw its own.

## Menu slot icons

In the screen factory's `MenuScreenLayout::slots`, give a visible slot a
`Some(ScreenSlot { position, no_item_icon })` entry at its backing index. Set
`no_item_icon` to `Some("minecraft:container/slot/shield".into())` for that GUI atlas sprite,
or `None` for a visible slot without an icon. The geometry and icon are client-side,
including for a screen with a bound recipe book.

The icon replaces an empty displayed stack at 16 × 16 pixels, like vanilla's
`Slot.getNoItemIcon`. An item or a quick-craft preview showing an item takes its place;
a single-slot drag hides all of the slot's contents. Custom backgrounds do not suppress
the icon. A local slot move carries the icon with it, and a local hide suppresses it.
Drawing uses the current GUI atlas without allocating for the lookup and uses its
missing-texture sprite for an unknown name.

## Mod container title and inventory labels

`MenuScreenLayout::title_label` and `MenuScreenLayout::inventory_label` optionally place those two labels,
independently of slot positions. `Some([x, y])` is relative to the screen image's
top-left corner, in GUI pixels. `None` uses vanilla's defaults: title `[8, 6]` and
inventory `[8, height - 94]`. Moving `inventory_x` or `inventory_y` moves slots,
not labels. `own_labels: true` suppresses both engine labels even when positions
are supplied; draw them in the callback you install with `MenuScreen::with_draw`.

The default inventory label is at y 72 for a standard 166-high screen and y 39
for a 133-high hopper layout. Chest layouts use `height - 94`; a shulker box's
167-high image places it at y 73.
When building a complete `MenuScreenLayout` literal, include `title_label: None` and
`inventory_label: None`, or use `..Default::default()`.

## Mounting widgets on a screen

Mount buttons ([`Button`]), single-line text fields ([`TextField`]), and custom widgets
([`ScreenWidget`]) inside `Hooks::screen_init_post` (`ScreenEvent.Init.Post`) through
`event.widgets` (`ScreenWidgets`). This works on your own mod screen (`host.mod_screen() == Some(id)`)
and on vanilla screens alike, matching vanilla's `Screen.addRenderableWidget` and NeoForge's
`ScreenEvent.Init.Post`.

```rust
{{#include ../../../examples/src/widget_screen.rs:widget_screen}}
```

Include `assets/widget_example/lang/en_us.json` in the example mod's resources, and use other
language files for translations. Done uses vanilla's `gui.done`; a note echoed from the text field
stays literal because it is player input.

The engine owns drawing order (mounted widgets draw inside the screen's Pre-guarded contents pass,
before `screen_render_foreground` and tooltips, so cancelling `screen_render_pre` skips them),
hover, focus, unified Tab / Shift+Tab and arrow navigation across native and mounted controls,
controller navigation, narration, and mouse/keyboard dispatch on screens, title screens, and
overlays alike. Mounted widgets are cleared when the screen closes or rebuilds its children
(for example on a resize that calls `Screen.rebuildWidgets`). A reposition-only resize preserves
children and focus, and same-instance resizes preserve `MouseHandler` double-click timing. Keep
text or selection that must survive a rebuild on your mod struct and seed the new widgets from it. Use
`event.widgets.set_initial_focus(id)` to focus a widget mounted by your listener on open
(`Screen.setInitialFocus`). Read `event.widgets.children()` for the ordered child list and
use `event.widgets.remove(child.id)` to remove a listed child (`ScreenEvent.Init.removeListener`).
The list includes earlier accepted mounts and the native controls on the pause menu and the
title screen, in vanilla's order (on the title screen: Singleplayer, Multiplayer, Minecraft Realms,
the Mods button while it shows, the Friends, Language and Accessibility icons, Options..., Quit Game
and the bottom notice), each labelled with its translation key. A removed native control is neither drawn nor
pressed until the screen initializes again. A later
listener sees your edits only if your callback completes normally. There is no live focus setter
or mutable lookup for built-in widgets.
A custom widget can track its own focus via `set_focused` and read or change the state it owns.

### Buttons with your own art

Build an interactive button with `Button::new(rect, label, on_press)` or a disabled one with
`Button::inactive(rect, label)`. Configure initial state with `.active(bool)`, `.visible(bool)`,
and `.sprites(ButtonSprites)`. With no custom art (the default), the button uses the vanilla
nine-slice sprites. With `ButtonSprites`, it uses the same GUI sprite names as
`DrawContext::sprite`:

```rust,ignore
use mod_api::client::widgets::{Button, ButtonSprites};
use mod_api::Text;

let mut art = ButtonSprites::new("mymod:open");
art.highlighted = Some("mymod:open_highlighted".into());
art.disabled = Some("mymod:open_disabled".into());
art.label_offset = [1.0, -1.0];

event.widgets.add_button(
    Button::new([20.0, 24.0, 80.0, 20.0], Text::translate("mymod.screen.open", vec![]), |state, host| {
        state.active = false;
        host.mod_screen_open("mymod");
    })
    .sprites(art),
);
```

Custom art stretches the full sprite into the button's rectangle rather than nine-slicing it.
Inactive takes precedence over hover or focus. An omitted or unknown state sprite falls back to
the custom normal sprite, then to the vanilla nine-slice for that state. `label_offset` is in GUI
pixels and moves only the label, not the hitbox or overflow clip. Mounted buttons register for
narration and controller focus automatically. They activate on left click, on Enter or Space with
keyboard focus, and through controller activation with controller focus. Controller activation
calls the widget without dispatching screen mouse hooks. Inside `on_press`, mutate `&mut ButtonState` (`rect`, `label`, `active`, `visible`,
`sprites`) directly.

### Text fields, IME and double-click

Mount a single-line text field with `TextField::new(rect, max_length)` and configure it with
`.value(...)`, `.hint(...)`, `.editable(bool)`, `.active(bool)`, `.visible(bool)`,
`.bordered(bool)`, `.on_change(|text, host| ...)`, and `.on_submit(|text, host| ...)`.
`max_length` counts UTF-16 code units, following vanilla `EditBox.setMaxLength` and
`EditBox.insertText`. An emoji represented by a surrogate pair needs two units.

The engine owns the `EditBox` layout, horizontal scroll, cursor, selection, and blink state
locally—mods never round-trip layout records or forward screen input hooks manually. Focused
mounted text fields support:

- **IME composition and commit**: enabling OS input methods while focused so preedit and committed
  text land directly in the field (and block screen hotkeys like `E` while focused).
- **Double-click and drag selection**: double-clicking selects the word under the cursor using
  vanilla's 250 ms same-screen same-button `MouseHandler` rule, and dragging extends the selection.
- **Standard editing and clipboard shortcuts**: arrow/Home/End navigation, word movement, Shift
  selection, Backspace/Delete, and Ctrl/Cmd + A/C/V/X.
- **Controller keyboard**: edits go to the focused mounted field. On Chat, completion and
  suggestion/history navigation are available only when the native chat editor owns the keyboard.

### Custom widgets

Implement [`ScreenWidget`] for controls beyond vanilla buttons and text fields and mount them with
`event.widgets.add_widget(Box::new(...))`. Implement `bounds(&self) -> [f32; 4]` and override only
the callbacks you need (`visible`, `active`, `render`, `mouse_clicked`, `mouse_released`,
`mouse_dragged`, `mouse_scrolled`, `key_pressed`, `key_released`, `char_typed`, `captures_input`,
`set_focused`). Every input callback defaults to `false` (unhandled). A pointer press goes only
to the first hit child; if that child declines it, focus stays where it was. Return `true` from
`captures_input()` when a focused custom
widget consumes typing so screen hotkeys yield to it and IME stays enabled. A panic inside a
mounted widget callback disables that mod's widget callbacks for the session and reports an error
naming the mod without breaking other widgets or the screen.

### Opening a mod screen and synchronous transitions

`ClientHost::mod_screen_open(mod_id)` follows vanilla's `Gui.setScreen` and NeoForge's
`ScreenEvent.Opening` / `ScreenEvent.Closing`: it can replace any open vanilla screen, title
screen, overlay (book, sign editor, dialog, or prompt), or another mod's screen. Once hooks allow
the transition, the engine runs the replaced screen's close cleanup (closing container or creative
menus and clearing any overlay), switches to `Screen::Mod` owned by `mod_id`, and runs
`screen_init_post`. It returns `false` when cancelled or redirected by `screen_opening`, while a
world or connection is loading, or without a client window.

The Mods screen's **Config** button opens your screen the same way when you register a config screen factory
([settings](../mod-settings.md#your-own-config-screen)); closing it then returns to the Mods screen.

Widget callbacks (`Button` `on_press`, `TextField` `on_change` / `on_submit`, and `ScreenWidget`
input handlers) receive `&mut dyn ClientHost` and execute screen transitions synchronously: calling
`host.close_screen()`, `host.mod_screen_close(id)`, or `host.mod_screen_open(next_id)` transitions
immediately, matching `Button.onPress` calling `Gui.setScreen`. If a callback switches or closes
the screen, the engine discards the previous screen's mounted widgets once the callback returns.

A dialog moving to its waiting screen or Back from a warning to a live dialog also proposes a
screen replacement, even though both receivers have the id `dialog`. If `screen_opening` cancels,
the current dialog's widgets and focus remain; neither `screen_closing` nor a replacement
`screen_init_post` fires.

When the server closes a container (`ClientboundContainerClosePacket`) and no screen follows,
`screen_opening` doesn't fire and the close can't be cancelled; `screen_closing` still receives
the container screen. When the close opens the title or the death screen instead,
`screen_opening` fires for that screen.

## Screen input Pre and Post hooks

Screen mouse, keyboard, character, and preedit hooks match NeoForge's `ScreenEvent.*` Pre/Post
contract on the **receiving screen**:

- **Receiving screen**: the engine snapshots the open screen at Pre time (including
  `event.screen.mod_screen` when a mod screen is open) and passes that same `ScreenView` to the
  matching Post hook, even if handling closed the screen or opened another mod's screen. Read
  `event.screen.mod_screen` in screen Post hooks rather than `host.mod_screen()`, which always
  reports the screen currently open at the moment it is called. A dialog, prompt, book, lectern,
  or sign editor in front of another screen is itself the receiver.
- **Focused text-box rewrites (`screen_char_typed_pre.field`)**: `event.field` is `Some` for
  vanilla edit boxes and for focused mounted built-in `TextField`s (whose edits and `on_change`
  callback stay in sync), and `None` for custom `ScreenWidget`s (which own their own state).
- **Double-click**: `screen_mouse_button_pressed_pre` and `screen_mouse_button_pressed_post` carry
  `event.double_click`, following vanilla `MouseHandler`'s 250 ms same-screen same-button rule.
- **Mouse press and release Post (`collect`, non-cancellable)**:
  `screen_mouse_button_pressed_post` and `screen_mouse_button_released_post` run after mounted
  widgets and the screen ran, whenever Pre did not cancel. `event.was_click_handled` and
  `event.was_release_handled` are the original answer, even if an earlier listener forced a
  different result. Set `*event.result` to `TriState::True` or `TriState::False` to force handled
  or unhandled; `TriState::Default` uses the original answer. The press Post receives Pre's
  `double_click` bit. The final press result determines whether it enters double-click history.
- **Key press and release Post (unhandled-only, cancellable)**:
  `screen_key_pressed_post` and `screen_key_released_post` fire on the receiving screen only when
  neither mounted widgets nor the screen handled the key. Set `event.cancelled = true` (or return
  `Flow::Cancel`) to mark the key handled so the game's key bindings skip it. A mod browser search
  box can decline a key; the container then handles its raw bindings, including rebound inventory
  close. A handled close or sort does not fire Post; an unhandled slot swap, pick or drop has
  already happened by Post. A mapping reached after Post cannot dispatch a second sort. Cancel Pre
  to prevent a slot action. Window releases, `sunrise-ctl key … up`, and the release phase of
  `key … tap` all deliver release Pre and, if unhandled, Post.
  - Cancelling physical release **Pre** skips the screen and Post, but still clears the device
    key and runs mapped keyboard release cleanup.
  - Cancelling socket release **Pre** skips the screen, Post, and the socket's normal key-up
    handling. A held mapping can stay down.
  - Cancelling unhandled release **Post** skips normal mapped release on either route, after the
    screen declined the event. It does not undo device tracking. Do not cancel a release merely
    to bypass the screen after allowing the mapped press.
- **Unhandled-only notices (non-cancellable)**:
  `screen_mouse_dragged_post`, `screen_mouse_scrolled_post`, `screen_char_typed_post`, and
  `screen_preedit_post` fire on the receiving screen only when neither mounted widgets nor the
  screen handled the input, and cannot cancel (`screen_char_typed_post` returns `()`). Printable
  window text without a physical key and unhandled `sunrise-ctl type` text use the same character
  hooks. Controller keyboard edits go to the focused child without emitting physical input hooks. A built-in
  `TextField` can consume preedit; a custom `ScreenWidget` cannot, even when its `captures_input()`
  returns `true`. A custom editor can observe the unhandled composition in `screen_preedit_post`
  and receives committed characters through `char_typed`.

## Quit Game

`ClientHost::request_client_quit()` asks the local client to exit, even on the title screen with no
world open. It takes a shared host reference. Unlike `close_screen()` or `disconnect()`, it quits
the application rather than just closing the screen or leaving the world. The request does not save,
disconnect, or call shutdown listeners during the hook. After dispatch, the client follows Quit
Game: it disconnects normally and waits for any integrated server to save and release the world lock.
Shutdown failures use the game's normal error reporting.

The method returns `true` when the request is recorded, not when the client successfully shuts down.
It returns `false` if the host has no live client or if a quit is already pending or underway.
For example, to let the player press Q on the title screen while this mod is loaded:

```rust
{{#include ../../../examples/src/title_quit.rs:title_quit}}
```

## Labels: wrapped text without a widget

A label wraps and places a styled component without registering a widget. Call
[`GuiCanvas::text_with_word_wrap`](gui-paint.md#drawing-directly-with-a-canvas) to lay it out and
paint it in one operation, inside the current pose and scissor:

```rust
{{#include ../../../examples/src/client_rendering.rs:scaled_text_lines}}
```

The width is in label-local GUI pixels after scaling, so the text wraps at `max_width / scale`.
Wrapping follows `Font.split`: fitting spaces break lines, overlong words break at a character,
and newline starts another line, including an empty line after a trailing `\n`. A width below
one unscaled pixel still wraps at one pixel. Styles, legacy formatting, inline sprites and player
heads are preserved. A centred line sits at `wrap_width / 2 - floor(line_width / 2)` before
scaling, as in vanilla's `TextAlignment.CENTER.calculateLeft`.

Use [`ClientHost::visit_text_lines`](styled-text-measurements.md) to measure or inspect lines
outside drawing. Both calls return the widest line and the last line's offset plus one line height
(or zero for empty text). A non-finite width, an overflowing `max_width / scale`, or a non-positive
or non-finite scale refuses the layout. There is no separate limit on the number of lines or
characters.

## An item or a model turned in 3D

`DrawContext::model_3d(subject, view)` draws one item or one resolved standalone model in a
screen under a turn of the mod's own: a symmetry wand's screen shows its wand turned about all
three axes and its mirror under a pose it built itself.

```rust,ignore
// 30 degrees about the viewer's axis, as a row-major 3x4 matrix.
let (s, c) = 30f32.to_radians().sin_cos();
let turn = [c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
draw.model_3d(Gui3dSubject::Item(&wand), Gui3dView::new([60.0, 80.0], 64.0, turn))?;
draw.model_3d(Gui3dSubject::Model(self.mirror), Gui3dView::new([120.0, 60.0], 40.0, IDENTITY_TRANSFORM))?;
```

- `Gui3dSubject::Item(&stack)` borrows the stack for the call, leaving it available for another
  draw. The item looks as it does in a slot, without its count, damage bar or cooldown: its model's
  `gui` display transform comes first, then `view.transform`, with the GUI light the model asks
  for. `Gui3dSubject::Model(model)` still takes a handle from `RenderAssets::model` by value. Its
  0 to 1 cube is drawn about its centre, full bright and shaded by face as a scene's blocks are.
- `Gui3dView { at, size, transform }`: `at` is where the subject's centre goes, in GUI pixels,
  and `size` the pixels per block (an item is a slot's size when `size` is 16). `transform` is a
  row-major 3x4 affine matrix applied about the centre in the frame `x` right, `y` up, `z`
  towards the viewer, so a rotation about any axis, roll included, needs no agreed Euler order.
  Animate by passing a new matrix each frame.
- The view is orthographic. Both subjects have a depth of their own: a draw never intersects
  what the screen drew before or after it, and later drawing covers it in call order. An item's
  faces meet each other by depth, as a model's do, and its enchantment glint follows them. The
  OptiFine CIT glint layers of a stack are not drawn on this path. There is no z input. Use
  [`DrawContext::with_scissor`](#clipping-what-a-screen-draws) to clip either subject.
- In an available draw context, a `minecraft:air` stack is a no-op (`Ok(())`) even with an
  invalid view or unreadable component overrides, matching `GuiGraphicsExtractor.item`. Refusals draw nothing: `Gui3dError::Unavailable` (no game, or a
  server), `NotFinite` (a non-finite view value or transformed corner, or a non-positive size),
  `UnknownItem` (an unknown key or an item missing from the current registry), `InvalidStack`
  (component overrides that cannot be converted for drawing), and `Model(ModelError)` (a stale
  or unknown handle).

## Room beside a container

`Hooks::screen_room` lets a client mod reserve room for a panel to the left or right of an open container.
This is a layout query, not a drawing or cancellation hook. It does not add recipe properties,
slot membership or ghost-placement APIs.

```rust,ignore
fn screen_room(&self, _: &dyn ClientHost, event: &mut ScreenRoomEvent<'_>) {
    let panel_width_with_gap = 149.0;
    event.right = event.right.max(panel_width_with_gap);
    // A panel on the other side writes `event.left` the same way.
}
```

Import `ClientHost`, `Hooks` and `ScreenRoomEvent` from `mod_api` and implement the method in your subscribed `Hooks` implementation. Keep the query pure and cheap: do not draw, mutate game state or do expensive work here.

Each side's answer starts at zero. Later listeners see earlier answers and keep the maximum, not a sum. Panels asking for 80 and 149 GUI pixels reserve 149 pixels together, not 229. They share the reservation and must coordinate their own contents; the engine does not stack panels or assign per-mod offsets.

The query's `event.screen.rect` is the **pre-reservation** container rectangle. Later drawing and input `ScreenView` values expose the shifted rectangle. Do not subtract the shift yourself from those later views.

Left room is measured from the open recipe book's left edge, including its selected category tab, and right room from the window's right edge. When the left room, the book, the window and the right room all fit, the window and book move right by half the left room less half the right room, each half rounded down. Slots, hit boxes and effect panels follow in the same frame. Otherwise, or when a mod menu's recipe book toggle would leave the screen, nothing moves on either side. A panel should check the later window rectangle (and the book's left edge, for a left panel) plus its required room against the viewport before drawing there. A request is not a guarantee of space.

A negative or non-finite final answer counts as zero. Zero and absent listeners preserve the original layout. A finite request too large to fit also preserves it. The hook runs for the inventory, containers and custom menus, but not Creative, Merchant or Beacon screens, or a narrow screen where the recipe book covers the menu.

The answer is queried at most once per frame and screen and reused by every layout consumer that frame. A recursive layout query during dispatch sees zero, avoiding recursive dispatch. The next frame queries again. A geometry-changing answer rebuilds the layout that frame; unchanged geometry reuses the existing build, even if the numerical answer changed without affecting the resulting layout.

## Clipping what a screen draws

A scrolling list or a viewport into a larger page shows only the part inside a box.
`DrawContext::with_scissor(rect, &mut |clipped| { ... })` cuts draws made through `clipped` to
`[x, y, w, h]` in GUI pixels, like vanilla's `GuiGraphics.enableScissor` and `disableScissor`
around the callback. The full drawing context is available inside, including widgets, scenes,
items and text. The callback returns `()` and the clip does not roll back drawing; handle errors
from draw methods inside the callback.

```rust,ignore
fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
    let [x, y, w, h] = self.list_box;
    let _ = event.draw.with_scissor([x, y, w, h], &mut |clipped| {
        for (row, line) in self.lines.iter().enumerate() {
            let top = y + row as f32 * 12.0 - self.scroll;
            clipped.text(line, [x + 2.0, top], 0xFFFF_FFFF, false);
        }
    });
}
```

- **Nesting.** An inner clip is intersected with the outer clip, so it never shows more than that
  one. A nonpositive width or height, or a disjoint inner clip, clips everything. The enclosing
  clip is restored on return or unwind, before any subsequent draws.
- **Depth.** Scenes keep their own box too. Cutting an item or scene preserves depth: a cut face
  still draws in front of or behind its neighbours as it did uncut.
- **Refusals.** `GuiClipRefusal::NotFinite` (a coordinate, size or computed endpoint is NaN or
  infinite) or `Unavailable` (no scissor drawing surface) leaves the enclosing clip as it was and
  does not call the callback.

For a fallible, transactional group of GUI primitives [on a canvas](gui-paint.md#drawing-directly-with-a-canvas),
pass `Some(rect)` to `canvas.scope`. That scissor uses the child scope's pose at entry; later
pose changes do not move it.

## Hover and pointer grab

A mod screen hears every pointer move through `screen_mouse_moved_pre` (vanilla's
`Screen.mouseMoved`; it fires before `screen_mouse_dragged_pre` when a button is held). For a
screen that wants relative motion with the cursor held (a virtual machine's display, a 3D
preview), call `host.set_pointer_grab(true)` from your click hook and read
`screen_pointer_motion`; `host.pointer_grabbed()` tells whether you hold it, and
`host.set_pointer_grab(false)` releases it. The grab also ends when the screen closes, the window
loses focus or your mod unloads, and refocusing does not bring it back: grab again. Escape reaches
your screen like any key, so handle it yourself. The rustdoc of the hooks and `set_pointer_grab` has the contract.

## Dropped files

`screen_files_dropped(screen, paths)` tells a mod that files were dropped on the window while a
screen is open, a vanilla screen or yours (vanilla's `Screen.onFilesDrop`). It fires once per drop
with every path in the order the operating system gave them; a path that is not valid UTF-8 comes
through unchanged, so open it as a `Path`. The game reads no file, so read the ones you want
yourself. The hook only observes: it cannot cancel the drop, and the game's own handling of it (a
dropped resource pack) still runs. Nothing is sent over the network.
