//! One full-context scissor scope, restricting its range of HUD draws.
use mod_api::GuiClipRefusal;
use sunrise_client_render::hud::HudBatch;

/// Where a clip began and its rectangle (`[x0, y0, x1, y1]`), intersected with its parent.
/// The scoped draw context owns the clip along with the borrowed batch, and closes it on drop.
pub struct GuiClip {
    start: usize,
    rect: [f32; 4],
}

impl GuiClip {
    pub fn open(
        batch: &HudBatch,
        rect: [f32; 4],
        outer: Option<&Self>,
    ) -> Result<Self, GuiClipRefusal> {
        if !rect.iter().all(|v| v.is_finite()) {
            return Err(GuiClipRefusal::NotFinite);
        }
        let [x, y, w, h] = rect;
        let (x1, y1) = (x + w.max(0.0), y + h.max(0.0));
        if ![x1, y1].iter().all(|v| v.is_finite()) {
            return Err(GuiClipRefusal::NotFinite);
        }
        let [l, t, r, b] = outer.map_or([0.0, 0.0, batch.width, batch.height], |c| c.rect);
        let [left, top] = [x.max(l), y.max(t)];
        let clip = [left, top, x1.min(r).max(left), y1.min(b).max(top)];
        Ok(Self {
            start: batch.vertices.len(),
            rect: clip,
        })
    }

    pub fn rect(&self) -> [f32; 4] {
        self.rect
    }

    pub fn close(self, batch: &mut HudBatch) {
        let [x0, y0, x1, y1] = self.rect;
        batch.clip_since(self.start, x0, y0, x1, y1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(batch: &mut HudBatch, x: f32, y: f32, size: f32) {
        let p = [[x, y], [x + size, y], [x + size, y + size], [x, y + size]];
        batch.quad(
            p,
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            u32::MAX,
        );
    }

    #[test]
    fn a_clip_cuts_only_what_was_drawn_inside_it() {
        let mut batch = HudBatch::scratch([640, 480]);
        square(&mut batch, 0.0, 0.0, 300.0);
        let before: Vec<_> = batch.vertices.iter().map(|v| v.pos).collect();
        let clip = GuiClip::open(&batch, [10.0, 10.0, 100.0, 100.0], None).unwrap();
        square(&mut batch, 0.0, 0.0, 300.0);
        clip.close(&mut batch);
        let kept: Vec<_> = batch.vertices[..before.len()]
            .iter()
            .map(|v| v.pos)
            .collect();
        assert_eq!(kept, before, "drawn before the clip");
        assert_eq!(
            batch.vertices[6..]
                .iter()
                .map(|v| v.pos)
                .collect::<Vec<_>>(),
            before
        );
        assert_eq!(
            batch.scissors_in(0..12).collect::<Vec<_>>(),
            [(0..6, None), (6..12, Some([10.0, 10.0, 110.0, 110.0])),]
        );
    }

    #[test]
    fn nested_clips_intersect_and_a_disjoint_one_clips_everything() {
        for (outer, inner, expected) in [
            (
                [0.0, 0.0, 100.0, 100.0],
                [50.0, 50.0, 200.0, 200.0],
                Some([50.0, 50.0, 100.0, 100.0]),
            ),
            ([0.0, 0.0, 10.0, 10.0], [20.0, 20.0, 10.0, 10.0], None),
            ([0.0, 0.0, 10.0, 10.0], [0.0, 0.0, -5.0, 10.0], None),
        ] {
            let mut batch = HudBatch::scratch([640, 480]);
            let outer = GuiClip::open(&batch, outer, None).unwrap();
            let inner = GuiClip::open(&batch, inner, Some(&outer)).unwrap();
            square(&mut batch, 0.0, 0.0, 300.0);
            inner.close(&mut batch);
            outer.close(&mut batch);
            let clip = batch.scissors_in(0..6).next().unwrap().1.unwrap();
            let [l, t, r, b] = clip;
            assert_eq!((r > l && b > t).then_some(clip), expected);
        }
    }

    #[test]
    fn nonfinite_scissors_leave_the_enclosing_clip_unchanged() {
        let batch = HudBatch::scratch([640, 480]);
        let rect = [0.0, 0.0, 10.0, 10.0];
        let outer = GuiClip::open(&batch, rect, None).unwrap();
        for rect in [
            [f32::NAN, 0.0, 1.0, 1.0],
            [0.0, 0.0, f32::INFINITY, 1.0],
            [0.0, 0.0, f32::NAN, 1.0],
            [0.0, 0.0, f32::NEG_INFINITY, 1.0],
            [0.0, 0.0, 1.0, f32::NAN],
            [0.0, 0.0, 1.0, f32::NEG_INFINITY],
            [f32::MAX, 0.0, f32::MAX, 1.0],
        ] {
            assert_eq!(
                GuiClip::open(&batch, rect, Some(&outer)).err(),
                Some(GuiClipRefusal::NotFinite)
            );
        }
        assert_eq!(
            GuiClip::open(&batch, rect, Some(&outer)).unwrap().rect,
            [0.0, 0.0, 10.0, 10.0]
        );
    }
}
