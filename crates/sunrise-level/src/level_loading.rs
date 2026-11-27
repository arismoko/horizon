//! Portable server chunk-loading progress and chunk-status map view.

/// `Minecraft.doWorldLoad`: `max(5, 3) + ChunkLevel.RADIUS_AROUND_FULL_CHUNK + 1`.
pub const STATUS_RADIUS: i32 = 8;
/// `LevelLoadProgressTracker.EXPECTED_PLAYER_CHUNKS`: the 7x7 chunks
/// `PrepareSpawnTask` waits for around the player.
const PLAYER_CHUNKS: usize = 49;
/// `LevelLoadProgressTracker.PREPARE_SERVER_WEIGHT`.
const PREPARE_WEIGHT: usize = 10;

/// `ChunkStatus`, as far as the chunk status map tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}
impl Status {
    /// `LevelLoadingScreen.COLORS` (RGB).
    pub fn color(self) -> u32 {
        match self {
            Self::Empty => 0x545454,
            Self::StructureStarts => 0x999999,
            Self::StructureReferences => 0x5F6191,
            Self::Biomes => 0x80B252,
            Self::Terrain => 0x303572,
            Self::Features => 0x21C600,
            Self::InitializeLight => 0xCCCCCC,
            Self::Light => 0xFFE0A0,
            Self::Spawn => 0xF26060,
            Self::Full => 0xFFFFFF,
        }
    }
}

/// `LevelLoadListener.Stage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    StartServer,
    PrepareGlobalSpawn,
    LoadInitialChunks,
    LoadPlayerChunks,
}

/// `LevelLoadProgressTracker` (with `includePlayerChunks`).
#[derive(Debug, Default)]
struct ProgressTracker {
    total_weight: usize,
    finalized_weight: usize,
    segment_weight: usize,
    segment_fraction: f32,
    progress: f32,
}
impl ProgressTracker {
    fn tracks(stage: Stage) -> bool {
        matches!(stage, Stage::LoadInitialChunks | Stage::LoadPlayerChunks)
    }
    fn begin_segment(&mut self, weight: usize) {
        self.segment_weight = weight;
        self.segment_fraction = 0.0;
        self.update_progress();
    }
    fn finish_segment(&mut self) {
        self.finalized_weight += self.segment_weight;
        self.segment_weight = 0;
        self.update_progress();
    }
    fn update_progress(&mut self) {
        self.progress = if self.total_weight == 0 {
            0.0
        } else {
            (self.finalized_weight as f32 + self.segment_fraction * self.segment_weight as f32)
                / self.total_weight as f32
        };
    }
    fn start(&mut self, stage: Stage, total_chunks: usize) {
        match stage {
            Stage::LoadInitialChunks => {
                self.total_weight = PREPARE_WEIGHT + total_chunks + PLAYER_CHUNKS;
                self.begin_segment(PREPARE_WEIGHT);
                self.finish_segment();
                self.begin_segment(total_chunks);
            }
            Stage::LoadPlayerChunks => self.begin_segment(PLAYER_CHUNKS),
            _ => {}
        }
    }
    fn update(&mut self, stage: Stage, current: usize, total: usize) {
        if Self::tracks(stage) {
            self.segment_fraction = if total == 0 {
                0.0
            } else {
                current as f32 / total as f32
            };
            self.update_progress();
        }
    }
    fn finish(&mut self, stage: Stage) {
        if Self::tracks(stage) {
            self.finish_segment();
        }
    }
}

