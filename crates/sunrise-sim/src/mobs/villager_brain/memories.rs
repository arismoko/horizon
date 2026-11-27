// Included in villager_brain.rs to retain its private scope.
/// `PositionTracker`: an entity (`EntityTracker`, at its eyes or feet) or a
/// block (`BlockPosTracker`, at its centre or at a given point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tracker {
    Entity(Who, bool),
    Block([i32; 3]),
    Point([f64; 3]),
}
impl Tracker {
    /// `currentPosition`.
    pub fn position(&self, w: &MobWorld) -> Option<[f64; 3]> {
        match *self {
            Tracker::Entity(who, eye) => w.target_info(who).map(|t| {
                [
                    t.position.x,
                    if eye { t.eye_y } else { t.position.y },
                    t.position.z,
                ]
            }),
            Tracker::Block(p) => Some([
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]),
            Tracker::Point(v) => Some(v),
        }
    }
    /// `currentBlockPosition`.
    pub fn block(&self, w: &MobWorld) -> Option<[i32; 3]> {
        match *self {
            Tracker::Entity(who, _) => w.target_info(who).map(|t| block_of(t.position)),
            Tracker::Block(p) => Some(p),
            Tracker::Point(v) => Some([
                v[0].floor() as i32,
                v[1].floor() as i32,
                v[2].floor() as i32,
            ]),
        }
    }
}
fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
/// `WalkTarget`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTarget {
    pub target: Tracker,
    pub speed: f32,
    pub close: i32,
}
impl WalkTarget {
    pub(super) fn block(p: [i32; 3], speed: f32, close: i32) -> Self {
        Self {
            target: Tracker::Block(p),
            speed,
            close,
        }
    }
    /// `new WalkTarget(Vec3, ...)`: the block containing the point.
    pub(super) fn point(p: [f64; 3], speed: f32, close: i32) -> Self {
        Self::block(
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ],
            speed,
            close,
        )
    }
}

/// `NearestVisibleLivingEntities`: the nearby entities and a lazily filled
/// cache of `Sensor.isEntityTargetable`.
#[derive(Clone, Debug, Default)]
pub struct Visible {
    pub(super) list: Vec<Who>,
    pub(super) cache: Vec<(Who, bool)>,
}

/// The memories a villager's brain registers.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    pub home: Slot<[i32; 3]>,
    pub job_site: Slot<[i32; 3]>,
    pub potential_job_site: Slot<[i32; 3]>,
    pub meeting_point: Slot<[i32; 3]>,
    pub secondary_job_site: Slot<Vec<[i32; 3]>>,
    pub mobs: Slot<Vec<Who>>,
    pub visible_mobs: Slot<Visible>,
    pub visible_villager_babies: Slot<Vec<Who>>,
    pub nearest_players: Slot<Vec<Who>>,
    pub nearest_visible_player: Slot<Who>,
    pub nearest_visible_targetable_player: Slot<Who>,
    pub nearest_visible_targetable_players: Slot<Vec<Who>>,
    pub walk_target: Slot<WalkTarget>,
    pub look_target: Slot<Tracker>,
    pub interaction_target: Slot<Who>,
    pub breed_target: Slot<Who>,
    /// `PATH`: the navigation's path (its target).
    pub path: Slot<Path>,
    pub doors_to_close: Slot<Vec<[i32; 3]>>,
    pub nearest_bed: Slot<[i32; 3]>,
    /// `HURT_BY`: the damage (its game time).
    pub hurt_by: Slot<i64>,
    pub hurt_by_entity: Slot<Who>,
    pub nearest_hostile: Slot<Who>,
    pub hiding_place: Slot<[i32; 3]>,
    pub heard_bell_time: Slot<i64>,
    /// `NEAREST_VISIBLE_WANTED_ITEM`: where the item entity lies.
    pub nearest_wanted_item: Slot<[f64; 3]>,
    pub cant_reach_walk_target_since: Slot<i64>,
    pub golem_detected_recently: Slot<bool>,
    pub last_slept: Slot<i64>,
    pub last_woken: Slot<i64>,
    pub last_worked_at_poi: Slot<i64>,
}
impl Memories {
    fn tick(&mut self) {
        self.home.tick();
        self.job_site.tick();
        self.potential_job_site.tick();
        self.meeting_point.tick();
        self.secondary_job_site.tick();
        self.mobs.tick();
        self.visible_mobs.tick();
        self.visible_villager_babies.tick();
        self.nearest_players.tick();
        self.nearest_visible_player.tick();
        self.nearest_visible_targetable_player.tick();
        self.nearest_visible_targetable_players.tick();
        self.walk_target.tick();
        self.look_target.tick();
        self.interaction_target.tick();
        self.breed_target.tick();
        self.path.tick();
        self.doors_to_close.tick();
        self.nearest_bed.tick();
        self.hurt_by.tick();
        self.hurt_by_entity.tick();
        self.nearest_hostile.tick();
        self.hiding_place.tick();
        self.heard_bell_time.tick();
        self.nearest_wanted_item.tick();
        self.cant_reach_walk_target_since.tick();
        self.golem_detected_recently.tick();
        self.last_slept.tick();
        self.last_woken.tick();
        self.last_worked_at_poi.tick();
    }
    /// `Brain.pack`: the memories with a codec, which a rebuilt brain keeps.
    fn pack(&self) -> Memories {
        Memories {
            home: self.home.clone(),
            job_site: self.job_site.clone(),
            potential_job_site: self.potential_job_site.clone(),
            meeting_point: self.meeting_point.clone(),
            golem_detected_recently: self.golem_detected_recently.clone(),
            last_slept: self.last_slept.clone(),
            last_woken: self.last_woken.clone(),
            last_worked_at_poi: self.last_worked_at_poi.clone(),
            ..Memories::default()
        }
    }
}

