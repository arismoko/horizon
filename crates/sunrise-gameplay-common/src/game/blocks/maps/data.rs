use std::collections::HashMap;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_items::{blocks::MAP_COLORS, items::ItemStack};

/// What a decoration is filed under in `MapItemSavedData.decorations` (a
/// `LinkedHashMap<String, MapDecoration>`): a player's marker by their
/// connection, the banner at a position (`MapBanner.getId`), a framed map's
/// frame by its entity (`getFrameKey`).
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Player(u32),
    /// A named entry of an item's `minecraft:map_decorations`.
    Item(JavaString),
    /// A decoration a server sent (`addClientSideDecorations`).
    Remote,
    Banner([i32; 3]),
    Frame(i32),
}
/// A decoration (`MapDecoration`): sprite, map position (-128..127, half
/// pixels), rotation (0..15) and name, filed under its key.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    pub sprite: &'static str,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
    pub name: Option<String>,
    pub key: Key,
}

/// `MapBanner`: a banner a map marks, as `banners` saves it.
#[derive(Clone, Debug, PartialEq)]
pub struct MapBanner {
    pub pos: [i32; 3],
    /// `DyeColor.getName` of the banner's base colour.
    pub color: String,
    pub name: Option<String>,
}
impl MapBanner {
    /// `MapBanner.getDecoration`: the marker of the banner's colour.
    pub(in crate::game) fn sprite(&self) -> &'static str {
        decoration_sprite(&format!("banner_{}", self.color)).unwrap_or(BANNER_WHITE)
    }
    /// `MapBanner.CODEC`.
    pub(in crate::game) fn to_nbt(&self) -> Tag {
        let mut entries = vec![
            ("pos", Tag::IntArray(self.pos.to_vec())),
            ("color", Tag::String(self.color.as_str().into())),
        ];
        if let Some(name) = &self.name {
            entries.push(("name", Tag::String(name.as_str().into())));
        }
        Tag::compound(entries)
    }
    pub(in crate::game) fn from_nbt(tag: &Tag) -> Option<Self> {
        // DyeColor.CODEC is read leniently: white when missing or unknown.
        let color = sunrise_sim::save::text(tag.get("color"))
            .filter(|c| decoration_sprite(&format!("banner_{c}")).is_some())
            .unwrap_or_else(|| "white".to_owned());
        Some(Self {
            pos: block_pos(tag.get("pos"))?,
            color,
            name: sunrise_sim::save::text(tag.get("name")),
        })
    }
}
/// `MapFrame`: an item frame a map hangs in, as `frames` saves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapFrame {
    pub pos: [i32; 3],
    /// The frame's `Direction.get2DDataValue` times 90.
    pub rotation: i32,
    pub entity_id: i32,
}
impl MapFrame {
    /// `MapFrame.CODEC`.
    pub(in crate::game) fn to_nbt(self) -> Tag {
        Tag::compound([
            ("pos", Tag::IntArray(self.pos.to_vec())),
            ("rotation", Tag::Int(self.rotation)),
            ("entity_id", Tag::Int(self.entity_id)),
        ])
    }
    pub(in crate::game) fn from_nbt(tag: &Tag) -> Option<Self> {
        let int = |k: &str| sunrise_sim::save::number(tag.get(k)).map(|v| v as i32);
        Some(Self {
            pos: block_pos(tag.get("pos"))?,
            rotation: int("rotation")?,
            entity_id: int("entity_id")?,
        })
    }
}
/// `BlockPos.CODEC`: three ints, as an int array or a list.
pub(in crate::game) fn block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        Tag::List(l) if l.len() == 3 => {
            let int = |t: &Tag| match t {
                Tag::Int(v) => Some(*v),
                _ => None,
            };
            Some([int(&l[0])?, int(&l[1])?, int(&l[2])?])
        }
        _ => None,
    }
}
/// The item frame a map hangs in, as `tickCarriedBy` reads it
/// (`placedInFrame`): the block it is in, its `Direction.get3DDataValue`
/// (down 0, up 1, north 2, south 3, west 4, east 5) and its entity id.
#[derive(Clone, Copy)]
pub struct Framed {
    pub pos: [i32; 3],
    pub facing: u8,
    pub id: i32,
}
/// `Direction.get2DDataValue` of a 3D data value: south 0, west 1, north 2,
/// east 3, and -1 for up and down.
pub(in crate::game) fn direction_2d(facing: u8) -> i32 {
    match facing {
        3 => 0,
        4 => 1,
        2 => 2,
        5 => 3,
        _ => -1,
    }
}
/// `MapDecorationTypes` the markers use: the player's arrow, the arrows that
/// replace it past the map's edge, the item frame's, the white banner's (the
/// fallback colour).
pub const PLAYER: &str = "map/decorations/player";
pub(in crate::game) const FRAME: &str = "map/decorations/frame";
pub(in crate::game) const BANNER_WHITE: &str = "map/decorations/white_banner";
pub(in crate::game) const PLAYER_OFF_MAP: &str = "map/decorations/player_off_map";
pub(in crate::game) const PLAYER_OFF_LIMITS: &str = "map/decorations/player_off_limits";
/// A player carrying a map, or seeing it framed
/// (`MapItemSavedData.HoldingPlayer`): the column sweep's step, and what
/// changed since the last `ClientboundMapItemDataPacket` it was sent.
pub struct Holder {
    pub(in crate::game) who: u32,
    /// The last tick it carried the map, and saw it framed.
    pub carried: i64,
    pub(in crate::game) framed: i64,
    pub step: i32,
    /// The changed pixels' box (min x, min y, max x, max y).
    pub(in crate::game) dirty: Option<[i32; 4]>,
    pub(in crate::game) decorations: bool,
    pub(in crate::game) tick: u32,
    /// Wore map invisibility gear on its last tick
    /// (`MapItemSavedData.hasMapInvisibilityItemEquipped`).
    pub invisible: bool,
}
impl Holder {
    pub(in crate::game) fn new(who: u32) -> Self {
        Self {
            who,
            carried: 0,
            framed: 0,
            step: 0,
            dirty: Some([0, 0, 127, 127]),
            decorations: true,
            tick: 0,
            invisible: false,
        }
    }
    /// Still carrying the map, or seeing it framed (frames tick every 10).
    pub(in crate::game) fn holds(&self, now: i64) -> bool {
        self.carried + 1 >= now || self.framed + 10 >= now
    }
}
pub struct MapData {
    pub center: [i32; 2],
    pub scale: u8,
    pub dimension: String,
    pub locked: bool,
    pub tracking: bool,
    pub unlimited: bool,
    pub colors: Vec<u8>,
    pub decorations: Vec<Decoration>,
    /// `bannerMarkers` and `frameMarkers` (`banners`, `frames` when saved).
    pub banners: Vec<MapBanner>,
    pub frames: Vec<MapFrame>,
    pub(in crate::game) holders: Vec<Holder>,
    /// Changed since the last save / texture upload.
    pub dirty: bool,
    pub texture_dirty: bool,
}
impl MapData {
    /// `MapItemSavedData.createFresh`: centred on the scale's grid cell.
    pub fn fresh(x: f64, z: f64, scale: u8, dimension: &str) -> Self {
        let size = 128 * (1 << scale);
        let cell = |v: f64| (v.floor() as i32 + 64).div_euclid(size) * size + size / 2 - 64;
        Self {
            center: [cell(x), cell(z)],
            scale,
            dimension: dimension.to_owned(),
            locked: false,
            tracking: true,
            unlimited: false,
            colors: vec![0; 128 * 128],
            decorations: Vec::new(),
            banners: Vec::new(),
            frames: Vec::new(),
            holders: Vec::new(),
            dirty: true,
            texture_dirty: true,
        }
    }
    /// `MapItemSavedData.setColor` for every pixel of `colors` (128 by 128, row by row): each
    /// changed one marks the map to save, redraw and send. Whether one changed; None for
    /// another length.
    pub fn paint(&mut self, colors: &[u8]) -> Option<bool> {
        if colors.len() != self.colors.len() {
            return None;
        }
        let mut changed = false;
        for (i, &color) in colors.iter().enumerate() {
            changed |= self.update_color((i % 128) as i32, (i / 128) as i32, color);
        }
        Some(changed)
    }
    pub fn update_color(&mut self, x: i32, y: i32, color: u8) -> bool {
        let i = (x + y * 128) as usize;
        if self.colors[i] == color {
            return false;
        }
        self.colors[i] = color;
        self.dirty = true;
        self.texture_dirty = true;
        // setColorsDirty: every holder's next packet carries it.
        for h in &mut self.holders {
            h.dirty = Some(match h.dirty {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
        true
    }
    /// `tickCarriedBy`'s holder for `who`, added when new.
    pub fn holder(&mut self, who: u32) -> &mut Holder {
        let i = match self.holders.iter().position(|h| h.who == who) {
            Some(i) => i,
            None => {
                self.holders.push(Holder::new(who));
                self.holders.len() - 1
            }
        };
        &mut self.holders[i]
    }
    /// The holders that no longer carry the map nor see it framed go, and
    /// their markers with them (`tickCarriedBy`'s removeDecoration).
    pub fn drop_holders(&mut self, now: i64) {
        self.holders.retain(|h| h.holds(now));
        let holders = &self.holders;
        let before = self.decorations.len();
        self.decorations.retain(|d| match &d.key {
            Key::Player(w) => holders.iter().any(|h| h.who == *w),
            _ => true,
        });
        if self.decorations.len() != before {
            self.decorations_changed();
        }
    }
    /// setDecorationsDirty.
    pub(in crate::game) fn decorations_changed(&mut self) {
        for h in &mut self.holders {
            h.decorations = true;
        }
    }
    /// `MapItemSavedData.removeDecoration`: whatever `key` names goes, and
    /// every holder is told the decorations changed, found or not.
    pub fn remove_decoration(&mut self, key: Key) {
        self.decorations.retain(|d| d.key != key);
        self.decorations_changed();
    }
    /// `MapItemSavedData.addDecoration` with `calculateDecorationLocationAndType`
    /// and `calculateRotation`: `sprite` at the world position `x_pos`,
    /// `z_pos` under `key`, in place when the key has a decoration so that the
    /// order (the drawing order) stays. A position off the map leaves it out
    /// (a player's turns into an off-map arrow, then nothing past 320 without
    /// unlimited tracking) and removes what the key had. `game_time` is the
    /// level's: without a level the Nether's spinning rotation is not used.
    #[allow(clippy::too_many_arguments)]
    pub fn add_decoration(
        &mut self,
        sprite: &'static str,
        game_time: Option<i64>,
        key: Key,
        x_pos: f64,
        z_pos: f64,
        y_rot: f64,
        name: Option<String>,
    ) {
        let scaling = (1 << self.scale) as f32;
        let dx = (x_pos - f64::from(self.center[0])) as f32 / scaling;
        let dz = (z_pos - f64::from(self.center[1])) as f32 / scaling;
        let inside = (-63.0..=63.0).contains(&dx) && (-63.0..=63.0).contains(&dz);
        let shown = if sprite != PLAYER {
            (inside || self.unlimited).then_some(sprite)
        } else if inside {
            Some(sprite)
        } else if dx.abs() < 320.0 && dz.abs() < 320.0 {
            Some(PLAYER_OFF_MAP)
        } else {
            self.unlimited.then_some(PLAYER_OFF_LIMITS)
        };
        let Some(sprite) = shown else {
            self.remove_decoration(key);
            return;
        };
        let spinning = self.dimension == "minecraft:the_nether" && game_time.is_some();
        let decoration = Decoration {
            sprite,
            x: clamp_map_coordinate(dx),
            y: clamp_map_coordinate(dz),
            rot: decoration_rotation(spinning, game_time.unwrap_or(0), y_rot),
            name,
            key,
        };
        match self
            .decorations
            .iter()
            .position(|d| d.key == decoration.key)
        {
            Some(i) if self.decorations[i] == decoration => {}
            Some(i) => {
                self.decorations[i] = decoration;
                self.decorations_changed();
            }
            None => {
                self.decorations.push(decoration);
                self.decorations_changed();
            }
        }
    }
    /// `tickCarriedBy` adds only missing component keys. Another carried
    /// copy without those entries must not erase an explorer map's target.
    pub fn add_item_decorations(&mut self, targets: Vec<StaticDecoration>, game_time: i64) {
        for (name, sprite, x, z, rotation) in targets {
            let key = Key::Item(name);
            if !self.decorations.iter().any(|d| d.key == key) {
                self.add_decoration(
                    sprite,
                    Some(game_time),
                    key,
                    x,
                    z,
                    f64::from(rotation),
                    None,
                );
            }
        }
    }
    /// `MapItemSavedData.tickCarriedBy`'s check on every holder but `who`, the
    /// player ticking: one wearing map invisibility gear has no marker.
    pub fn hide_invisible(&mut self, who: u32) {
        let hidden: Vec<u32> = self
            .holders
            .iter()
            .filter(|h| h.who != who && h.invisible)
            .map(|h| h.who)
            .collect();
        for w in hidden {
            self.remove_decoration(Key::Player(w));
        }
    }
    /// `MapItemSavedData.tickCarriedBy` with `placedInFrame`, for `who`, a
    /// player in the level of `frame`, the item frame holding the map.
    /// `carries` is whether their inventory holds the map (`mapMatcher`),
    /// `invisible` whether they wear map invisibility gear. Without a carried
    /// copy the player has no marker; the frame gets its own, which replaces
    /// the marker of another frame entity remembered at the same block.
    pub fn tick_framed(
        &mut self,
        who: u32,
        now: i64,
        carries: bool,
        invisible: bool,
        frame: Framed,
    ) {
        let h = self.holder(who);
        h.framed = now;
        h.invisible = invisible;
        if !carries {
            self.remove_decoration(Key::Player(who));
        }
        self.hide_invisible(who);
        if !self.tracking {
            return;
        }
        let pos = frame.pos;
        if let Some(old) = self.frames.iter().find(|f| f.pos == pos).copied()
            && old.entity_id != frame.id
        {
            self.remove_decoration(Key::Frame(old.entity_id));
        }
        let rotation = direction_2d(frame.facing) * 90;
        let (x, z) = (f64::from(pos[0]), f64::from(pos[2]));
        let key = Key::Frame(frame.id);
        self.add_decoration(FRAME, Some(now), key, x, z, f64::from(rotation), None);
        let marker = MapFrame {
            pos,
            rotation,
            entity_id: frame.id,
        };
        match self.frames.iter().position(|f| f.pos == pos) {
            Some(i) if self.frames[i] == marker => {}
            Some(i) => {
                self.frames[i] = marker;
                self.dirty = true;
            }
            None => {
                self.frames.push(marker);
                self.dirty = true;
            }
        }
    }
    /// `MapItemSavedData.removedFromFrame`: the frame entity's marker goes,
    /// and the frame remembered at its block.
    pub fn removed_from_frame(&mut self, pos: [i32; 3], entity_id: i32) {
        self.remove_decoration(Key::Frame(entity_id));
        self.frames.retain(|f| f.pos != pos);
        self.dirty = true;
    }
    /// `MapItemSavedData.isTrackedCountOverLimit`: more than `limit`
    /// decorations count (`MapDecorationType.trackCount`).
    pub fn tracked_over_limit(&self, limit: usize) -> bool {
        self.decorations
            .iter()
            .filter(|d| tracks_count(d.sprite))
            .count()
            > limit
    }
    /// `MapItemSavedData.toggleBanner`: a banner inside the map that has no
    /// marker gets one (unless the map tracks too many), one whose marker is
    /// there as it stands loses it, and a changed banner's marker is renewed.
    /// `current` is the banner at `pos` (`MapBanner.fromWorld`); true when
    /// the markers changed.
    pub fn toggle_banner(
        &mut self,
        pos: [i32; 3],
        current: Option<MapBanner>,
        game_time: i64,
    ) -> bool {
        let (x_pos, z_pos) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
        let scale = f64::from(1 << self.scale);
        let xd = (x_pos - f64::from(self.center[0])) / scale;
        let yd = (z_pos - f64::from(self.center[1])) / scale;
        if !(xd >= -63.0 && yd >= -63.0 && xd <= 63.0 && yd <= 63.0) {
            return false;
        }
        let Some(banner) = current else {
            return false;
        };
        if let Some(i) = self.banners.iter().position(|b| *b == banner) {
            self.banners.remove(i);
            self.remove_decoration(Key::Banner(pos));
            self.dirty = true;
            return true;
        }
        if self.tracked_over_limit(256) {
            return false;
        }
        self.banners.retain(|b| b.pos != pos);
        self.banners.push(banner.clone());
        let sprite = banner.sprite();
        self.add_decoration(
            sprite,
            Some(game_time),
            Key::Banner(pos),
            x_pos,
            z_pos,
            180.0,
            banner.name,
        );
        self.dirty = true;
        true
    }
    /// `MapItemSavedData.checkBanners`: a banner marker in the column `x`,
    /// `z` whose banner is gone or changed (`current` reads
    /// `MapBanner.fromWorld` at a position) is dropped with its decoration.
    pub fn check_banners(
        &mut self,
        x: i32,
        z: i32,
        current: impl Fn([i32; 3]) -> Option<MapBanner>,
    ) {
        let stale: Vec<MapBanner> = self
            .banners
            .iter()
            .filter(|b| b.pos[0] == x && b.pos[2] == z && current(b.pos).as_ref() != Some(*b))
            .cloned()
            .collect();
        for banner in stale {
            self.banners.retain(|b| *b != banner);
            self.remove_decoration(Key::Banner(banner.pos));
            self.dirty = true;
        }
    }
    /// `HoldingPlayer.nextUpdatePacket` for `who`: the changed pixels, and
    /// the decorations (at most every 5th call while they changed).
    pub fn update_packet(
        &mut self,
        id: i32,
        who: u32,
        kind: impl Fn(&str) -> Option<i32>,
    ) -> Option<sunrise_protocol::packets::play::MapItemData> {
        use sunrise_protocol::packets::play::{MapDecoration, MapItemData, MapPatch};
        let h = self.holders.iter_mut().find(|h| h.who == who)?;
        let patch = h.dirty.take().map(|[x0, y0, x1, y1]| {
            let (width, height) = (x1 + 1 - x0, y1 + 1 - y0);
            let mut colors = Vec::with_capacity((width * height) as usize);
            for y in 0..height {
                for x in 0..width {
                    colors.push(self.colors[(x0 + x + (y0 + y) * 128) as usize]);
                }
            }
            MapPatch {
                start_x: x0,
                start_y: y0,
                width,
                height,
                colors,
            }
        });
        let decorations = if h.decorations && {
            let due = h.tick % 5 == 0;
            h.tick += 1;
            due
        } {
            h.decorations = false;
            Some(
                self.decorations
                    .iter()
                    .filter_map(|d| {
                        Some(MapDecoration {
                            kind: kind(decoration_type(d.sprite)?)?,
                            x: d.x,
                            y: d.y,
                            rot: d.rot as i8,
                            name: d.name.as_ref().map(|n| Tag::String(n.as_str().into())),
                        })
                    })
                    .collect(),
            )
        } else {
            None
        };
        (patch.is_some() || decorations.is_some()).then_some(MapItemData {
            map_id: id,
            scale: self.scale as i8,
            locked: self.locked,
            decorations,
            patch,
        })
    }
    /// `MapItemSavedData.<clinit>`: the CODEC save arm omits optional defaults.
    pub fn to_nbt(&self) -> Tag {
        let mut data = vec![
            ("dimension", Tag::String(self.dimension.as_str().into())),
            ("xCenter", Tag::Int(self.center[0])),
            ("zCenter", Tag::Int(self.center[1])),
            ("colors", Tag::ByteArray(self.colors.clone())),
        ];
        if self.scale != 0 {
            data.push(("scale", Tag::Byte(self.scale as i8)));
        }
        for (key, value, default) in [
            ("locked", self.locked, false),
            ("trackingPosition", self.tracking, true),
            ("unlimitedTracking", self.unlimited, false),
        ] {
            if value != default {
                data.push((key, Tag::Byte(i8::from(value))));
            }
        }
        if !self.banners.is_empty() {
            data.push((
                "banners",
                Tag::List(self.banners.iter().map(MapBanner::to_nbt).collect()),
            ));
        }
        if !self.frames.is_empty() {
            data.push((
                "frames",
                Tag::List(self.frames.iter().copied().map(MapFrame::to_nbt).collect()),
            ));
        }
        Tag::compound([
            ("data", Tag::compound(data)),
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
        ])
    }
    pub fn from_nbt(tag: &Tag) -> Option<Self> {
        let d = tag.get("data")?;
        let int = |k: &str| match d.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let dimension = match d.get("dimension") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => "minecraft:overworld".to_owned(),
        };
        let mut colors = match d.get("colors") {
            Some(Tag::ByteArray(c)) => c.clone(),
            _ => Vec::new(),
        };
        colors.resize(128 * 128, 0);
        let list = |k: &str| match d.get(k) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        let mut map = Self {
            center: [int("xCenter")?, int("zCenter")?],
            scale: int("scale").unwrap_or(0).clamp(0, 4) as u8,
            dimension,
            locked: int("locked").unwrap_or(0) != 0,
            tracking: int("trackingPosition").unwrap_or(1) != 0,
            unlimited: int("unlimitedTracking").unwrap_or(0) != 0,
            colors,
            decorations: Vec::new(),
            banners: list("banners")
                .iter()
                .filter_map(MapBanner::from_nbt)
                .collect(),
            frames: list("frames")
                .iter()
                .filter_map(MapFrame::from_nbt)
                .collect(),
            holders: Vec::new(),
            dirty: false,
            texture_dirty: true,
        };
        // The constructor adds each marker without a level: no Nether spin.
        for b in map.banners.clone() {
            let (x, z) = (f64::from(b.pos[0]), f64::from(b.pos[2]));
            map.add_decoration(b.sprite(), None, Key::Banner(b.pos), x, z, 180.0, b.name);
        }
        for f in map.frames.clone() {
            let (x, z) = (f64::from(f.pos[0]), f64::from(f.pos[2]));
            let rotation = f64::from(f.rotation);
            map.add_decoration(FRAME, None, Key::Frame(f.entity_id), x, z, rotation, None);
        }
        Some(map)
    }
}
#[derive(Default)]
pub struct Maps {
    pub data: HashMap<i32, MapData>,
    /// The highest map id handed out (`maps/last_id.dat`), loaded on first use.
    pub last_id: Option<i32>,
    pub missing: std::collections::HashSet<i32>,
}
/// `MapColor.getColorFromPackedId` as RGBA bytes (NONE and undefined IDs are transparent).
pub fn packed_rgba(packed: u8) -> [u8; 4] {
    let id = usize::from(packed >> 2);
    let Some(&col) = MAP_COLORS.get(id).filter(|_| id != 0) else {
        return [0; 4];
    };
    let modifier = [180u32, 220, 255, 135][usize::from(packed & 3)];
    let ch = |s: u32| ((col >> s & 255) * modifier / 255) as u8;
    [ch(16), ch(8), ch(0), 255]
}

/// `get(DataComponents.MAP_ID)`: the map id any stack carries. Filled maps
/// (explorer maps included) carry one; a command can give it to any item.
pub fn map_id(stack: &ItemStack) -> Option<i32> {
    stack.data().and_then(|d| d.map_id)
}
/// `MapDecorationTypes` Sunrise draws: (type, sprite).
pub(in crate::game) const DECORATION_TYPES: [(&str, &str); 40] = [
    ("player", "map/decorations/player"),
    ("frame", "map/decorations/frame"),
    ("red_marker", "map/decorations/red_marker"),
    ("blue_marker", "map/decorations/blue_marker"),
    ("target_x", "map/decorations/target_x"),
    ("target_point", "map/decorations/target_point"),
    ("player_off_map", "map/decorations/player_off_map"),
    ("player_off_limits", "map/decorations/player_off_limits"),
    ("mansion", "map/decorations/woodland_mansion"),
    ("monument", "map/decorations/ocean_monument"),
    ("banner_white", "map/decorations/white_banner"),
    ("banner_orange", "map/decorations/orange_banner"),
    ("banner_magenta", "map/decorations/magenta_banner"),
    ("banner_light_blue", "map/decorations/light_blue_banner"),
    ("banner_yellow", "map/decorations/yellow_banner"),
    ("banner_lime", "map/decorations/lime_banner"),
    ("banner_pink", "map/decorations/pink_banner"),
    ("banner_gray", "map/decorations/gray_banner"),
    ("banner_light_gray", "map/decorations/light_gray_banner"),
    ("banner_cyan", "map/decorations/cyan_banner"),
    ("banner_purple", "map/decorations/purple_banner"),
    ("banner_blue", "map/decorations/blue_banner"),
    ("banner_brown", "map/decorations/brown_banner"),
    ("banner_green", "map/decorations/green_banner"),
    ("banner_red", "map/decorations/red_banner"),
    ("banner_black", "map/decorations/black_banner"),
    ("red_x", "map/decorations/red_x"),
    ("village_desert", "map/decorations/desert_village"),
    ("village_plains", "map/decorations/plains_village"),
    ("village_savanna", "map/decorations/savanna_village"),
    ("village_snowy", "map/decorations/snowy_village"),
    ("village_taiga", "map/decorations/taiga_village"),
    ("jungle_temple", "map/decorations/jungle_temple"),
    ("swamp_hut", "map/decorations/swamp_hut"),
    ("trial_chambers", "map/decorations/trial_chambers"),
    ("abandoned_camp", "map/decorations/abandoned_camp"),
    ("ancient_city", "map/decorations/ancient_city"),
    ("desert_pyramid", "map/decorations/desert_pyramid"),
    ("mineshaft", "map/decorations/mineshaft"),
    ("ocean_ruin_warm", "map/decorations/warm_ocean_ruins"),
];
/// `MapDecorationType.showOnItemFrame` is false for the player arrows and the two markers.
/// `MapRenderer.render`'s pose for decoration `d` in map pixels (0..128,
/// y down) at depth `z`: its quad is (-1, -1) to (1, 1) under it.
pub fn decoration_pose(d: &Decoration, z: f32) -> glam::Mat4 {
    decoration_pose_at(d.x, d.y, d.rot, z)
}
/// [`decoration_pose`] for a decoration at `x`, `y` (half map pixels) turned `rot`.
pub fn decoration_pose_at(x: i8, y: i8, rot: u8, z: f32) -> glam::Mat4 {
    glam::Mat4::from_translation(glam::Vec3::new(
        f32::from(x) / 2.0 + 64.0,
        f32::from(y) / 2.0 + 64.0,
        z,
    )) * glam::Mat4::from_rotation_z((f32::from(rot) * 360.0 / 16.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(4.0))
        * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.125, 0.0))
}
pub fn shown_on_frame(sprite: &str) -> bool {
    // A mod's type says for itself (`showOnItemFrame`).
    if let Some(shown) = sunrise_sim::map_decoration_types::registered().shown_on_frame(sprite) {
        return shown;
    }
    !matches!(
        decoration_type(sprite),
        Some("player" | "red_marker" | "blue_marker" | "player_off_map" | "player_off_limits")
    )
}
/// `MapDecorationType.trackCount`: the players' arrows, the markers, the
/// frames and the banners count toward a map's limit, the structures do not.
pub(in crate::game) fn tracks_count(sprite: &str) -> bool {
    decoration_type(sprite).is_some_and(|t| {
        t.starts_with("banner_")
            || matches!(
                t,
                "player"
                    | "frame"
                    | "red_marker"
                    | "blue_marker"
                    | "player_off_map"
                    | "player_off_limits"
            )
    })
}
/// The sprite of a decoration type that a `minecraft:map_decorations`
/// entry or a server's packet names: vanilla's, or one a mod registered
/// (`map_decoration_types`).
pub fn decoration_sprite(id: &str) -> Option<&'static str> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    DECORATION_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sprite)| *sprite)
        .or_else(|| sunrise_sim::map_decoration_types::registered().sprite(id))
}
/// The type a decoration's sprite stands for, as its registry names it (a mod's with its
/// namespace).
pub(in crate::game) fn decoration_type(sprite: &str) -> Option<&'static str> {
    DECORATION_TYPES
        .iter()
        .find(|(_, s)| *s == sprite)
        .map(|(name, _)| *name)
        .or_else(|| sunrise_sim::map_decoration_types::registered().id_of(sprite))
}
/// Whether `sprite` draws one of vanilla's decoration types: a mod's type may not take it.
pub(in crate::game) fn is_vanilla_decoration_sprite(sprite: &str) -> bool {
    DECORATION_TYPES.iter().any(|(_, s)| *s == sprite)
}
/// A named `minecraft:map_decorations` entry: key, sprite, x, z, rotation.
pub type StaticDecoration = (JavaString, &'static str, f64, f64, f32);

/// `MapItemSavedData.clampMapCoordinate`.
pub fn clamp_map_coordinate(d: f32) -> i8 {
    if d <= -63.0 {
        -128
    } else if d >= 63.0 {
        127
    } else {
        (d * 2.0 + 0.5) as i8
    }
}
/// `MapItemSavedData.calculateRotation`: spinning in the Nether.
pub fn decoration_rotation(nether: bool, game_time: i64, y_rot: f64) -> u8 {
    if nether {
        let s = (game_time / 10) as i32;
        (s.wrapping_mul(s)
            .wrapping_mul(34_187_121)
            .wrapping_add(s.wrapping_mul(121))
            >> 15
            & 15) as u8
    } else {
        let adjusted = if y_rot < 0.0 {
            y_rot - 8.0
        } else {
            y_rot + 8.0
        };
        ((adjusted * 16.0 / 360.0) as i32 & 15) as u8
    }
}
impl MapData {
    /// `MapItemSavedData.scaled`: a fresh map one scale up around the same centre.
    pub fn scaled(&self) -> MapData {
        let mut m = MapData::fresh(
            f64::from(self.center[0]),
            f64::from(self.center[1]),
            (self.scale + 1).min(4),
            &self.dimension,
        );
        m.tracking = self.tracking;
        m.unlimited = self.unlimited;
        m
    }
    /// `MapItemSavedData.locked`: the same pixels, banner markers and
    /// decorations, locked (the frame markers are not copied).
    pub fn locked_copy(&self) -> MapData {
        let mut m = MapData::fresh(
            f64::from(self.center[0]),
            f64::from(self.center[1]),
            self.scale,
            &self.dimension,
        );
        m.center = self.center;
        m.colors = self.colors.clone();
        m.banners = self.banners.clone();
        m.decorations = self.decorations.clone();
        m.tracking = self.tracking;
        m.unlimited = self.unlimited;
        m.locked = true;
        m
    }
}
