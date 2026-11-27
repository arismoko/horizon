use super::*;
use crate::game::host::draw::GameDrawContext;
use mod_api::{__private::game::CallbackOwner, ClientHost, DrawContext, GuiLayerWrapContext};
use sunrise_client::gui_layers::{self, GuiLayerBody, VanillaGuiLayer};

#[allow(dead_code)]
impl crate::game::ClientGame<'_> {
    /// Applies a client mod's `clear_particles` if one is waiting. Called at the start of
    /// each particle tick, after `RenderFrameEvent.Pre`, and at particle extraction, after
    /// HUD, screen and entity callbacks. Extraction also discards its geometry if a model
    /// callback asks to clear while it is being built, even while paused or tick-frozen.
    pub(crate) fn apply_mod_particle_clear(&mut self) {
        if self.client.mod_clear_particles.take() {
            self.clear_client_particles();
        }
    }
    /// `ParticleEngine.clearParticles` over every group Sunrise keeps apart: the engine's
    /// particles, starters, queue and emitters, the item pickups and the elder guardian
    /// apparition.
    pub(in crate::game) fn clear_client_particles(&mut self) {
        self.client.mod_clear_particles.set(false);
        self.client.particles.0.clear();
        self.client.pickups.0.clear();
    }
    /// Replacing or closing the client level also discards its `ClientExplosionTracker` work.
    pub(in crate::game) fn clear_client_level_particles(&mut self) {
        self.client.particles.0.discard_level_explosions();
        self.clear_client_particles();
    }
    /// `ParticleEngine.countParticles`: every live group. Sunrise keeps vanilla's groups
    /// apart: ordinary particles, firework starters and elder guardians in the engine,
    /// and ITEM_PICKUP in `pickups`.
    pub fn client_particle_count(&self) -> usize {
        self.client.particles.0.group_count() + self.client.pickups.0.live.len()
    }
    /// A mod's own screen closes (Escape, or `mod_screen_close`): to the Mods screen when
    /// its Config opened it (the parent screen NeoForge's `IConfigScreenFactory` returns to).
    pub fn close_mod_screen(&mut self) {
        if self.client.screen == Screen::Mod {
            if self.client.native_mod_screen_parent {
                self.set_screen(Screen::Options);
            } else if self.client.title.0.is_some() {
                self.close_mod_screen_to_title();
            } else {
                self.set_screen(Screen::Playing);
                self.capture(true);
            }
            if self.client.screen == Screen::Mod {
                // `screen_opening` kept it open, or opened a mod's own.
                return;
            }
        }
        self.client.native_mod_screen = None;
    }
    /// `Screen.onClose`'s `setScreen(null)` with no world: the title screen
    /// opens, asked for once as `mod` to `title`, and a mod's answer applies
    /// as on any title screen. Told to the frame's observer, so it doesn't
    /// announce it again.
    pub(crate) fn close_mod_screen_to_title(&mut self) {
        let decision = if self.screen_hooks_heard() {
            let current = self.screen_change_view(Screen::Mod);
            let none = self.screen_change_view(Screen::Playing);
            self.decide_screen(&current, &screen_view_with_id(&none, "title"), true, false)
        } else {
            ScreenDecision::Open
        };
        match decision {
            ScreenDecision::Keep => (),
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.client.title.0.as_mut() {
                    title.open_by_id("title");
                }
                self.apply_screen(Screen::Playing, None);
            }
        }
    }
    /// Whether a box in `corner` shows: F3's debug screen takes both top
    /// corners (its left and right columns), and a bottom corner while one
    /// of its charts is drawn there.
    pub(crate) fn native_corner_shown(&self, corner: Corner) -> bool {
        let charts = self
            .client
            .debug_charts
            .corners(!self.has_singleplayer_server());
        !debug_takes(corner, self.client.debug, charts)
    }
}

