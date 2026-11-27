// Included in goals.rs to retain its private scope.
pub const MOVE: u8 = 1;
pub const LOOK: u8 = 2;
pub const JUMP: u8 = 4;
pub const TARGET: u8 = 8;

/// The facts of a target one mob may take, as far as [`Mob::valid_target`]
/// reads them (#1175): another mob — what a tick's snapshot carries, or the
/// game's own state — or a player. One rule set; both the write handle and a
/// goal tick's view fill these from where they stand.
pub enum TargetFacts {
    /// A mob target: `asValidTarget`'s facts of it.
    Mob {
        id: u32,
        species: Species,
        alive: bool,
        /// `invulnerable_flag`, the flag `asValidTarget` refuses.
        invulnerable: bool,
        baby: bool,
    },
    /// A player target: its survival liveness, its `invulnerable` ability,
    /// the modes that cannot be attacked (creative, spectator) and its uuid.
    Player {
        alive: bool,
        invulnerable: bool,
        creative: bool,
        spectator: bool,
        uuid: [i32; 4],
    },
}

/// What goals may read and emit during a tick.
pub struct Cx<'a, 'w, 'l> {
    pub w: &'a MobWorld<'w>,
    /// The level: its random (`level.getRandom()`), for the draws vanilla's
    /// behaviours make from it rather than from the mob's own, and the damage hooks.
    pub level: &'a mut crate::health::DamageLevel<'l>,
    pub events: &'a mut Vec<MobEvent>,
}
pub trait Goal: std::fmt::Debug + Send + Sync {
    fn flags(&self) -> u8;
    /// A goal that draws its first start delay in its constructor (a schooling fish's
    /// `FollowFlockLeaderGoal`): the delay again, when the mob's random is seeded afterwards.
    #[inline]
    fn set_start_delay(&mut self, _ticks: i32) {}
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool;
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    #[inline]
    fn interruptable(&self) -> bool {
        true
    }
    #[inline]
    fn start(&mut self, _m: &mut Mob, _cx: &mut Cx) {}
    #[inline]
    fn stop(&mut self, _m: &mut Mob, _cx: &mut Cx) {}
    #[inline]
    fn every_tick(&self) -> bool {
        false
    }
    /// A goal that stands in for a brain behaviour: it tries to start on every tick,
    /// as `Brain.startEachNonRunningBehavior` does, not only on the goal selector's
    /// full ticks.
    #[inline]
    fn starts_every_tick(&self) -> bool {
        false
    }
    #[inline]
    fn tick(&mut self, _m: &mut Mob, _cx: &mut Cx) {}
    /// Goals that move the body itself can stop for the owning level to handle
    /// a trample, passenger fall, or hurt. The ordinary path runs without a pause.
    #[inline]
    fn tick_until_movement_pause(
        &mut self,
        mob: &mut Mob,
        cx: &mut Cx,
    ) -> Option<super::living::TravelPause> {
        self.tick(mob, cx);
        None
    }
    /// Finish a goal that stopped in its own movement; false for a different goal.
    #[inline]
    fn after_movement(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn box_clone(&self) -> Box<dyn Goal>;
    /// The goal's name for debugging (`Goal.toString`).
    #[inline]
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
            .rsplit("::")
            .next()
            .unwrap_or("Goal")
    }
}
impl Clone for Box<dyn Goal> {
    #[inline]
    fn clone(&self) -> Self {
        self.box_clone()
    }
}
macro_rules! goal_clone {
    () => {
        #[inline]
        fn box_clone(&self) -> Box<dyn crate::mobs::goals::Goal> {
            Box::new(self.clone())
        }
    };
}
pub(crate) use goal_clone;

/// Goal.reducedTickDelay
pub fn reduced(ticks: i32) -> i32 {
    (ticks + 1) / 2
}
fn adjusted(every_tick: bool, ticks: i32) -> i32 {
    if every_tick { ticks } else { reduced(ticks) }
}

#[derive(Clone, Debug)]
struct Wrapped {
    priority: i32,
    goal: Box<dyn Goal>,
    running: bool,
}
#[derive(Clone, Debug, Default)]
pub struct GoalSelector {
    goals: Vec<Wrapped>,
    locked: [Option<usize>; 4],
    pub disabled: u8,
    /// How many goals start on every tick ([`Goal::starts_every_tick`]): with none, a tick
    /// that only ticks the running goals tries no start.
    brain_starters: usize,
    /// Bumped whenever the selector is cleared or reset so staged `register_goals`
    /// applications can detect selector mutations while listeners ran.
    generation: u64,
}
/// Where a goal tick paused for a game-owned move. Public, with [`GoalPause`], [`GoalBreed`], and
/// [`GoalSelector::tick_until_pause`], because `ai::server_ai_step`, a replaceable function,
/// calls it: a replacement copy of that function is compiled outside this crate.
#[derive(Debug)]
pub struct GoalMovementPause {
    pub(super) movement: super::living::TravelPause,
    slot: usize,
    pub(super) force: bool,
}

