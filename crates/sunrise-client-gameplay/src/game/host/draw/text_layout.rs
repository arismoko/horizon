//! Position wrapped visual lines once for both host reads and GUI paint.
use mod_api::{TextAlign, TextLayoutError, TextLayoutSize, TextLayoutSpec, TextLine};
use std::cell::Cell;
use sunrise_client::{
    bidi,
    font::{DEFAULT, Font},
};
use sunrise_game_data::text::{Text, TextSpanScratch};

/// `Font.lineHeight`, in unscaled GUI pixels.
const LINE_HEIGHT: f32 = 9.0;

pub struct LabelLayout {
    wrap_width: f32,
    align: TextAlign,
    scale: f32,
}

impl TryFrom<TextLayoutSpec> for LabelLayout {
    type Error = TextLayoutError;

    fn try_from(spec: TextLayoutSpec) -> Result<Self, Self::Error> {
        if !spec.scale.is_finite() || spec.scale <= 0.0 {
            return Err(TextLayoutError::ScaleInvalid);
        }
        let width = spec.max_width / spec.scale;
        if !width.is_finite() {
            return Err(TextLayoutError::WidthNotFinite);
        }
        Ok(Self {
            // StringSplitter.splitLines wraps at a minimum width of one pixel.
            wrap_width: width.max(1.0),
            align: spec.align,
            scale: spec.scale,
        })
    }
}

thread_local! {
    static LABEL_TEXT_SCRATCH: Cell<TextSpanScratch> = const { Cell::new(TextSpanScratch::new()) };
}

fn with_line_scratch<T>(visit: impl FnOnce(&mut TextSpanScratch) -> T) -> T {
    LABEL_TEXT_SCRATCH.with(|scratch| {
        // A line visitor may ask for another layout. Take the buffer rather than holding a
        // mutable borrow across that callback; each overlapping layout owns its own scratch.
        let mut buffer = scratch.take();
        let result = visit(&mut buffer);
        scratch.set(buffer);
        result
    })
}

struct Placement {
    local: [f32; 2],
    scaled: [f32; 2],
    width: f32,
}

impl LabelLayout {
    pub fn visit_components(
        &self,
        font: &Font,
        text: &mod_api::Text,
        visit: &mut dyn FnMut(TextLine<'_>),
    ) -> Result<TextLayoutSize, TextLayoutError> {
        with_line_scratch(|scratch| {
            let local = sunrise_sim::api_text::to_local_engine(text);
            let mut size = TextLayoutSize::default();
            let mut placed = Vec::new();
            // Check all the geometry before giving the visitor any line. Keep only borrowed
            // ranges and placements; the logical text lends the same scratch on each call.
            for (i, line) in font
                .formatted_lines_with_font(local.engine(), self.wrap_width, DEFAULT)
                .enumerate()
            {
                let width = scratch
                    .with_resolved(line.ranges(), |text| font.width_visual(&bidi::visual(text)));
                placed.push((line, self.place(i, width, &mut size)?));
            }
            for (line, at) in placed {
                scratch.with_resolved(line.ranges(), |logical| {
                    visit(TextLine::new(
                        &local.text_to_api(logical),
                        at.scaled,
                        at.width,
                    ));
                });
            }
            Ok(size)
        })
    }

    /// Paint visual lines at their unscaled positions. A paint caller rolls back its batch
    /// if a later line's geometry is refused; no owned component or replay list is returned.
    pub fn visit_visual(
        &self,
        font: &Font,
        text: &Text,
        mut visit: impl FnMut(&Text, [f32; 2]),
    ) -> Result<TextLayoutSize, TextLayoutError> {
        with_line_scratch(|scratch| {
            let mut size = TextLayoutSize::default();
            for (i, line) in font
                .formatted_lines_with_font(text, self.wrap_width, DEFAULT)
                .enumerate()
            {
                scratch.with_resolved(line.ranges(), |line| {
                    let visual = bidi::visual(line);
                    let at = self.place(i, font.width_visual(&visual), &mut size)?;
                    visit(&visual, at.local);
                    Ok::<_, TextLayoutError>(())
                })?;
            }
            Ok(size)
        })
    }

    fn place(
        &self,
        line: usize,
        width: f32,
        size: &mut TextLayoutSize,
    ) -> Result<Placement, TextLayoutError> {
        let x = match self.align {
            TextAlign::Left => 0.0,
            // TextAlignment.CENTER.calculateLeft floors half of the line's width.
            TextAlign::Centre => self.wrap_width / 2.0 - (width / 2.0).floor(),
            TextAlign::Right => self.wrap_width - width,
        };
        let y = line as f32 * LINE_HEIGHT;
        let [scaled_x, scaled_y, scaled_width] = [x, y, width].map(|v| v * self.scale);
        let height = scaled_y + LINE_HEIGHT * self.scale;
        if ![scaled_x, scaled_y, scaled_width, height]
            .into_iter()
            .all(f32::is_finite)
        {
            return Err(TextLayoutError::GeometryNotFinite);
        }
        size.width = size.width.max(scaled_width);
        size.height = height;
        Ok(Placement {
            local: [x, y],
            scaled: [scaled_x, scaled_y],
            width: scaled_width,
        })
    }
}