impl<'g> crate::game::ClientGame<'g> {
    pub(in crate::game) fn draw_gui_layer_body(
        &mut self,
        batch: &mut HudBatch,
        body: &GuiLayerBody,
        vanilla: Option<&mut VanillaGuiDraw<'_, 'g>>,
    ) {
        match body {
            GuiLayerBody::Vanilla => {
                // Only a vanilla slot can contain this node; its paint site lends the closure.
                // A wrapper that set the heights before `draw_previous` moves this row.
                self.apply_gui_status_heights();
                if let Some(draw) = vanilla {
                    CallbackOwner::call_engine(|| draw(self, batch));
                }
            }
            GuiLayerBody::Native { owner, draw } => {
                if let Some(owner) = owner.as_ref().filter(|owner| owner.is_enabled()) {
                    let _ = owner.call("HUD layer", "gui_layer_draw", || {
                        draw(self, &mut GameDrawContext::new(self, batch));
                    });
                }
            }
            GuiLayerBody::Wrapped {
                owner,
                wrap,
                previous,
            } => {
                let Some(owner) = owner.as_ref().filter(|owner| owner.is_enabled()) else {
                    self.draw_gui_layer_body(batch, previous, vanilla);
                    return;
                };
                let _ = owner.call("HUD layer", "gui_layer_wrapper", || {
                    wrap(&mut GuiLayerWrapFrame {
                        game: self,
                        batch,
                        previous,
                        vanilla,
                    });
                });
            }
        }
    }
    /// Draws inserted layers not drawn yet this frame, below `until` or to the end with `None`.
    /// A vanilla slot keeps its place even if its original body emits no paint.
    /// Inserted layers do not draw outside the layer walk.
    pub fn draw_mod_gui_layers(&mut self, until: Option<VanillaGuiLayer>, batch: &mut HudBatch) {
        let Some(next) = self.client.hud_layer_next else {
            return;
        };
        let Some(order) = gui_layers::order() else {
            return;
        };
        let slots = order.slots();
        let end = until.map_or(slots.len(), |layer| order.position(layer));
        // An earlier anchor can still paint later (e.g. deferred subtitles). Don't redraw slots.
        for slot in &slots[next.min(end)..end] {
            if slot.vanilla_anchor().is_none() {
                self.run_gui_layer(slot.id(), batch, &slot.body, None, false);
            }
        }
        // Past `until` itself: its paint site draws it, not the cursor.
        self.client.hud_layer_next = Some(next.max(end + usize::from(until.is_some())));
    }
    /// Dispatches a vanilla slot after flushing inserted layers below it. Replacing or wrapping
    /// a body leaves the slot's ID, order and hooks unchanged. The original body's eligibility
    /// applies only to the original paint, including when a wrapper delegates to it.
    pub fn gui_layer(
        &mut self,
        layer: VanillaGuiLayer,
        batch: &mut HudBatch,
        draw: &mut dyn FnMut(&mut crate::game::ClientGame<'g>, &mut HudBatch),
    ) {
        self.draw_mod_gui_layers(Some(layer), batch);
        sunrise_client::spans::time(layer.span_name(), || {
            self.run_gui_layer(
                layer.id(),
                batch,
                gui_layers::order().map_or(&GuiLayerBody::Vanilla, |order| {
                    &order.slots()[order.position(layer)].body
                }),
                Some(draw),
                matches!(
                    layer,
                    VanillaGuiLayer::PlayerHealth
                        | VanillaGuiLayer::ArmorLevel
                        | VanillaGuiLayer::FoodLevel
                        | VanillaGuiLayer::VehicleHealth
                        | VanillaGuiLayer::AirLevel
                ),
            );
        });
    }
    pub(crate) fn run_gui_layer(
        &mut self,
        id: &'static str,
        batch: &mut HudBatch,
        body: &GuiLayerBody,
        vanilla: Option<&mut VanillaGuiDraw<'_, 'g>>,
        status_row: bool,
    ) {
        let mut canceled = false;
        self.native_gui_layer_pre(id, batch, &mut canceled);
        if canceled {
            return;
        }
        let start = batch.vertices.len();
        self.draw_gui_layer_body(batch, body, vanilla);
        self.apply_gui_status_heights();
        if status_row {
            self.keep_status_rows(&batch.vertices[start..], batch.width, batch.height);
        }
        if let Some(bounds) = batch.visible_vertex_bounds(start) {
            self.client.hud_layer_bounds.push((id, bounds));
        }
        self.native_gui_layer_post(id, batch);
        self.apply_gui_status_heights();
    }
}

// The vanilla paint procedure belongs to the caller's frame, not the frozen layer order.
#[cfg(feature = "client")]
pub(in crate::game) type VanillaGuiDraw<'p, 'g> =
    dyn FnMut(&mut crate::game::ClientGame<'g>, &mut HudBatch) + 'p;

#[cfg(feature = "client")]
pub(in crate::game) struct GuiLayerWrapFrame<'a, 'p, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ClientGame<'g>,
    pub(in crate::game) batch: &'a mut HudBatch,
    pub(in crate::game) previous: &'a GuiLayerBody,
    pub(in crate::game) vanilla: Option<&'a mut VanillaGuiDraw<'p, 'g>>,
}

#[cfg(feature = "client")]
impl GuiLayerWrapContext for GuiLayerWrapFrame<'_, '_, '_> {
    fn host(&self) -> &dyn ClientHost {
        &*self.game
    }

    fn with_draw(&mut self, draw: &mut dyn FnMut(&mut dyn DrawContext)) {
        draw(&mut GameDrawContext::new(self.game, self.batch));
    }

    fn draw_previous(&mut self) {
        self.game
            .draw_gui_layer_body(self.batch, self.previous, self.vanilla.as_deref_mut());
    }
}

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;
