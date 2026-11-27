//! Portable far-terrain column, span, tile-key, and zoom-aim data types.
pub mod bake;
pub mod cache;
use crate::columns::Surface;
use sunrise_world::registry::BlockStateId;

pub const CELLS: usize = 16;
/// How many resident chunks may wait for the worker's summary at once
/// (`Lod::observing`).
pub const OBSERVE_AHEAD: usize = 8;

/// Overworld `DimensionType.MIN_Y` used until `chunk.rs` lands in `sunrise-level` (M4).
pub const DEFAULT_MIN_Y: i32 = -64;

/// Far decoration's threads (`vanillagen::lod`): a fifth of the cores. They
/// run at idle priority, but on the same cores as the frame and the near
/// chunks all the same.
pub fn decoration_threads(cores: usize) -> usize {
    (cores / 5).clamp(1, 12)
}

/// The most a tile a zoomed view magnifies may split past the plan it would
/// have had ([`ZoomAim`]): cells three times smaller, so nine times the tiles
/// where it applies. A spyglass covers about a sixteenth of the circle, so at
/// the 4078 tiles and ~1500 quads a tile measured at `lodDistance` 256 this
/// asks for roughly 45% more far tiles while the zoom is held, and gives them
/// back when it is released. Four would be about 85%: raise it only with a
/// measurement, because the cost grows with the square.
pub const MAX_ZOOM_BOOST: i32 = 3;
/// Yaw is rounded to this many degrees before it reaches the plan, so turning
/// the mouse does not rebuild the quadtree every frame. The cone is widened by
/// as much, so rounding never swings it off a tile being looked at.
pub const ZOOM_YAW_STEP: f32 = 5.0;

/// Where a zoomed view asks for finer far tiles (the `lodZoomDetail` extra).
///
/// `Plan::visit`'s falloff keeps a tile's cells roughly the same size on
/// screen, which silently assumes the field of view never changes. A spyglass
/// (`SpyglassItem`, a tenth of it) or the zoom mod breaks that: the same tile
/// covers many times more pixels, so its cells look as coarse as a tile ten
/// times nearer would. This asks the tiles the zoom actually magnifies to
/// split further.
///
/// Only those: lifting the whole circle would ask for about a hundred times
/// the tiles, which at the quads measured above is gigabytes of vertices. The
/// cone is the narrowed view's, and `MAX_ZOOM_BOOST` caps how far it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoomAim {
    /// The view direction in x/z, in thousandths of a unit vector.
    dir: [i32; 2],
    /// The cosine of the cone's half-angle, in thousandths.
    cos_half: i32,
    /// How many times further than the unzoomed plan a tile inside the cone
    /// may split. Two or more; [`ZoomAim::new`] gives `None` for less.
    boost: i32,
}

impl ZoomAim {
    /// `fov_degrees` is the zoomed field of view and `magnification` how many
    /// times the view is magnified (1 when it is not). `None` when the zoom is
    /// too slight to be worth replanning for.
    pub fn new(yaw: f32, fov_degrees: f32, magnification: f32) -> Option<Self> {
        if !magnification.is_finite() || !fov_degrees.is_finite() || fov_degrees <= 0.0 {
            return None;
        }
        let boost = (magnification.round() as i64).clamp(1, i64::from(MAX_ZOOM_BOOST)) as i32;
        if boost < 2 {
            return None;
        }
        // Half the field of view spans the screen's width; three quarters of
        // it reaches the corners, and the rounding step is added on top.
        let half = (fov_degrees * 0.75 + ZOOM_YAW_STEP).clamp(5.0, 90.0);
        let radians = ((yaw / ZOOM_YAW_STEP).round() * ZOOM_YAW_STEP).to_radians();
        // `view_vector`'s x and z: yaw 0 looks toward +z.
        let thousandths = |v: f32| (v * 1000.0).round() as i32;
        Some(Self {
            dir: [thousandths(-radians.sin()), thousandths(radians.cos())],
            cos_half: thousandths(half.to_radians().cos()),
            boost,
        })
    }
    /// Whether the cone reaches `key`, so the zoom magnifies it.
    ///
    /// The tile is taken as the circle around its centre that covers it, and
    /// that circle's own angular radius widens the cone for the test, so a
    /// tile straddling the edge counts in rather than out. A tile whose circle
    /// reaches the camera is always in: there is no direction to compare, and
    /// it fills the view whichever way the player faces.
    pub fn boost(self) -> i32 {
        self.boost
    }
    pub fn covers(&self, key: TileKey, camera: [i32; 2]) -> bool {
        let [ox, _, oz] = key.origin();
        let size = key.size();
        // The camera stands in the tile: it is in view whichever way the
        // player faces, and there is no direction to compare it with. Tested
        // on the box rather than on the circle below, whose radius a camera
        // on a corner matches exactly -- a float coin-toss.
        // In i64, so a tile at the world's edge never overflows.
        let (ox64, oz64, size64) = (i64::from(ox), i64::from(oz), i64::from(size));
        if (ox64..ox64 + size64).contains(&i64::from(camera[0]))
            && (oz64..oz64 + size64).contains(&i64::from(camera[1]))
        {
            return true;
        }
        let half = f64::from(size) / 2.0;
        let cx = f64::from(ox) + half - f64::from(camera[0]);
        let cz = f64::from(oz) + half - f64::from(camera[1]);
        let distance = (cx * cx + cz * cz).sqrt();
        let radius = half * std::f64::consts::SQRT_2;
        if distance <= radius {
            return true;
        }
        let dir = [
            f64::from(self.dir[0]) / 1000.0,
            f64::from(self.dir[1]) / 1000.0,
        ];
        let cos = (cx * dir[0] + cz * dir[1]) / distance;
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        // cos(angle - tile radius), expanded so no inverse trigonometry runs
        // per tile: the quadtree asks this of every node it visits.
        let radius_sin = (radius / distance).min(1.0);
        let radius_cos = (1.0 - radius_sin * radius_sin).max(0.0).sqrt();
        cos * radius_cos + sin * radius_sin >= f64::from(self.cos_half) / 1000.0
    }
}