/// `Activity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Core,
    Idle,
    Work,
    Play,
    Rest,
    Meet,
    Panic,
    Raid,
    PreRaid,
    Hide,
}
impl Act {
    pub fn name(self) -> &'static str {
        match self {
            Act::Core => "core",
            Act::Idle => "idle",
            Act::Work => "work",
            Act::Play => "play",
            Act::Rest => "rest",
            Act::Meet => "meet",
            Act::Panic => "panic",
            Act::Raid => "raid",
            Act::PreRaid => "pre_raid",
            Act::Hide => "hide",
        }
    }
    /// The bucket order of `HashMap<Activity, ...>` (16 buckets, the hash
    /// of the name; equal buckets keep the villager's insertion order).
    fn slot(self) -> u8 {
        match self {
            Act::Play => 0,
            Act::Core => 1,
            Act::Rest => 2,
            Act::Hide => 3,
            Act::Meet => 4,
            Act::Idle => 5,
            Act::PreRaid => 6,
            Act::Work => 7,
            Act::Panic => 8,
            Act::Raid => 9,
        }
    }
}

/// The scan rate of each sensor, in `BRAIN_PROVIDER` order.
const SENSOR_RATES: [i64; 9] = [20, 20, 20, 20, 20, 20, 20, 40, 200];

/// A villager's brain.
#[derive(Clone, Debug)]
pub struct Brain {
    pub(super) generation: u64,
    pub mem: Memories,
    sensors: [i64; 9],
    /// The active non-core activity (`CORE` is always active).
    pub activity: Act,
    default_activity: Act,
    last_schedule_update: i64,
    list: Vec<Entry>,
    pub shuffle_seed: i64,
    baby: bool,
    /// Whether the nearest-bed sensor's batch cache is used (`NearestBedSensor`).
    bed_cache: Vec<([i32; 3], i64)>,
    /// `NearestItemSensor` is due; the game, which knows the items, answers.
    scan_items: bool,
}
#[derive(Clone, Debug)]
struct Entry {
    prio: i32,
    act: Act,
    b: B,
}

fn brain(m: &Mob) -> Option<&Brain> {
    super::villager::ext(m).and_then(|e| e.brain.as_deref())
}
fn brain_mut(m: &mut Mob) -> &mut Brain {
    let (baby, farmer) = (m.is_baby(), super::villager::is_farmer(m));
    if brain(m).is_none() {
        let generation = m.next_brain_generation();
        super::villager::ext_mut(m).brain = Some(Box::new(Brain::new(generation, baby, 0, farmer)));
        m.stage_brain_behaviors(super::mod_behaviors::BehaviorTarget::Villager(generation));
    }
    super::villager::ext_mut(m).brain.as_deref_mut().unwrap()
}
pub(super) fn mem(m: &mut Mob) -> &mut Memories {
    &mut brain_mut(m).mem
}
/// `CANT_REACH_WALK_TARGET_SINCE`, for `compute_path`.
fn cant_reach(m: &mut Mob) -> &mut Slot<i64> {
    &mut mem(m).cant_reach_walk_target_since
}
pub(super) fn mem_ref(m: &Mob) -> Option<&Memories> {
    brain(m).map(|b| &b.mem)
}

impl Brain {
    fn new(generation: u64, baby: bool, shuffle_seed: i64, farmer: bool) -> Self {
        let mut b = Brain {
            generation,
            mem: Memories::default(),
            sensors: [0; 9],
            activity: Act::Idle,
            default_activity: Act::Idle,
            last_schedule_update: -9999,
            list: Vec::new(),
            shuffle_seed,
            baby,
            bed_cache: Vec::new(),
            scan_items: false,
        };
        b.list = packages(baby, shuffle_seed, farmer);
        b
    }
    fn requirements_met(&self, a: Act) -> bool {
        match a {
            Act::Work => !self.baby && self.mem.job_site.present(),
            Act::Play => self.baby,
            Act::Meet => self.mem.meeting_point.present(),
            Act::Core
            | Act::Idle
            | Act::Rest
            | Act::Panic
            | Act::Raid
            | Act::PreRaid
            | Act::Hide => true,
        }
    }
    pub fn is_active(&self, a: Act) -> bool {
        a == Act::Core || a == self.activity
    }
    /// `setActiveActivityIfPossible`.
    pub fn set_active_if_possible(&mut self, a: Act) {
        if self.requirements_met(a) {
            self.activity = a;
        } else {
            self.activity = self.default_activity;
        }
    }
    /// `updateActivityFromSchedule`.
    fn update_from_schedule(&mut self, game_time: i64, day_time: i64) {
        if game_time - self.last_schedule_update > 20 {
            self.last_schedule_update = game_time;
            let scheduled = schedule(self.baby, day_time);
            if !self.is_active(scheduled) {
                self.set_active_if_possible(scheduled);
            }
        }
    }
}
