//! `render_name_tag_do_render` (#1164): asked per line of a name tag, score line first. With no
//! listener the tags draw byte for byte as before; a listener that changes nothing leaves them so;
//! a cancel drops only its line, the name keeping its lift; what a listener draws is kept.
use crate::game::{ClientGame, native_entity_refs::tests::with_client_game};
use mod_api::{Hooks, NameTagLine, RenderNameTagDoRenderEvent};
use std::sync::{Arc, Mutex};
use sunrise_client::mesher::Vertex;

const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// What the listener heard, and what it does: the line it cancels and whether it draws a quad.
#[derive(Default)]
struct Script {
    heard: Vec<(u32, String, NameTagLine, String, [f64; 3])>,
    cancel: Option<NameTagLine>,
    draw: bool,
}

struct Listener(Arc<Mutex<Script>>);
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn render_name_tag_do_render(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderNameTagDoRenderEvent<'_>,
    ) -> mod_api::Flow {
        let mut script = self.0.lock().unwrap();
        script.heard.push((
            event.entity_id,
            event.kind.to_string(),
            event.line,
            event.content.to_string(),
            event.attachment,
        ));
        if script.draw && event.line == NameTagLine::Name {
            let corners = [
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
            ];
            event
                .draw
                .quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
        }
        event.cancelled = script.cancel == Some(event.line);
        mod_api::Flow::Continue
    }
}

/// The same mod with no `render_name_tag_do_render` listener: the baseline game loads exactly
/// what the listening one does, so only the subscription differs.
struct Silent;
#[mod_api::__subscriptions]
impl Hooks for Silent {
    fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {}
}

/// A mannequin named "Bob" in view, whose tag has two lines: its description (the score line)
/// and its name.
fn add_bob(game: &mut ClientGame<'_>) {
    game.mobs.clear();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:mannequin")
        .unwrap()
        .clone();
    let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut bob = sunrise_client::mobs::Mob::new(77, kind, at, 0.0);
    bob.custom_name = Some("Bob".into());
    bob.name_visible = true;
    game.mobs.push(bob);
}

/// One frame's name tags: the depth-tested vertices and the see-through ones, as bytes.
fn tags(game: &mut ClientGame<'_>) -> (Vec<u8>, Vec<u8>) {
    game.prepare_entity_culling();
    game.mob_geometry(CAMERA, 1.0, &mut Vec::new());
    let (mut out, mut see_through) = (Vec::<Vertex>::new(), Vec::<Vertex>::new());
    let mut collector = std::mem::take(&mut game.client.see_through_collector);
    collector.clear();
    game.name_tag_geometry(CAMERA, 1.0, &mut out, &mut collector);
    collector.flush(&mut see_through);
    game.client.see_through_collector = collector;
    let bytes = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).to_vec();
    (bytes(&out), bytes(&see_through))
}

#[test]
#[ignore = "daily"]
fn name_tag_lines_are_asked_in_order_and_draw_as_before_unless_canceled() {
    let mut vanilla = None;
    let silent = vec![mod_api::ModEntry::new("tags", Box::new(Silent))];
    mod_platform::testing::with_mods(silent.into(), || {
        with_client_game(|game| {
            add_bob(game);
            vanilla = Some(tags(game));
        })
    });
    let (out, see_through) = vanilla.unwrap();
    assert!(!out.is_empty() && !see_through.is_empty());
    let size = std::mem::size_of::<Vertex>();
    let script = Arc::new(Mutex::new(Script::default()));
    let mods = vec![mod_api::ModEntry::new(
        "tags",
        Box::new(Listener(script.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            add_bob(game);
            assert_eq!(
                tags(game),
                (out.clone(), see_through.clone()),
                "a listener that only reads"
            );
            let heard = std::mem::take(&mut script.lock().unwrap().heard);
            let lines: Vec<_> = heard.iter().map(|h| (h.0, h.1.as_str(), h.2)).collect();
            assert_eq!(
                lines,
                [
                    (77, "minecraft:mannequin", NameTagLine::Score),
                    (77, "minecraft:mannequin", NameTagLine::Name)
                ]
            );
            assert!(!heard[0].3.is_empty());
            assert_eq!(heard[1].3, "Bob");
            assert_eq!(heard[0].4, heard[1].4);
            assert!(
                heard[0].4[1] > 1.0,
                "the attachment is the mannequin's height"
            );

            // The score line canceled: the name alone, where it was.
            script.lock().unwrap().cancel = Some(NameTagLine::Score);
            let (name, name_see_through) = tags(game);
            assert!(!name.is_empty() && name.len() < out.len());
            assert!(out.ends_with(&name), "the name keeps its lift");
            assert!(see_through.ends_with(&name_see_through));

            // The name line canceled and a quad drawn in its place: the score line and the quad.
            {
                let mut script = script.lock().unwrap();
                script.cancel = Some(NameTagLine::Name);
                script.draw = true;
            }
            let (drawn, _) = tags(game);
            assert_eq!(drawn.len(), out.len() - name.len() + 4 * size);
        })
    });
}