/// The coarsest tile level: `Plan` roots cover at most `MAX_CHUNKS` chunks
/// around (a level-10 tile is 16,384 blocks wide).
pub const MAX_LEVEL: u8 = 10;
/// The farthest `Plan` reaches, in chunks.
pub const MAX_CHUNKS: u32 = 1 << MAX_LEVEL;

/// A half-open material run; gaps between runs remain air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub min: i16,
    pub max: i16,
    pub state: BlockStateId,
    pub biome: u16,
    pub light: u16,
}
/// A column's material runs. Almost every far column has one to four, so
/// they live inline: the sampler builds millions of columns and a heap
/// allocation each dominated its memory and allocator time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spans(smallvec::SmallVec<[Span; 4]>);
impl Spans {
    pub const fn new() -> Self {
        Self(smallvec::SmallVec::new_const())
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self(smallvec::SmallVec::with_capacity(capacity))
    }
    pub fn push(&mut self, span: Span) {
        self.0.push(span);
    }
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
    pub fn as_slice(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::Deref for Spans {
    type Target = [Span];
    fn deref(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::DerefMut for Spans {
    fn deref_mut(&mut self) -> &mut [Span] {
        &mut self.0
    }
}
impl From<Vec<Span>> for Spans {
    fn from(spans: Vec<Span>) -> Self {
        Self(spans.into())
    }
}
impl FromIterator<Span> for Spans {
    fn from_iter<I: IntoIterator<Item = Span>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}
impl IntoIterator for Spans {
    type Item = Span;
    type IntoIter = smallvec::IntoIter<[Span; 4]>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Spans {
    type Item = &'a Span;
    type IntoIter = std::slice::Iter<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<'a> IntoIterator for &'a mut Spans {
    type Item = &'a mut Span;
    type IntoIter = std::slice::IterMut<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}
impl<const N: usize> PartialEq<[Span; N]> for Spans {
    fn eq(&self, other: &[Span; N]) -> bool {
        self.as_slice() == other
    }
}
impl PartialEq<Vec<Span>> for Spans {
    fn eq(&self, other: &Vec<Span>) -> bool {
        self.as_slice() == other.as_slice()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub surface: Option<Surface>,
    pub spans: Spans,
}
impl Column {
    /// Reduce an entire horizontal footprint, rather than inflating its corner
    /// column. A height is drawn where any column of the footprint holds a
    /// block there, in the material most of them hold: anything joined to
    /// the ground stays joined at every width. Drawing only heights most of
    /// the footprint held dropped trunks (one column of 16 or 256) under the
    /// crowns they hold up, and the crowns hung in the air as slabs.
    pub fn reduced(columns: &[Column; 256], x: usize, z: usize, width: usize) -> Self {
        if width == 1 {
            return columns[z * 16 + x].clone();
        }
        let mut events = Vec::new();
        for dz in z..z + width {
            for dx in x..x + width {
                for span in &columns[dz * 16 + dx].spans {
                    let material = (span.state, span.biome, span.light);
                    events.push((span.min, material, 1));
                    events.push((span.max, material, -1));
                }
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut result = Self::default();
        let mut votes = Vec::<((BlockStateId, u16), i32)>::new();
        let mut sky = [0i32; 16];
        let mut block = [0i32; 16];
        let mut occupied = 0;
        let mut bottom = DEFAULT_MIN_Y as i16;
        for (y, material, change) in events {
            if bottom < y {
                if occupied > 0
                    && let Some(&((state, biome), _)) =
                        votes.iter().max_by_key(|&(material, count)| {
                            (*count, std::cmp::Reverse((material.0.0, material.1)))
                        })
                {
                    // Lighting must not split votes for the same material.
                    // Reduction can expose buried faces; retain the brightest
                    // actual light in this occupied footprint, per channel.
                    let light = (sky.iter().rposition(|&n| n > 0).unwrap_or(0) << 4)
                        | (block.iter().rposition(|&n| n > 0).unwrap_or(0) << 12);
                    result.push(Span {
                        min: bottom,
                        max: y,
                        state,
                        biome,
                        light: light as u16,
                    });
                }
                bottom = y;
            }
            occupied += change;
            sky[usize::from((material.2 >> 4) & 15)] += change;
            block[usize::from((material.2 >> 12) & 15)] += change;
            let material = (material.0, material.1);
            if let Some(vote) = votes.iter_mut().find(|vote| vote.0 == material) {
                vote.1 += change;
            } else {
                votes.push((material, change));
            }
        }
        result
    }
    pub fn from_surface(surface: Option<Surface>) -> Self {
        let mut column = Self {
            surface,
            spans: Spans::new(),
        };
        if let Some(s) = surface {
            if s.height > DEFAULT_MIN_Y {
                column.spans.push(Span {
                    min: DEFAULT_MIN_Y as i16,
                    max: s.height as i16,
                    state: s.state,
                    biome: s.biome,
                    light: 240,
                });
            }
            if let Some((height, state)) = s.water {
                column.spans.push(Span {
                    min: s.height as i16,
                    max: height as i16,
                    state,
                    biome: s.biome,
                    light: 240,
                });
            }
        }
        column
    }
    pub fn push(&mut self, span: Span) {
        if let Some(last) = self.spans.last_mut()
            && last.max == span.min
            && last.state == span.state
            && last.biome == span.biome
            && last.light == span.light
        {
            last.max = span.max;
        } else {
            self.spans.push(span);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub level: u8,
}
impl TileKey {
    pub fn size(self) -> i32 {
        16 << self.level
    }
    pub fn origin(self) -> [i32; 3] {
        [self.x * self.size(), 0, self.z * self.size()]
    }
    pub fn parent(self) -> Self {
        Self {
            x: self.x.div_euclid(2),
            z: self.z.div_euclid(2),
            level: self.level + 1,
        }
    }
    pub fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            x: self.x * 2 + (i & 1) as i32,
            z: self.z * 2 + (i >> 1) as i32,
            level: self.level - 1,
        })
    }
    /// The squared horizontal distance from `camera` to the tile's nearest point.
    pub fn reach(self, camera: [i32; 2]) -> i64 {
        let [x, _, z] = self.origin();
        let gap = |c: i32, lo: i32| i64::from((lo - c).max(c - lo - self.size()).max(0));
        gap(camera[0], x).pow(2) + gap(camera[1], z).pow(2)
    }
    /// Whether the tile covers any of the blocks `[x0, z0, x1, z1)`.
    pub fn overlaps(self, [x0, z0, x1, z1]: [i32; 4]) -> bool {
        let [ox, _, oz] = self.origin();
        let size = self.size();
        ox < x1 && oz < z1 && ox + size > x0 && oz + size > z0
    }
    /// Whether new data for `area` drawn in cells `widths` wide changes the
    /// tile's samples: its cells are that wide, and it covers `area` or the
    /// blocks around it a cell's depth reads (`bluemap::SKIRT`: a block
    /// around a chunk, the next cells' middles past one).
    pub fn refreshed_by(self, area: [i32; 4], widths: (i32, i32)) -> bool {
        let cell = self.size() / CELLS as i32;
        (widths.0..=widths.1).contains(&cell) && self.overlaps(Self::around(area, cell))
    }
    pub fn around([x0, z0, x1, z1]: [i32; 4], cell: i32) -> [i32; 4] {
        let m = if cell <= 16 { 1 } else { cell };
        [x0 - m, z0 - m, x1 + m, z1 + m]
    }
    pub fn distance(self, camera: [i32; 2]) -> i32 {
        let [x, _, z] = self.origin();
        (x - camera[0])
            .max(camera[0] - x - self.size())
            .max(z - camera[1])
            .max(camera[1] - z - self.size())
            .max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_material_votes_ignore_light_and_retain_exposure() {
        let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
        for i in 0..16 {
            columns[(i / 4) * 16 + i % 4].spans.push(Span {
                min: 60,
                max: 64,
                state: BlockStateId(if i < 10 { 2 } else { 1 }),
                biome: 0,
                light: if i < 10 { (i as u16 + 6) << 4 } else { 0 },
            });
        }
        let coarse = Column::reduced(&columns, 0, 0, 4);
        assert_eq!(coarse.spans.len(), 1);
        assert_eq!(
            coarse.spans[0].state,
            BlockStateId(2),
            "majority grass must beat dark dirt"
        );
        assert_eq!(coarse.spans[0].light, 240, "retain sampled sky exposure");
        for column in &mut columns {
            for span in &mut column.spans {
                span.light = 0;
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            0,
            "unlit caves stay unlit"
        );
        columns[0].spans[0].light = 7 << 12;
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            7 << 12,
            "block light does not invent skylight"
        );
    }
}