/// The integrated server's side of `LevelLoadTracker`: what its
/// `LevelLoadListener` reported (`serverProgress`, `hasProgress`) and the
/// chunk status map around the load focus (`ChunkLoadStatusView`), which the
/// server thread writes and the window's loading screen reads.
#[derive(Debug, Default)]
pub struct ServerLoadView {
    tracker: std::sync::Mutex<ProgressTracker>,
    /// `serverStage != null`.
    has_progress: std::sync::atomic::AtomicBool,
    /// `statusView.get(x, z)` for x and z in `0..=2 * STATUS_RADIUS`.
    cells: std::sync::Mutex<Vec<Option<Status>>>,
}
impl ServerLoadView {
    /// `LevelLoadListener.start`.
    pub fn start(&self, stage: Stage, total_chunks: usize) {
        Self::lock(&self.tracker).start(stage, total_chunks);
        // `serverStage = stage`, whatever the stage.
        self.has_progress
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    /// `LevelLoadListener.update`.
    pub fn update(&self, stage: Stage, current: usize, total: usize) {
        Self::lock(&self.tracker).update(stage, current, total);
    }
    /// `LevelLoadListener.finish`.
    pub fn finish(&self, stage: Stage) {
        Self::lock(&self.tracker).finish(stage);
    }
    /// `LevelLoadTracker.hasProgress`.
    pub fn has_progress(&self) -> bool {
        self.has_progress.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// `LevelLoadTracker.serverProgress`.
    pub fn server_progress(&self) -> f32 {
        Self::lock(&self.tracker).progress
    }
    /// The map `ChunkLoadStatusView.get` reads, replaced as a whole.
    pub fn set_status(&self, cells: Vec<Option<Status>>) {
        *Self::lock(&self.cells) = cells;
    }
    /// `ChunkLoadStatusView.get`.
    pub fn status(&self, x: i32, z: i32) -> Option<Status> {
        let diameter = (STATUS_RADIUS * 2 + 1) as usize;
        let (x, z) = (usize::try_from(x).ok()?, usize::try_from(z).ok()?);
        if x >= diameter || z >= diameter {
            return None;
        }
        Self::lock(&self.cells)
            .get(x * diameter + z)
            .copied()
            .flatten()
    }
    fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `LevelLoadProgressTracker.get` once the server is prepared, with `ready`
/// of the player's 49 chunks loaded (no forced chunks to load first).
pub fn progress(ready: usize) -> f32 {
    let total = PREPARE_WEIGHT + PLAYER_CHUNKS;
    (PREPARE_WEIGHT + ready.min(PLAYER_CHUNKS)) as f32 / total as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_progress_follows_the_listener_like_the_progress_tracker() {
        let view = ServerLoadView::default();
        assert!(!view.has_progress() && view.server_progress() == 0.0);
        // A stage the tracker ignores still counts as progress being made.
        view.start(Stage::StartServer, 0);
        assert!(view.has_progress() && view.server_progress() == 0.0);
        // prepareLevels with no forced chunks: the 10 of preparing, of 59.
        view.start(Stage::LoadInitialChunks, 0);
        view.finish(Stage::LoadInitialChunks);
        assert_eq!(view.server_progress(), 10.0 / 59.0);
        // PrepareSpawnTask: the 49 player chunks.
        view.start(Stage::LoadPlayerChunks, 49);
        view.update(Stage::LoadPlayerChunks, 20, 49);
        assert_eq!(view.server_progress(), (10.0 + 20.0) / 59.0);
        assert_eq!(view.server_progress(), progress(20));
        view.update(Stage::LoadPlayerChunks, 49, 49);
        view.finish(Stage::LoadPlayerChunks);
        assert_eq!(view.server_progress(), 1.0);
    }

    #[test]
    fn the_status_map_is_read_from_the_north_west_corner() {
        let view = ServerLoadView::default();
        assert_eq!(view.status(0, 0), None);
        let diameter = (STATUS_RADIUS * 2 + 1) as usize;
        let mut cells = vec![None; diameter * diameter];
        cells[3 * diameter + 5] = Some(Status::Full);
        view.set_status(cells);
        assert_eq!(view.status(3, 5), Some(Status::Full));
        assert_eq!(view.status(5, 3), None);
        assert_eq!(view.status(-1, 0), None);
        assert_eq!(view.status(diameter as i32, 0), None);
    }
}