#[derive(Debug)]
pub struct GoalBreed {
    next: usize,
    force: bool,
}

#[derive(Debug)]
pub enum GoalPause {
    Movement(GoalMovementPause),
    Breed(Box<MobEvent>, GoalBreed),
}

impl GoalSelector {
    pub fn add(&mut self, priority: i32, goal: impl Goal + 'static) {
        self.add_boxed(priority, Box::new(goal));
    }
    /// `add` for a goal built at runtime (`Box<dyn Goal>`).
    pub fn add_boxed(&mut self, priority: i32, goal: Box<dyn Goal>) {
        self.brain_starters += usize::from(goal.starts_every_tick());
        self.goals.push(Wrapped {
            priority,
            goal,
            running: false,
        });
    }
    /// `Goal::set_start_delay` of the goal of this name.
    pub fn set_start_delay(&mut self, name: &str, ticks: i32) {
        for g in &mut self.goals {
            if g.goal.name() == name {
                g.goal.set_start_delay(ticks);
            }
        }
    }
    pub fn clear(&mut self) {
        self.goals.clear();
        self.brain_starters = 0;
        self.locked = [None; 4];
        self.generation = self.generation.wrapping_add(1);
    }
    /// Generation counter bumped on [`Self::clear`].
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn running_names(&self) -> Vec<&'static str> {
        self.goals
            .iter()
            .filter(|g| g.running)
            .map(|g| g.goal.name())
            .collect()
    }
    /// Whether a goal of this name is registered, running or not.
    pub fn has(&self, name: &str) -> bool {
        self.goals.iter().any(|g| g.goal.name() == name)
    }
    pub fn is_running(&self, name: &str) -> bool {
        self.goals
            .iter()
            .any(|g| g.running && g.goal.name() == name)
    }
    fn stop(&mut self, i: usize, m: &mut Mob, cx: &mut Cx) {
        if self.goals[i].running {
            self.goals[i].running = false;
            self.goals[i].goal.stop(m, cx);
        }
    }
    /// GoalSelector.tick
    pub fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.prepare_tick(m, cx);
        self.tick_running(m, cx, true);
    }

    fn prepare_tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        for i in 0..self.goals.len() {
            if self.goals[i].running
                && (self.goals[i].goal.flags() & self.disabled != 0
                    || !self.goals[i].goal.can_continue(m, cx))
            {
                self.stop(i, m, cx);
            }
        }
        self.start_goals(m, cx, false);
    }

    /// The start half of `GoalSelector.tick`: each goal not running whose flags are free (or
    /// held by an interruptable goal of a lower priority) starts if it can. With
    /// `brain_only`, only the goals that stand in for brain behaviours
    /// ([`Goal::starts_every_tick`]), on a tick the selector otherwise only ticks its running
    /// goals.
    fn start_goals(&mut self, m: &mut Mob, cx: &mut Cx, brain_only: bool) {
        if brain_only && self.brain_starters == 0 {
            return;
        }
        for slot in &mut self.locked {
            if slot.is_some_and(|i| !self.goals[i].running) {
                *slot = None;
            }
        }
        for i in 0..self.goals.len() {
            let flags = self.goals[i].goal.flags();
            if self.goals[i].running
                || flags & self.disabled != 0
                || brain_only && !self.goals[i].goal.starts_every_tick()
            {
                continue;
            }
            let replaceable =
                (0..4)
                    .filter(|b| flags & (1 << b) != 0)
                    .all(|b| match self.locked[b] {
                        None => true,
                        Some(j) => {
                            self.goals[j].goal.interruptable()
                                && self.goals[i].priority < self.goals[j].priority
                        }
                    });
            if !replaceable || !self.goals[i].goal.can_use(m, cx) {
                continue;
            }
            for b in 0..4 {
                if flags & (1 << b) != 0 {
                    if let Some(j) = self.locked[b] {
                        self.stop(j, m, cx);
                    }
                    self.locked[b] = Some(i);
                }
            }
            if !self.goals[i].running {
                self.goals[i].running = true;
                self.goals[i].goal.start(m, cx);
            }
        }
    }
    pub fn tick_running(&mut self, m: &mut Mob, cx: &mut Cx, force: bool) {
        if !force {
            self.start_goals(m, cx, true);
        }
        for g in &mut self.goals {
            if g.running && (force || g.goal.every_tick()) {
                g.goal.tick(m, cx);
            }
        }
    }
    pub fn tick_until_pause(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        force: bool,
        breed: bool,
    ) -> Option<GoalPause> {
        if force {
            self.prepare_tick(m, cx);
        } else {
            self.start_goals(m, cx, true);
        }
        self.tick_from(m, cx, force, 0, breed)
    }

    fn tick_from(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        force: bool,
        start: usize,
        breed: bool,
    ) -> Option<GoalPause> {
        for (slot, g) in self.goals.iter_mut().enumerate().skip(start) {
            if !g.running || !(force || g.goal.every_tick()) {
                continue;
            }
            let before = cx.events.len();
            if let Some(movement) = g.goal.tick_until_movement_pause(m, cx) {
                return Some(GoalPause::Movement(GoalMovementPause {
                    movement,
                    slot,
                    force,
                }));
            }
            if breed
                && cx.events.len() > before
                && matches!(cx.events.last(), Some(MobEvent::Breed { .. }))
            {
                return Some(GoalPause::Breed(
                    Box::new(cx.events.pop().unwrap()),
                    GoalBreed {
                        next: slot + 1,
                        force,
                    },
                ));
            }
        }
        None
    }

    pub(super) fn resume_after_birth(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        paused: GoalBreed,
    ) -> Option<GoalPause> {
        self.tick_from(m, cx, paused.force, paused.next, true)
    }

    pub(super) fn resume_movement(
        &mut self,
        m: &mut Mob,
        cx: &mut Cx,
        paused: GoalMovementPause,
        breed: bool,
    ) -> Option<GoalPause> {
        if let ControlFlow::Break(movement) = paused.movement.resume(m, cx.w, cx.level, cx.events) {
            return Some(GoalPause::Movement(GoalMovementPause {
                movement,
                ..paused
            }));
        }
        if m.removed {
            return None;
        }
        // An owning-host listener can remove or shift the running goal. Only
        // the still-resident goal holding this pause can finish its path update.
        let next = self
            .goals
            .iter_mut()
            .position(|g| g.goal.after_movement(m, cx))
            .map_or(paused.slot, |slot| slot + 1);
        self.tick_from(m, cx, paused.force, next, breed)
    }

    pub fn stop_all(&mut self, m: &mut Mob, cx: &mut Cx) {
        for i in 0..self.goals.len() {
            self.stop(i, m, cx);
        }
        self.locked = [None; 4];
    }
    /// `GoalSelector.removeAllGoals(predicate)`: every goal `predicate`
    /// picks goes, a running one stopped first (`removeGoal` stops it), and
    /// the flag locks it held move with the list. How many went.
    pub fn remove_all(
        &mut self,
        predicate: impl Fn(&dyn Goal) -> bool,
        m: &mut Mob,
        cx: &mut Cx,
    ) -> usize {
        let mut removed = 0;
        let mut i = 0;
        while i < self.goals.len() {
            if predicate(&*self.goals[i].goal) {
                self.stop(i, m, cx);
                let goal = self.goals.remove(i);
                self.brain_starters -= usize::from(goal.goal.starts_every_tick());
                removed += 1;
                for slot in &mut self.locked {
                    match *slot {
                        Some(j) if j == i => *slot = None,
                        Some(j) if j > i => *slot = Some(j - 1),
                        _ => {}
                    }
                }
            } else {
                i += 1;
            }
        }
        removed
    }
    /// Every goal as (priority, name, running), in registration order.
    pub fn entries(&self) -> impl Iterator<Item = (i32, &'static str, bool)> + '_ {
        self.goals
            .iter()
            .map(|g| (g.priority, g.goal.name(), g.running))
    }
}

// --- random positions (DefaultRandomPos / LandRandomPos / RandomPos) -------

/// RandomPos.moveUpToAboveSolid: out of the solid blocks, then up to
/// `above` more blocks of air.
fn move_up_to_above_solid(w: &MobWorld, mut p: [i32; 3], above: i32) -> [i32; 3] {
    if !w.solid(p[0], p[1], p[2]) {
        return p;
    }
    p[1] += 1;
    while p[1] < w.world.top_y() && w.solid(p[0], p[1], p[2]) {
        p[1] += 1;
    }
    let first = p[1];
    while p[1] < w.world.top_y() && p[1] - first < above {
        p[1] += 1;
        if w.solid(p[0], p[1], p[2]) {
            p[1] -= 1;
            break;
        }
    }
    p
}
/// `Mob.isWithinHome(BlockPos)`: `Vec3i.distSqr` in doubles, so no
/// coordinate pair overflows, against the radius squared as a Java `int`
/// (which wraps past 46 340, as vanilla's does).
pub fn within_home(c: [i32; 3], r: i32, p: [i32; 3]) -> bool {
    let d = |i: usize| f64::from(c[i]) - f64::from(p[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < f64::from(r.wrapping_mul(r))
}
