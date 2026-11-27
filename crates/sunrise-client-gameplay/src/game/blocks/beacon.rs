//! Beacons (`BeaconBlockEntity`, `BeaconMenu`, `BeaconScreen`,
//! `BeaconRenderer`): the beam sections tinted by stained glass, the pyramid
//! levels every 80 ticks, the primary/secondary effects for players in
//! range, the activate/ambient/deactivate sounds, the 230x219 menu with its
//! power buttons and payment slot, and the rotating, scrolling beam.
use super::stations::{
    INPUT,
    tooltip::{WidgetTooltipPositioner, quantize_tooltip_anchor},
};
use sunrise_client::{hud::HudBatch, mesher::Vertex};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_game_data::text::Text;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::beacon_common::{EFFECTS, effect_box_hits};
use sunrise_items::items::ItemStack;
use sunrise_level::{containers::BlockEntity, test_instance_block::Status};
use sunrise_protocol::packets::play::{self, Serverbound as S};

/// The beacon screen is 54 px wider than the 176-px menus it shares an origin with.
pub(in crate::game) const SHIFT: f32 = -27.0;
pub(in crate::game) const HEIGHT: f32 = 219.0;
/// `BeaconScreen.extractBackground`'s payment row in drawing order, with each
/// item's x: netherite, emerald, diamond, gold, iron. Not by value: the
/// emerald is left of the diamond.
const PAYMENT_ROW: [(&str, f32); 5] = [
    ("minecraft:netherite_ingot", 20.0),
    ("minecraft:emerald", 41.0),
    ("minecraft:diamond", 63.0),
    ("minecraft:gold_ingot", 86.0),
    ("minecraft:iron_ingot", 108.0),
];
/// `BeaconMenu.PaymentSlot` and the player inventory, relative to the 176 origin.
pub(in crate::game::client_side) fn slot_position(i: usize) -> Option<(f32, f32)> {
    Some(match i {
        0..9 => (36.0 + SHIFT + i as f32 * 18.0, 195.0),
        9..36 => (
            36.0 + SHIFT + ((i - 9) % 9) as f32 * 18.0,
            137.0 + ((i - 9) / 9) as f32 * 18.0,
        ),
        INPUT => (136.0 + SHIFT, 110.0),
        _ => return None,
    })
}
/// A power button: (effect index, x, y, primary) relative to the beacon's left edge.
fn buttons() -> Vec<(usize, f32, f32, bool)> {
    let mut out = Vec::new();
    let tiers: [&[usize]; 3] = [&[0, 1], &[2, 3], &[4]];
    for (tier, effects) in tiers.iter().enumerate() {
        let count = effects.len() as i32;
        let total = count * 22 + (count - 1) * 2;
        for (c, &e) in effects.iter().enumerate() {
            let x = 76 + c as i32 * 24 - total / 2;
            out.push((e, x as f32, 22.0 + tier as f32 * 25.0, true));
        }
    }
    // Tier 4: regeneration, then the upgrade (primary II) button.
    let total = 2 * 22 + 2;
    out.push((5, (167 - total / 2) as f32, 47.0, false));
    out
}
fn upgrade_button() -> (f32, f32) {
    let total = 2 * 22 + 2;
    ((167 + 24 - total / 2) as f32, 47.0)
}
fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
/// `BeaconPowerButton.setEffect`'s tooltip: the effect's description, and for
/// `BeaconUpgradePowerButton` (`createEffectDescription`) " II" after it.
fn effect_tooltip(e: usize, upgraded: bool) -> Text {
    let effect = Text::translate(&format!("effect.minecraft.{}", EFFECTS[e].0), &[]);
    if upgraded {
        effect.append(" II")
    } else {
        effect
    }
}
/// Tooltips exist on both active and inactive beacon buttons. Return the origin of the button
/// matched by `hit`, for either pointer hover or keyboard focus.
fn button_tooltip(
    primary: Option<usize>,
    buttons: &[(usize, f32, f32, bool)],
    hit: impl Fn(f32, f32) -> bool,
) -> Option<(Text, [f32; 2])> {
    for &(e, x, y, _) in buttons {
        if hit(x, y) {
            return Some((effect_tooltip(e, false), [x, y]));
        }
    }
    if let Some(p) = primary {
        let (x, y) = upgrade_button();
        if hit(x, y) {
            return Some((effect_tooltip(p, true), [x, y]));
        }
    }
    for (x, key) in [(164.0, "gui.done"), (190.0, "gui.cancel")] {
        if hit(x, 107.0) {
            return Some((Text::translate(key, &[]), [x, 107.0]));
        }
    }
    None
}
impl crate::game::ClientGame<'_> {
    /// `TestInstanceBlockEntity.getBeamSections`: grey while running, green
    /// for a pass, red for a failed required test and orange for an optional
    /// one; each section is one block tall and the last runs to the sky.
    fn test_instance_beam(&self, e: &BlockEntity) -> Option<Vec<(u32, i32)>> {
        let BlockEntity::TestInstanceBlock(b) = e else {
            return None;
        };
        let color = match b.data.status {
            Status::Cleared => return None,
            Status::Running => 0x80_8080,
            Status::Finished if b.data.error_message.is_none() => 0x00_FF00,
            Status::Finished => {
                let required = b
                    .data
                    .test
                    .as_deref()
                    .and_then(|key| {
                        let (ns, path) = key.split_once(':').unwrap_or(("minecraft", key));
                        let file = self
                            .common
                            .options
                            .resources
                            .join("data")
                            .join(ns)
                            .join("test_instance")
                            .join(format!("{path}.json"));
                        let json: serde_json::Value =
                            serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
                        sunrise_gameplay_common::test_instances::parse_test_def(&json)
                    })
                    .is_none_or(|t| t.required);
                if required { 0xFF_0000 } else { 0xFF_8000 }
            }
        };
        Some(vec![(color, 1)])
    }

    /// `ClientLevel.tickBlockEntities` for the beacon at `key`: a server's chunk data
    /// carries no beam sections, so the client builds them too; effects,
    /// sounds and the advancement stay with the server.
    pub(in crate::game) fn client_tick_beacon(&mut self, key: (i32, i32, i32)) {
        let Some((lease, BlockEntity::Beacon(mut b))) = self.common.world.block_entities.take(&key)
        else {
            return;
        };
        let _ = self.advance_beacon_beam(key, &mut b);
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Beacon(b));
    }
    fn beacon_levels(&self) -> i32 {
        let Some((s, ui)) = self.station_and_ui() else {
            return 0;
        };
        if let Some(levels) = ui.beacon_levels {
            return levels;
        }
        let pos = s.pos;
        match self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        {
            Some(BlockEntity::Beacon(b)) => b.levels,
            _ => 0,
        }
    }
    /// Clicks on the power, confirm and cancel buttons.
    /// The effect, upgrade, confirm and cancel buttons a controller can
    /// focus (GUI origin relative).
    pub(in crate::game::client_side) fn beacon_targets(&self) -> Vec<[f32; 4]> {
        let (ux, uy) = upgrade_button();
        buttons()
            .into_iter()
            .map(|(_, x, y, _)| (x, y))
            .chain([(ux, uy), (164.0, 107.0), (190.0, 107.0)])
            .map(|(x, y)| [x + SHIFT, y, 22.0, 22.0])
            .collect()
    }
    pub(in crate::game::client_side) fn beacon_click(&mut self, local: (f32, f32)) -> bool {
        let l = (local.0 - SHIFT, local.1);
        let levels = self.beacon_levels();
        let Some(sel) = self.common.station.as_ref().map(|s| s.beacon) else {
            return false;
        };
        for (e, x, y, primary) in buttons() {
            if !inside(l, x, y, 22.0, 22.0) {
                continue;
            }
            let tier = EFFECTS[e].1;
            let selected = if primary {
                sel[0] == Some(e)
            } else {
                sel[1] == Some(e)
            };
            if tier <= levels.min(if primary { 3 } else { 4 })
                && !selected
                && (primary || levels >= 4)
            {
                if let Some(s) = self.common.station.as_mut() {
                    if primary {
                        s.beacon[0] = Some(e);
                        if s.beacon[1] != Some(e) {
                            s.beacon[1] = None;
                        }
                    } else {
                        s.beacon[1] = Some(e);
                    }
                }
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            }
            return true;
        }
        let (ux, uy) = upgrade_button();
        if inside(l, ux, uy, 22.0, 22.0) {
            if levels >= 4
                && let Some(p) = sel[0]
                && sel[1] != Some(p)
                && let Some(s) = self.common.station.as_mut()
            {
                s.beacon[1] = Some(p);
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            }
            return true;
        }
        if inside(l, 164.0, 107.0, 22.0, 22.0) {
            let paid = self
                .common
                .station
                .as_ref()
                .is_some_and(|s| s.inputs[0].is_some());
            if paid && sel[0].is_some() {
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                self.confirm_beacon(sel);
                self.close_beacon_screen();
            }
            return true;
        }
        if inside(l, 190.0, 107.0, 22.0, 22.0) {
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            self.close_beacon_screen();
            return true;
        }
        false
    }
    fn close_beacon_screen(&mut self) {
        self.close_container_client();
        self.set_screen(super::Screen::Playing);
        self.capture(true);
    }
    /// `BeaconScreen`'s confirm button sends `ServerboundSetBeaconPacket` to the server.
    fn confirm_beacon(&mut self, sel: [Option<usize>; 2]) {
        if self.client.remote.is_some() {
            let id = |e: Option<usize>| e.and_then(|e| self.remote_effect_network_id(EFFECTS[e].0));
            let (primary, secondary) = (id(sel[0]), id(sel[1]));
            self.send(S::SetBeacon(play::SetBeacon { primary, secondary }));
        }
    }
    /// `BeaconScreen`: background, labels, payment icons, power buttons.
    pub(in crate::game::client_side) fn draw_beacon(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = self.inventory_origin(batch);
        let (bx, by) = (ox + SHIFT, oy);
        let local = (self.client.cursor.0 - bx, self.client.cursor.1 - by);
        let levels = self.beacon_levels();
        let sel = self.common.station.as_ref().map_or([None; 2], |s| s.beacon);
        let paid = self
            .common
            .station
            .as_ref()
            .is_some_and(|s| s.inputs[0].is_some());
        // BeaconScreen draws no title, but has the menu's.
        super::screen_info::name_container(|| {
            self.remote_menu_title().map_or_else(
                || self.translate_with_fallback("container.beacon", "Beacon"),
                |t| t.plain(),
            )
        });
        self.render_transparent_background("beacon", batch);
        let atlas = &self.client.gui.atlas;
        if let Some(mut r) = atlas.sprites.get("gui/container/beacon").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 230.0 * du;
            r.v1 = r.v0 + HEIGHT * dv;
            batch.rect_uv(bx, by, 230.0, HEIGHT, r, u32::MAX);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let label = 0xFFE0_E0E0;
        for (key, fallback, cx) in [
            ("block.minecraft.beacon.primary", "Primary Power", 62.0),
            ("block.minecraft.beacon.secondary", "Secondary Power", 169.0),
        ] {
            let text = self.translate_with_fallback(key, fallback);
            let atlas = &self.client.gui.atlas;
            let w = HudBatch::text_width(atlas, &text);
            batch.text(
                atlas,
                &text,
                bx + cx - (w / 2.0).floor(),
                by + 10.0,
                label,
                false,
            );
        }
        for (item, x) in PAYMENT_ROW {
            self.draw_item(batch, &ItemStack::new(item, 1), bx + x, by + 109.0);
        }
        let atlas = &self.client.gui.atlas;
        let buttons = buttons();
        let anchor = quantize_tooltip_anchor(self.widget_tooltip_cursor());
        let rect = |x, y| {
            let [left, top] = quantize_tooltip_anchor((bx + x, by + y));
            [left, top, 22, 22]
        };
        let is_hovered = |[left, top, width, height]: [i32; 4]| {
            (left..left + width).contains(&anchor[0]) && (top..top + height).contains(&anchor[1])
        };
        let focused = if self.key_focus_driving() {
            self.pad_focus_rect()
                .map(|rect| rect.map(|value| value as i32))
        } else {
            None
        };
        // A focused widget's deferred tooltip takes precedence over other widgets' hover tips.
        let tooltip = focused
            .and_then(|focus| button_tooltip(sel[0], &buttons, |x, y| rect(x, y) == focus))
            .or_else(|| button_tooltip(sel[0], &buttons, |x, y| is_hovered(rect(x, y))))
            .map(|(text, [x, y])| {
                let bounds = rect(x, y);
                let positioner = if is_hovered(bounds) {
                    WidgetTooltipPositioner::Menu(bounds)
                } else {
                    WidgetTooltipPositioner::BelowOrAbove(bounds)
                };
                (text, positioner)
            });
        let button =
            |batch: &mut HudBatch, x: f32, y: f32, active: bool, selected: bool, icon: &str| {
                let hover = inside(local, x, y, 22.0, 22.0);
                let sprite = if !active {
                    "button_disabled"
                } else if selected {
                    "button_selected"
                } else if hover {
                    "button_highlighted"
                } else {
                    "button"
                };
                batch.sprite(atlas, &format!("container/beacon/{sprite}"), bx + x, by + y);
                batch.sprite_scaled(
                    atlas,
                    icon,
                    bx + x + 2.0,
                    by + y + 2.0,
                    18.0,
                    18.0,
                    u32::MAX,
                );
            };
        for (e, x, y, primary) in buttons {
            let tier = EFFECTS[e].1;
            let active = if primary {
                tier <= levels && tier < 4
            } else {
                levels >= 4
            };
            let selected = if primary {
                sel[0] == Some(e)
            } else {
                sel[1] == Some(e)
            };
            let icon = format!("mob_effect/{}", EFFECTS[e].0);
            button(batch, x, y, active, selected, &icon);
        }
        if let Some(p) = sel[0] {
            let (x, y) = upgrade_button();
            let icon = format!("mob_effect/{}", EFFECTS[p].0);
            button(batch, x, y, levels >= 4, sel[1] == Some(p), &icon);
        }
        button(
            batch,
            164.0,
            107.0,
            paid && sel[0].is_some(),
            false,
            "container/beacon/confirm",
        );
        button(batch, 190.0, 107.0, true, false, "container/beacon/cancel");
        let slots: Vec<usize> = (0..36).chain([INPUT]).collect();
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        for i in slots {
            let (sx, sy) = self.slot_position(i);
            if let Some(stack) = self.slot_mut(i).clone() {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        let mut stack_tip = None;
        if let Some(slot) = self.slot_at(batch, self.client.cursor.0, self.client.cursor.1) {
            stack_tip = self.slot_mut(slot).clone();
        }
        let browser_tip = self.draw_mod_browser(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if self.common.carried.is_some() {
            self.draw_carried(batch);
            if let Some(stack) = stack_tip.filter(|s| self.tooltip_with_carried(s)) {
                self.draw_stack_tooltip(batch, &stack, self.client.cursor.0, self.client.cursor.1);
            }
        } else if let Some(stack) = stack_tip {
            self.draw_stack_tooltip(batch, &stack, self.client.cursor.0, self.client.cursor.1);
        } else if let Some((text, positioner)) = tooltip {
            self.draw_tooltip_lines_positioned(batch, &[(text, u32::MAX)], anchor, &positioner);
        }
        if let Some(tooltip) = browser_tip {
            self.draw_mod_browser_tooltip(batch, tooltip);
        }
    }
    /// `BeaconRenderer`: each section's rotating solid beam and its glow.
    pub fn beacon_geometry(&mut self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let Some(s) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/beacon/beacon_beam")
            .copied()
        else {
            return;
        };
        let time = (self.common.world.game_time.rem_euclid(40)) as f32 + alpha as f32;
        let scoping = self.scoping();
        // `BeaconRenderer.shouldRender` and `TestInstanceRenderer.shouldRender` cull at the
        // effective render distance.
        let view = self.effective_render_distance();
        let beams = self
            .common
            .world
            .block_entities
            .iter()
            .filter_map(|(k, e)| {
                let (reach, sections) = match beam(e) {
                    Some(sections) => (Reach::Beacon, std::borrow::Cow::Borrowed(sections)),
                    None => (
                        Reach::TestInstance,
                        std::borrow::Cow::Owned(self.test_instance_beam(e)?),
                    ),
                };
                Some((k, reach, sections))
            });
        for (k, reach, sections) in beams {
            let base = glam::DVec3::new(f64::from(k.0) + 0.5, f64::from(k.1), f64::from(k.2) + 0.5);
            if !reach.should_render([k.0, k.1, k.2], camera, view) {
                continue;
            }
            let offset = base - camera;
            let distance = glam::DVec2::new(offset.x, offset.z).length() as f32;
            let scale = if scoping {
                1.0
            } else {
                (distance / 96.0).max(1.0)
            };
            let origin = (glam::DVec3::new(f64::from(k.0), f64::from(k.1), f64::from(k.2))
                - camera)
                .as_vec3();
            let start = out.len();
            sunrise_client::block_entity_render::beacon_beam(
                origin, &sections, time, scale, s, out,
            );
            // HDR output: the beam shines (entity.wgsl fs_hdr_mark).
            for v in &mut out[start..] {
                v.layer |= u32::from(sunrise_client::mesher::BRIGHT) << 16;
            }
        }
    }
}
/// The beam sections of an active beacon (`BeaconRenderer` draws no beam for
/// a beacon without a pyramid or with all sections cleared).
fn beam(e: &BlockEntity) -> Option<&[(u32, i32)]> {
    match e {
        BlockEntity::Beacon(b) if b.levels > 0 && !b.sections.is_empty() => Some(&b.sections),
        _ => None,
    }
}

use sunrise_gameplay_common::game::beacon_common::BeaconCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_level::containers::Beacon;
    /// `BeaconScreen.extractBackground`: netherite 20, emerald 41, diamond
    /// 41+22, gold 42+44, iron 42+66.
    #[test]
    fn the_payment_row_reads_netherite_emerald_diamond_gold_iron() {
        assert_eq!(
            PAYMENT_ROW,
            [
                ("minecraft:netherite_ingot", 20.0),
                ("minecraft:emerald", 41.0),
                ("minecraft:diamond", 63.0),
                ("minecraft:gold_ingot", 86.0),
                ("minecraft:iron_ingot", 108.0),
            ]
        );
    }
    /// `BeaconUpgradePowerButton.createEffectDescription` appends " II"; the
    /// primary buttons' description is the bare effect.
    #[test]
    fn only_the_upgrade_button_names_the_second_level() {
        for e in 0..EFFECTS.len() {
            let primary = effect_tooltip(e, false).plain();
            let upgrade = effect_tooltip(e, true).plain();
            assert!(!primary.is_empty());
            assert!(!primary.ends_with(" II"));
            assert_eq!(upgrade, format!("{primary} II"));
        }
    }
    /// `BeaconScreenButton.setTooltip`: every button has one while hovered,
    /// and the sprite buttons' is their label whether or not they are active
    /// (the tick is disabled without a payment and a chosen power).
    #[test]
    fn every_hovered_beacon_button_shows_its_tooltip() {
        let at = |(x, y): (f32, f32)| (x + 11.0, y + 11.0);
        let name =
            |e: usize| sunrise_game_data::lang::get(&format!("effect.minecraft.{}", EFFECTS[e].0));
        let buttons = buttons();
        let hovered_tooltip = |at, primary| {
            button_tooltip(primary, &buttons, |x, y| inside(at, x, y, 22.0, 22.0))
                .map(|(text, _)| text)
        };
        for &(e, x, y, _) in &buttons {
            let tip = hovered_tooltip(at((x, y)), None).unwrap_or_default();
            assert_eq!(tip.plain(), name(e), "button {e}");
        }
        let upgrade = at(upgrade_button());
        assert!(hovered_tooltip(upgrade, None).is_none());
        assert_eq!(
            hovered_tooltip(upgrade, Some(0)).unwrap().plain(),
            format!("{} II", name(0))
        );
        assert_eq!(
            hovered_tooltip(at((164.0, 107.0)), None)
                .unwrap_or_default()
                .plain(),
            sunrise_game_data::lang::get("gui.done")
        );
        assert_eq!(
            hovered_tooltip(at((190.0, 107.0)), Some(0))
                .unwrap_or_default()
                .plain(),
            sunrise_game_data::lang::get("gui.cancel")
        );
        assert!(hovered_tooltip((0.0, 0.0), Some(0)).is_none());
    }
    #[test]
    fn only_active_beacons_have_beams() {
        let mut b = Beacon::default();
        assert!(beam(&BlockEntity::Beacon(Box::new(b.clone()))).is_none());
        b.levels = 1;
        b.sections.push((0xFFFFFF, 10));
        assert_eq!(
            beam(&BlockEntity::Beacon(Box::new(b))),
            Some(&[(0xFFFFFF, 10)][..])
        );
    }
    #[test]
    fn effect_box_uses_player_overlap() {
        let hit = |x: f64, y: f64| {
            effect_box_hits(
                [0, 100, 0],
                50.0,
                384.0,
                [x - 0.3, y, -0.3],
                [x + 0.3, y + 1.8, 0.3],
            )
        };
        assert!(hit(-50.2, 100.0));
        assert!(!hit(-50.4, 100.0));
        assert!(hit(51.2, 100.0));
        assert!(!hit(51.4, 100.0));
        assert!(hit(0.0, 48.3));
        assert!(!hit(0.0, 48.1));
        assert!(hit(0.0, 534.9));
        assert!(!hit(0.0, 535.0));
    }
}
