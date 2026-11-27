use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Draws the open station or brewing stand screen; false when none is open.
    pub fn draw_station(&mut self, batch: &mut HudBatch) -> bool {
        if let Some(kind) = self.common.station.as_ref().map(|s| s.kind) {
            if self.client.station_ui.is_none() {
                debug_assert!(false, "open client station missing StationUi");
                return false;
            }
            if kind == Kind::Merchant {
                return self.draw_merchant(batch);
            }
            if kind == Kind::Beacon {
                self.draw_beacon(batch);
                return true;
            }
        }
        let brewing = self.brewing_open();
        if self.common.station.is_none() && !brewing {
            return false;
        }
        // `SmithingScreen.containerTick`: the icons of the empty slots follow the template.
        let (now, partial) = (
            self.visual_ticks(),
            self.common.clock.camera_fraction() as f32,
        );
        if let Some((s, ui)) = self
            .station_and_ui_mut()
            .filter(|(s, _)| s.kind == Kind::Smithing)
        {
            let template = s.inputs[0].as_ref().map(|t| t.item.clone());
            let (base, base_len, additions, additions_len) =
                smithing_slot_icons(template.as_deref());
            let lists: [&[&'static str]; 3] = [
                &EMPTY_TEMPLATE_ICONS,
                &base[..base_len],
                &additions[..additions_len],
            ];
            for (cycling, icons) in ui.cycling.iter_mut().zip(lists) {
                cycling.advance(now, icons);
            }
        }
        let (ox, oy) = self.inventory_origin(batch);
        let local = (self.client.cursor.0 - ox, self.client.cursor.1 - oy);
        let mut tooltip: Option<Vec<(sunrise_game_data::text::Text, u32)>> = None;
        let mut anvil_name = false;
        let mut recipe_tooltip: Option<ItemStack> = None;
        {
            self.render_transparent_background("station", batch);
        }
        let inventory_label = self.translate_with_fallback("container.inventory", "Inventory");
        if brewing {
            let Some(BlockEntity::Brewing(b)) = self.common.container_kind() else {
                return true;
            };
            let (fuel, total_fuel, brew, total_brew) =
                (b.fuel, b.total_fuel, b.brew_time, b.total_brew_time);
            let atlas = &self.client.gui.atlas;
            Self::part(
                batch,
                atlas,
                "gui/container/brewing_stand",
                (0.0, 0.0),
                (176.0, 166.0),
                (ox, oy),
            );
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            let fuel_len = if total_fuel > 0 {
                ((18 * fuel + total_fuel - 1) / total_fuel).clamp(0, 18)
            } else {
                0
            };
            if fuel_len > 0 {
                Self::part(
                    batch,
                    atlas,
                    "container/brewing_stand/fuel_length",
                    (0.0, 0.0),
                    (fuel_len as f32, 4.0),
                    (ox + 60.0, oy + 44.0),
                );
            }
            if brew > 0 && total_brew > 0 {
                let len = (28.0 * (1.0 - brew as f32 / total_brew as f32)) as i32;
                if len > 0 {
                    Self::part(
                        batch,
                        atlas,
                        "container/brewing_stand/brew_progress",
                        (0.0, 0.0),
                        (9.0, len as f32),
                        (ox + 97.0, oy + 16.0),
                    );
                }
                let bubble = [29, 24, 20, 16, 11, 6, 0][(brew / 2 % 7) as usize] as f32;
                if bubble > 0.0 {
                    Self::part(
                        batch,
                        atlas,
                        "container/brewing_stand/bubbles",
                        (0.0, 29.0 - bubble),
                        (12.0, bubble),
                        (ox + 63.0, oy + 14.0 + 29.0 - bubble),
                    );
                }
            }
            let default = self.translate_with_fallback("container.brewing", "Brewing Stand");
            let title = self.remote_menu_title().unwrap_or_else(|| {
                let named = self
                    .common
                    .station
                    .as_ref()
                    .and_then(|s| self.common.block_name_plain((s.pos[0], s.pos[1], s.pos[2])));
                Text::literal(named.unwrap_or(default)).resolve_legacy()
            });
            crate::game::screen_info::name_container(|| title.plain());
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ox + (176.0 - tw) / 2.0,
                oy + 6.0,
                GREY,
                false,
            );
            batch.text(atlas, &inventory_label, ox + 8.0, oy + 72.0, GREY, false);
            // Empty-slot icons.
            for (i, icon) in [
                (300, "potion"),
                (301, "potion"),
                (302, "potion"),
                (304, "brewing_fuel"),
            ] {
                if self.slot_mut(i).is_none() {
                    let (sx, sy) = self.station_slot_position(i).unwrap_or_default();
                    batch.sprite(
                        &self.client.gui.atlas,
                        &format!("container/slot/{icon}"),
                        ox + sx,
                        oy + sy,
                    );
                }
            }
        } else if let Some(kind) = self.common.station.as_ref().map(|s| s.kind) {
            // A server's menu has the title it was given.
            let title = self.remote_menu_title().unwrap_or_else(|| {
                Text::literal(self.translate_with_fallback(kind.title_key(), ""))
            });
            crate::game::screen_info::name_container(|| title.plain());
            {
                let atlas = &self.client.gui.atlas;
                Self::part(
                    batch,
                    atlas,
                    kind.texture(),
                    (0.0, 0.0),
                    (176.0, 166.0),
                    (ox, oy),
                );
                self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
                let (tx, ty) = kind.title_pos();
                batch.text_styled(atlas, &title, ox + tx, oy + ty, GREY, false);
                batch.text(atlas, &inventory_label, ox + 8.0, oy + 72.0, GREY, false);
            }
            if kind == Kind::Cartography {
                self.draw_cartography(batch, ox, oy);
            }
            if kind == Kind::Loom {
                tooltip = self.draw_loom(batch, ox, oy, local);
            }
            let atlas = &self.client.gui.atlas;
            let Some(s) = self.common.station.as_ref() else {
                return false;
            };
            let Some(ui) = self.client.station_ui.as_ref() else {
                debug_assert!(false, "open client station missing StationUi");
                return false;
            };
            let has_input = s.inputs.iter().any(Option::is_some);
            match kind {
                Kind::Enchanting => {
                    let (open, flip) = ui.book.at(self.common.clock.camera_fraction() as f32);
                    sunrise_client::item_render::gui::draw_enchanting_book(
                        &self.client.client_pack.item_renderer,
                        &atlas.sprites,
                        batch,
                        ox + 14.0,
                        oy + 14.0,
                        open,
                        flip,
                    );
                    if s.inputs[1].is_none() {
                        batch.sprite(atlas, "container/slot/lapis_lazuli", ox + 35.0, oy + 47.0);
                    }
                    let mut names = LegacyRandom::new(i64::from(self.common.level.xp_seed));
                    let gold = s.inputs[1].as_ref().map_or(0, |l| l.count as i32);
                    let level = self.common.experience.level;
                    let offers = s.offers.clone();
                    for i in 0..3 {
                        let (bx, by, bw, bh) = enchant_button(i);
                        let cost = offers.costs[i];
                        if cost == 0 {
                            batch.sprite_scaled(
                                atlas,
                                "container/enchanting_table/enchantment_slot_disabled",
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            continue;
                        }
                        let cost_text = cost.to_string();
                        let text_width = 86.0 - HudBatch::text_width(atlas, &cost_text);
                        let count = names.next_int_bound(2).unwrap_or(0) + 3;
                        let words: Vec<&str> = (0..count)
                            .map(|_| {
                                WORDS
                                    [names.next_int_bound(WORDS.len() as i32).unwrap_or(0) as usize]
                            })
                            .collect();
                        let message = words.join(" ");
                        let disabled =
                            (gold < i as i32 + 1 || level < cost) && !self.common.creative;
                        let col: i32 = if disabled {
                            batch.sprite_scaled(
                                atlas,
                                "container/enchanting_table/enchantment_slot_disabled",
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            batch.sprite(
                                atlas,
                                &format!("container/enchanting_table/level_{}_disabled", i + 1),
                                ox + bx + 1.0,
                                oy + by + 1.0,
                            );
                            let c = ((-9937334i32) & 16711422) >> 1;
                            let msg_color = argb(c | 0xFF00_0000u32 as i32);
                            self.galactic_deferred(
                                batch,
                                &message,
                                ox + bx + 20.0,
                                oy + by + 2.0,
                                text_width,
                                msg_color,
                            );
                            -12550384
                        } else {
                            let hover = inside(local, bx, by, bw, bh);
                            let sprite = if hover {
                                "enchantment_slot_highlighted"
                            } else {
                                "enchantment_slot"
                            };
                            batch.sprite_scaled(
                                atlas,
                                &format!("container/enchanting_table/{sprite}"),
                                ox + bx,
                                oy + by,
                                bw,
                                bh,
                                u32::MAX,
                            );
                            batch.sprite(
                                atlas,
                                &format!("container/enchanting_table/level_{}", i + 1),
                                ox + bx + 1.0,
                                oy + by + 1.0,
                            );
                            let msg_color = if hover { -128 } else { -9937334 };
                            self.galactic_deferred(
                                batch,
                                &message,
                                ox + bx + 20.0,
                                oy + by + 2.0,
                                text_width,
                                argb(msg_color),
                            );
                            -8323296
                        };
                        let atlas = &self.client.gui.atlas;
                        let cw = HudBatch::text_width(atlas, &cost_text);
                        batch.text(
                            atlas,
                            &cost_text,
                            ox + bx + 20.0 + 86.0 - cw,
                            oy + by + 9.0,
                            argb(col),
                            true,
                        );
                        // Tooltip with the clue (EnchantmentScreen.extractRenderState).
                        if inside(local, bx, by, bw, 17.0)
                            && let Some((id, l)) = &offers.clues[i]
                        {
                            let e = self.active_enchantment_definitions();
                            let (name, _) = e.full_name(id, *l);
                            let clue = self
                                .translate_with_fallback("container.enchant.clue", "%s . . . ?")
                                .replace("%s", &name);
                            let mut lines =
                                vec![(sunrise_game_data::text::Text::literal(clue), u32::MAX)];
                            if !self.common.creative {
                                lines.push((sunrise_game_data::text::Text::literal(""), u32::MAX));
                                if level < cost {
                                    lines.push((
                                        sunrise_game_data::text::Text::literal(
                                            self.translate_with_fallback(
                                                "container.enchant.level.requirement",
                                                "Level Requirement: %s",
                                            )
                                            .replace("%s", &cost.to_string()),
                                        ),
                                        rgba(255, 85, 85, 255),
                                    ));
                                } else {
                                    let n = i + 1;
                                    let lapis = if n == 1 {
                                        self.translate_with_fallback(
                                            "container.enchant.lapis.one",
                                            "1 Lapis Lazuli",
                                        )
                                    } else {
                                        self.translate_with_fallback(
                                            "container.enchant.lapis.many",
                                            "%s Lapis Lazuli",
                                        )
                                        .replace("%s", &n.to_string())
                                    };
                                    lines.push((
                                        sunrise_game_data::text::Text::literal(lapis),
                                        if gold >= n as i32 {
                                            rgba(170, 170, 170, 255)
                                        } else {
                                            rgba(255, 85, 85, 255)
                                        },
                                    ));
                                    let levels = if n == 1 {
                                        self.translate_with_fallback(
                                            "container.enchant.level.one",
                                            "1 Enchantment Level",
                                        )
                                    } else {
                                        self.translate_with_fallback(
                                            "container.enchant.level.many",
                                            "%s Enchantment Levels",
                                        )
                                        .replace("%s", &n.to_string())
                                    };
                                    lines.push((
                                        sunrise_game_data::text::Text::literal(levels),
                                        rgba(170, 170, 170, 255),
                                    ));
                                }
                            }
                            tooltip = Some(lines);
                        }
                    }
                }
                Kind::Anvil => {
                    let field = if s.inputs[0].is_some() {
                        "text_field"
                    } else {
                        "text_field_disabled"
                    };
                    batch.sprite_scaled(
                        atlas,
                        &format!("container/anvil/{field}"),
                        ox + 59.0,
                        oy + 20.0,
                        110.0,
                        16.0,
                        u32::MAX,
                    );
                    if has_input && s.result.is_none() {
                        batch.sprite(atlas, "container/anvil/error", ox + 99.0, oy + 45.0);
                    }
                    if s.inputs[0].is_some() {
                        // Drawn once this borrow of the station ends.
                        anvil_name = true;
                    }
                    let cost = s.anvil.cost;
                    if cost > 0 {
                        let mut color = argb(-8323296);
                        let line = if cost >= 40 && !self.common.creative {
                            color = argb(-40864);
                            Some(self.translate_with_fallback(
                                "container.repair.expensive",
                                "Too Expensive!",
                            ))
                        } else if s.result.is_none() {
                            None
                        } else {
                            if !self.may_take_result() {
                                color = argb(-40864);
                            }
                            Some(
                                self.translate_with_fallback(
                                    "container.repair.cost",
                                    "Enchantment Cost: %1$s",
                                )
                                .replace("%1$s", &cost.to_string()),
                            )
                        };
                        if let Some(line) = line {
                            let atlas = &self.client.gui.atlas;
                            let tw = HudBatch::text_width(atlas, &line);
                            let tx = 176.0 - 8.0 - tw - 2.0;
                            batch.fill(
                                atlas,
                                ox + tx - 2.0,
                                oy + 67.0,
                                176.0 - 8.0 - (tx - 2.0),
                                12.0,
                                argb(1325400064),
                            );
                            batch.text(atlas, &line, ox + tx, oy + 69.0, color, true);
                        }
                    }
                }
                Kind::Grindstone => {
                    if has_input && s.result.is_none() {
                        batch.sprite(atlas, "container/grindstone/error", ox + 92.0, oy + 31.0);
                    }
                }
                Kind::Smithing => {
                    self.draw_smithing_armor_stand(
                        batch,
                        s.result.as_ref(),
                        [ox + 121.0, oy + 20.0, ox + 161.0, oy + 80.0],
                    );
                    let mut onboarding: Option<String> = None;
                    // `CyclingSlotBackground.extractRenderState`: an empty slot shows the
                    // icons of what the template in the table takes.
                    for (i, cycling) in ui.cycling.iter().enumerate() {
                        if s.inputs[i].is_some() {
                            continue;
                        }
                        let (sx, sy) = kind.input_pos(i);
                        for (icon, alpha) in cycling.shown(partial) {
                            batch.sprite_scaled(
                                atlas,
                                &format!("container/slot/{icon}"),
                                ox + sx,
                                oy + sy,
                                16.0,
                                16.0,
                                ((alpha.clamp(0.0, 1.0) * 255.0).round() as u32) << 24
                                    | 0x00FF_FFFF,
                            );
                        }
                    }
                    if s.inputs.iter().all(Option::is_some) && s.result.is_none() {
                        batch.sprite(atlas, "container/smithing/error", ox + 65.0, oy + 46.0);
                        if inside(local, 65.0, 46.0, 28.0, 21.0) {
                            onboarding = Some(self.translate_with_fallback(
                                "container.upgrade.error_tooltip",
                                "Item can't be upgraded this way",
                            ));
                        }
                    }
                    // SmithingScreen.extractOnboardingTooltips: an empty template
                    // slot asks for a template; an empty base or addition slot says
                    // what the template wants there.
                    let hovered = (0..3).find(|&i| {
                        // AbstractContainerScreen.isHovering: a pixel round the slot.
                        let (sx, sy) = kind.input_pos(i);
                        inside(local, sx - 1.0, sy - 1.0, 18.0, 18.0)
                    });
                    match (hovered, &s.inputs[0]) {
                        (Some(0), None) => {
                            onboarding = Some(self.translate_with_fallback(
                                "container.upgrade.missing_template_tooltip",
                                "Add Smithing Template",
                            ));
                        }
                        (Some(slot @ (1 | 2)), Some(template))
                            if s.inputs[slot].is_none()
                                && template.item.ends_with("_smithing_template") =>
                        {
                            let name = if &*template.item
                                == "minecraft:netherite_upgrade_smithing_template"
                            {
                                "netherite_upgrade"
                            } else {
                                "armor_trim"
                            };
                            let what = if slot == 1 { "base" } else { "additions" };
                            let key = format!(
                                "item.minecraft.smithing_template.{name}.{what}_slot_description"
                            );
                            onboarding = Some(self.translate_with_fallback(&key, &key));
                        }
                        _ => {}
                    }
                    if let Some(text) = onboarding {
                        // font.split(component, 115).
                        tooltip = Some(
                            self.client
                                .gui
                                .atlas
                                .font
                                .split_lines(&sunrise_game_data::text::Text::literal(text), 115.0)
                                .into_iter()
                                .map(|(line, _)| (line, u32::MAX))
                                .collect(),
                        );
                    }
                }
                Kind::Merchant | Kind::Beacon | Kind::Cartography | Kind::Loom => {}
                Kind::Stonecutter => {
                    let display = s.inputs[0].is_some() && !s.stone.recipes.is_empty();
                    let active = display && s.stone.recipes.len() > 12;
                    let offs = if active { ui.scroll } else { 0.0 };
                    let scroller = if active {
                        "scroller"
                    } else {
                        "scroller_disabled"
                    };
                    batch.sprite(
                        atlas,
                        &format!("container/stonecutter/{scroller}"),
                        ox + 119.0,
                        oy + 15.0 + (41.0 * offs).floor(),
                    );
                    if display {
                        let (start, selected) = (ui.start, s.selected);
                        // graphics.item: the icon only, without the count.
                        let shown: Vec<(usize, ItemStack)> = (start
                            ..(start + 12).min(s.stone.recipes.len()))
                            .map(|i| {
                                (
                                    i,
                                    ItemStack {
                                        count: 1,
                                        ..self.station_recipes().stonecutting[s.stone.recipes[i]]
                                            .result
                                            .clone()
                                    },
                                )
                            })
                            .collect();
                        for (index, result) in shown {
                            let p = index - start;
                            let (x, y) = (
                                52.0 + (p % 4) as f32 * 16.0,
                                14.0 + (p / 4) as f32 * 18.0 + 2.0,
                            );
                            let hover = inside(local, x, y, 16.0, 18.0);
                            let sprite = if Some(index) == selected {
                                "recipe_selected"
                            } else if hover {
                                "recipe_highlighted"
                            } else {
                                "recipe"
                            };
                            batch.sprite_scaled(
                                &self.client.gui.atlas,
                                &format!("container/stonecutter/{sprite}"),
                                ox + x,
                                oy + y - 1.0,
                                16.0,
                                18.0,
                                u32::MAX,
                            );
                            self.draw_item(batch, &result, ox + x, oy + y);
                            if hover {
                                recipe_tooltip = Some(result);
                            }
                        }
                    }
                }
            }
        }
        if anvil_name {
            self.draw_anvil_name(batch, ox, oy);
        }
        let drag = self.drag_view();
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        for i in self.station_slots().unwrap_or_default() {
            let Some((sx, sy)) = self.slot_position_any(i) else {
                continue;
            };
            let stack = self.slot_mut(i).clone();
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        let slot_tooltip = if tooltip.is_none() {
            self.slot_at(batch, self.client.cursor.0, self.client.cursor.1)
                .and_then(|slot| self.slot_mut(slot).clone())
        } else {
            None
        };
        let browser_tip = self.draw_mod_browser(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if self.common.carried.is_some() {
            self.draw_carried(batch);
        }
        // StonecutterScreen.extractTooltip offers the recipe after the inherited slot tooltip.
        // Only the slot is subject to AbstractContainerScreen's carried-item check.
        if let Some(stack) = slot_tooltip
            .filter(|s| self.tooltip_with_carried(s))
            .or(recipe_tooltip)
        {
            self.draw_stack_tooltip(batch, &stack, self.client.cursor.0, self.client.cursor.1);
        } else if self.common.carried.is_none()
            && let Some(lines) = tooltip
        {
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        }
        if let Some(tooltip) = browser_tip {
            self.draw_mod_browser_tooltip(batch, tooltip);
        }
        true
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
