//! Portable shulker-box lid state and collision geometry (`ShulkerBoxBlockEntity`).

/// `Direction` names by `get3DDataValue`.
pub const DIRECTIONS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

fn step(direction: u8) -> [i32; 3] {
    match direction {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

/// `Direction.from3DDataValue`: `BY_3D_DATA[Mth.abs(data % 6)]`.
pub fn from_3d_data_value(data: i32) -> u8 {
    (data % 6).unsigned_abs() as u8
}

/// The 3D data value of a direction name.
pub fn direction_index(name: &str) -> Option<u8> {
    DIRECTIONS.iter().position(|d| *d == name).map(|i| i as u8)
}

/// `ShulkerBoxBlockEntity.AnimationStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShulkerStatus {
    #[default]
    Closed,
    Opening,
    Opened,
    Closing,
}

/// `ShulkerBoxBlockEntity` lid state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shulker {
    pub open_count: i32,
    pub status: ShulkerStatus,
    pub progress: f32,
    pub progress_old: f32,
}

/// Side effects of one [`Shulker::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShulkerTick {
    /// `doNeighborUpdates` calls (shape and neighbour updates around the box).
    pub neighbor_updates: u32,
    /// `moveCollidedEntities`: the swept lid box (min x, y, z, max x, y, z);
    /// entities in it move out along the facing.
    pub push: Option<[f64; 6]>,
}

/// `Shulker.getProgressDeltaAabb(size, direction, from, to, position)`.
pub fn progress_delta_aabb(
    size: f32,
    direction: u8,
    from: f32,
    to: f32,
    position: [f64; 3],
) -> [f64; 6] {
    let size_d = f64::from(size);
    let mut b = [
        -size_d * 0.5,
        0.0,
        -size_d * 0.5,
        size_d * 0.5,
        size_d,
        size_d * 0.5,
    ];
    let max = f64::from(from.max(to));
    let min = f64::from(from.min(to));
    let s = step(direction);
    for axis in 0..3 {
        // expandTowards
        let a = f64::from(s[axis]) * max * size_d;
        if a < 0.0 {
            b[axis] += a;
        } else if a > 0.0 {
            b[axis + 3] += a;
        }
    }
    for axis in 0..3 {
        // contract
        let a = -f64::from(s[axis]) * (1.0 + min) * size_d;
        if a < 0.0 {
            b[axis] -= a;
        } else if a > 0.0 {
            b[axis + 3] -= a;
        }
    }
    // new AABB keeps min <= max; then move to the position.
    let mut out = [0.0; 6];
    for axis in 0..3 {
        out[axis] = b[axis].min(b[axis + 3]) + position[axis];
        out[axis + 3] = b[axis].max(b[axis + 3]) + position[axis];
    }
    out
}

impl Shulker {
    /// `triggerEvent(1, openCount)`.
    pub fn trigger_event(&mut self, b0: i32, b1: i32) -> bool {
        if b0 != 1 {
            return false;
        }
        self.open_count = b1;
        if b1 == 0 {
            self.status = ShulkerStatus::Closing;
        }
        if b1 == 1 {
            self.status = ShulkerStatus::Opening;
        }
        true
    }
    /// `startOpen`/`stopOpen` for a non-spectator user: the new open count to
    /// send as block event 1, and whether this was the first opener (open) or
    /// the last closer (close), which play the sound and game event.
    pub fn start_open(&mut self) -> (i32, bool) {
        if self.open_count < 0 {
            self.open_count = 0;
        }
        self.open_count += 1;
        (self.open_count, self.open_count == 1)
    }
    pub fn stop_open(&mut self) -> (i32, bool) {
        self.open_count -= 1;
        (self.open_count, self.open_count <= 0)
    }
    /// `updateAnimation` (`ShulkerBoxBlockEntity.tick`); `facing` is the
    /// block's facing as a 3D data value.
    pub fn tick(&mut self, pos: [i32; 3], facing: u8) -> ShulkerTick {
        let mut out = ShulkerTick::default();
        self.progress_old = self.progress;
        match self.status {
            ShulkerStatus::Closed => self.progress = 0.0,
            ShulkerStatus::Opening => {
                self.progress += 0.1;
                if self.progress_old == 0.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress >= 1.0 {
                    self.status = ShulkerStatus::Opened;
                    self.progress = 1.0;
                    out.neighbor_updates += 1;
                }
                let bottom = [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]),
                    f64::from(pos[2]) + 0.5,
                ];
                out.push = Some(progress_delta_aabb(
                    1.0,
                    facing,
                    self.progress_old,
                    self.progress,
                    bottom,
                ));
            }
            ShulkerStatus::Opened => self.progress = 1.0,
            ShulkerStatus::Closing => {
                self.progress -= 0.1;
                if self.progress_old == 1.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress <= 0.0 {
                    self.status = ShulkerStatus::Closed;
                    self.progress = 0.0;
                    out.neighbor_updates += 1;
                }
            }
        }
        out
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box only opens when its lid has room
    /// (`level.noCollision` of the lid box at half progress, deflated by 1e-6).
    pub fn lid_box(pos: [i32; 3], facing: u8) -> [f64; 6] {
        let bottom = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let mut b = progress_delta_aabb(1.0, facing, 0.0, 0.5, bottom);
        for axis in 0..3 {
            b[axis] += 1.0e-6;
            b[axis + 3] -= 1.0e-6;
        }
        b
    }
}
