// Included in villager_brain.rs to retain its private scope.
/// The fixed behaviours whose only state is their running flag (OneShots
/// stop in the same tick).
#[derive(Clone, Debug)]
enum B {
    Swim(Timed),
    InteractWithDoor {
        running: bool,
        last_node: Option<PathNode>,
        cooldown: i32,
    },
    LookAtTargetSink(Timed),
    PanicTrigger(Timed),
    WakeUp(bool),
    ReactToBell(bool),
    SetRaidStatus(bool),
    ValidatePoi {
        running: bool,
        filter: PoiFilter,
        which: PoiMem,
    },
    MoveToTargetSink {
        t: Timed,
        cooldown: i32,
        path: Option<Path>,
        last_target: Option<[i32; 3]>,
        speed: f32,
    },
    PoiCompetitorScan(bool),
    FollowTradingPlayer(Timed),
    GoToWantedItem(bool),
    AcquirePoi {
        running: bool,
        filter: PoiFilter,
        /// The memory filled (and checked absent): job site, home, meeting.
        poi: PoiFilter,
        only_adult: bool,
        event: bool,
        next: i64,
        retries: Vec<([i32; 3], Retry)>,
    },
    GoToPotentialJobSite(Timed),
    YieldJobSite(bool),
    AssignProfession(bool),
    ResetProfession(bool),
    Gate(Box<Gate>),
    InteractWith {
        running: bool,
        kind: Interact,
    },
    VillageBoundStroll {
        running: bool,
        speed: f32,
        xz: i32,
        y: i32,
    },
    WalkToLookTarget(bool),
    JumpOnBed {
        t: Timed,
        bed: Option<[i32; 3]>,
        time_left: i32,
        jumps: i32,
        cooldown: i32,
    },
    DoNothing(Timed),
    GiveGiftToHero {
        t: Timed,
        until_next: i32,
        given: bool,
        started: i64,
    },
    SetLookAndInteract(bool),
    ShowTrades {
        t: Timed,
        look_time: i32,
        /// `playerItemStack` (outer `None`: null) and `displayItems`,
        /// `cycleCounter`, `displayIndex`.
        player_item: Option<Option<std::sync::Arc<str>>>,
        items: Vec<crate::items::ItemStack>,
        cycle: i32,
        index: usize,
    },
    TradeWithVillager(Timed),
    MakeLove {
        t: Timed,
        birth: i64,
    },
    LookAtEntity {
        running: bool,
        what: LookAt,
    },
    UpdateActivity(bool),
    /// A mod's behavior (#824), wrapped by `mod_behaviors`.
    Mod(super::mod_behaviors::ModBehavior),
    WorkAtPoi {
        t: Timed,
        last_check: i64,
        /// `WorkAtComposter` (a farmer's package) rather than `WorkAtPoi`.
        composter: bool,
    },
    StrollAroundPoi {
        running: bool,
        next_ok: i64,
        max_dist: i32,
        which: PoiMem,
    },
    StrollToPoi {
        running: bool,
        next_ok: i64,
    },
    StrollToPoiList {
        running: bool,
        next_ok: i64,
    },
    HarvestFarmland {
        t: Timed,
        next_ok: i64,
        target: Option<[i32; 3]>,
        valid: Vec<[i32; 3]>,
        worked: i32,
    },
    UseBonemeal {
        t: Timed,
        next_work: i64,
        last_session: i64,
        worked: i32,
        crop: Option<[i32; 3]>,
        /// The next crop is picked on the next tick, when the level shows the
        /// age the meal gave (vanilla grows the crop at once).
        repick: bool,
    },
    WalkToBlockMemory {
        running: bool,
        which: PoiMem,
        speed: f32,
        close: i32,
        too_far: i32,
        too_long: i64,
    },
    SleepInBed {
        t: Timed,
        next_ok: i64,
    },
    ClosestHomeAsWalkTarget {
        running: bool,
        last_update: i64,
        cache: Vec<([i32; 3], i64)>,
    },
    InsideBrownianWalk {
        running: bool,
        random: LegacyRandom,
    },
    GoToClosestVillage(bool),
    TriggerGate {
        running: bool,
        random: LegacyRandom,
        kids: Vec<(B, i32, f64)>,
    },
    SocializeAtBell(bool),
    CalmDown(bool),
    WalkAwayFrom {
        running: bool,
        hostile: bool,
    },
    RingBell(bool),
    ResetRaidStatus(bool),
    /// `BehaviorBuilder.sequence(triggerIf(raid condition), then)`.
    RaidSequence {
        running: bool,
        victory: bool,
        then: Box<B>,
    },
    MoveToSkySeeingSpot {
        running: bool,
        speed: f32,
    },
    Celebrate(Timed),
    LocateHidingPlace {
        running: bool,
        radius: i32,
        speed: f32,
        close: i32,
    },
    SetHiddenState {
        running: bool,
        hidden: i32,
    },
    PlayTag(bool),
}
/// `AcquirePoi.JitteredLinearRetry`.
#[derive(Clone, Copy, Debug)]
struct Retry {
    previous: i64,
    next: i64,
    delay: i32,
}
impl Retry {
    fn new(cx: &mut Bx, now: i64) -> Self {
        let mut r = Retry {
            previous: 0,
            next: 0,
            delay: 0,
        };
        r.mark(cx, now);
        r
    }
    fn mark(&mut self, cx: &mut Bx, now: i64) {
        self.previous = now;
        let suggested = self.delay + cx.level.random().next_int_bound(40).unwrap_or(0) + 40;
        self.delay = suggested.min(400);
        self.next = now + i64::from(self.delay);
    }
}

fn one(f: impl FnOnce() -> bool, running: &mut bool) -> bool {
    let r = f();
    if r {
        *running = true;
    }
    r
}

impl B {
    fn running(&self) -> bool {
        match self {
            B::Swim(t)
            | B::LookAtTargetSink(t)
            | B::PanicTrigger(t)
            | B::FollowTradingPlayer(t)
            | B::GoToPotentialJobSite(t)
            | B::DoNothing(t)
            | B::TradeWithVillager(t)
            | B::Celebrate(t) => t.running,
            B::MoveToTargetSink { t, .. }
            | B::GiveGiftToHero { t, .. }
            | B::ShowTrades { t, .. }
            | B::WorkAtPoi { t, .. }
            | B::HarvestFarmland { t, .. }
            | B::UseBonemeal { t, .. }
            | B::JumpOnBed { t, .. }
            | B::MakeLove { t, .. }
            | B::SleepInBed { t, .. } => t.running,
            B::InteractWithDoor { running, .. }
            | B::ValidatePoi { running, .. }
            | B::AcquirePoi { running, .. }
            | B::InteractWith { running, .. }
            | B::VillageBoundStroll { running, .. }
            | B::LookAtEntity { running, .. }
            | B::StrollAroundPoi { running, .. }
            | B::StrollToPoi { running, .. }
            | B::StrollToPoiList { running, .. }
            | B::WalkToBlockMemory { running, .. }
            | B::ClosestHomeAsWalkTarget { running, .. }
            | B::InsideBrownianWalk { running, .. }
            | B::TriggerGate { running, .. }
            | B::WalkAwayFrom { running, .. }
            | B::RaidSequence { running, .. }
            | B::MoveToSkySeeingSpot { running, .. }
            | B::LocateHidingPlace { running, .. }
            | B::SetHiddenState { running, .. } => *running,
            B::WakeUp(r)
            | B::ReactToBell(r)
            | B::SetRaidStatus(r)
            | B::PoiCompetitorScan(r)
            | B::GoToWantedItem(r)
            | B::YieldJobSite(r)
            | B::AssignProfession(r)
            | B::ResetProfession(r)
            | B::WalkToLookTarget(r)
            | B::SetLookAndInteract(r)
            | B::UpdateActivity(r)
            | B::GoToClosestVillage(r)
            | B::SocializeAtBell(r)
            | B::CalmDown(r)
            | B::RingBell(r)
            | B::ResetRaidStatus(r)
            | B::PlayTag(r) => *r,
            B::Mod(e) => e.running(),
            B::Gate(g) => g.running,
        }
    }
    /// `OneShot` behaviours stop at their first tick.
    fn one_shot_stop(&mut self) -> bool {
        match self {
            B::InteractWithDoor { running, .. }
            | B::ValidatePoi { running, .. }
            | B::AcquirePoi { running, .. }
            | B::InteractWith { running, .. }
            | B::VillageBoundStroll { running, .. }
            | B::LookAtEntity { running, .. }
            | B::StrollAroundPoi { running, .. }
            | B::StrollToPoi { running, .. }
            | B::StrollToPoiList { running, .. }
            | B::WalkToBlockMemory { running, .. }
            | B::ClosestHomeAsWalkTarget { running, .. }
            | B::InsideBrownianWalk { running, .. }
            | B::TriggerGate { running, .. }
            | B::WalkAwayFrom { running, .. }
            | B::RaidSequence { running, .. }
            | B::MoveToSkySeeingSpot { running, .. }
            | B::LocateHidingPlace { running, .. }
            | B::SetHiddenState { running, .. }
            | B::GoToClosestVillage(running)
            | B::SocializeAtBell(running)
            | B::CalmDown(running)
            | B::RingBell(running)
            | B::ResetRaidStatus(running)
            | B::PlayTag(running)
            | B::WakeUp(running)
            | B::ReactToBell(running)
            | B::SetRaidStatus(running)
            | B::PoiCompetitorScan(running)
            | B::GoToWantedItem(running)
            | B::YieldJobSite(running)
            | B::AssignProfession(running)
            | B::ResetProfession(running)
            | B::WalkToLookTarget(running)
            | B::SetLookAndInteract(running)
            | B::UpdateActivity(running) => {
                *running = false;
                true
            }
            _ => false,
        }
    }

    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        let now = cx.now;
        match self {
            B::Gate(g) => g.try_start(m, cx),
            B::Mod(e) => e.try_start(m, cx.w, cx.level.random(), cx.now),
            // -- Behaviours with a duration.
            B::Swim(t) => {
                if !swim_condition(m) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                true
            }
            B::LookAtTargetSink(t) => {
                if !mem(m).look_target.present() {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                true
            }
            B::PanicTrigger(t) => {
                t.begin(cx.level.random(), cx.now);
                // VillagerPanicTrigger.start
                if panic_condition(m) {
                    if !brain_mut(m).is_active(Act::Panic) {
                        let mm = mem(m);
                        mm.path.clear();
                        mm.walk_target.clear();
                        mm.look_target.clear();
                        mm.breed_target.clear();
                        mm.interaction_target.clear();
                    }
                    brain_mut(m).set_active_if_possible(Act::Panic);
                }
                true
            }
            B::MoveToTargetSink {
                t,
                cooldown,
                path,
                last_target,
                speed,
            } => {
                let mm = mem(m);
                if mm.path.present() || !mm.walk_target.present() {
                    return false;
                }
                // checkExtraStartConditions
                if *cooldown > 0 {
                    *cooldown -= 1;
                    return false;
                }
                let walk = mm.walk_target.value().unwrap();
                let reached = reached(m, cx.w, &walk);
                if !reached && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach) {
                    *last_target = walk.target.block(cx.w);
                    t.begin(cx.level.random(), cx.now);
                    // start
                    mem(m).path.set_opt(path.clone());
                    m.nav_move_to_path(cx.w, path.clone(), f64::from(*speed));
                    return true;
                }
                let mm = mem(m);
                mm.walk_target.clear();
                if reached {
                    mm.cant_reach_walk_target_since.clear();
                }
                false
            }
            B::FollowTradingPlayer(t) => {
                if !follow_trading_condition(m, cx.w) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                follow_trading_player(m);
                true
            }
            B::GoToPotentialJobSite(t) => {
                if !mem(m).potential_job_site.present() {
                    return false;
                }
                let a = brain_mut(m).activity;
                if !matches!(a, Act::Idle | Act::Work | Act::Play) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                true
            }
            B::JumpOnBed {
                t,
                bed,
                time_left,
                jumps,
                cooldown,
            } => {
                let mm = mem(m);
                let Some(nearest) = mm.nearest_bed.value() else {
                    return false;
                };
                if mm.walk_target.present() || !m.is_baby() {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                *bed = Some(nearest);
                *time_left = 100;
                *jumps = 3 + cx.level.random().next_int_bound(4).unwrap_or(0);
                *cooldown = 0;
                mem(m).walk_target.set(WalkTarget::block(nearest, 0.5, 0));
                true
            }
            B::DoNothing(t) => {
                t.begin(cx.level.random(), cx.now);
                true
            }
            B::GiveGiftToHero {
                t,
                until_next,
                given,
                started,
            } => {
                if !mem(m).nearest_visible_player.present() {
                    return false;
                }
                if !hero_visible(m, cx.w) {
                    return false;
                }
                if *until_next > 0 {
                    *until_next -= 1;
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                *given = false;
                *started = now;
                let mm = mem(m);
                mm.interaction_target.set(Who::Player);
                mm.look_target.set(Tracker::Entity(Who::Player, true));
                true
            }
            B::ShowTrades {
                t,
                look_time,
                cycle,
                index,
                ..
            } => {
                if !mem(m).interaction_target.present() || !show_trades_condition(m, cx.w) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                // start
                if let Some(target) = mem(m).interaction_target.value() {
                    mem(m).look_target.set(Tracker::Entity(target, true));
                }
                *cycle = 0;
                *index = 0;
                *look_time = 40;
                true
            }
            B::TradeWithVillager(t) => {
                let mm = mem(m);
                if !mm.interaction_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                if !villager_target_valid(m, cx.w, Mem::InteractionTarget) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                let target = mem(m).interaction_target.value().unwrap();
                lock_gaze(m, target, cx);
                true
            }
            B::MakeLove { t, birth } => {
                let mm = mem(m);
                if !mm.breed_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                if !breeding_possible(m, cx.w) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                let target = mem(m).breed_target.value().unwrap();
                lock_gaze(m, target, cx);
                *birth = now + 275 + i64::from(m.random.next_int_bound(50).unwrap_or(0));
                true
            }
            B::WorkAtPoi {
                t,
                last_check,
                composter,
            } => {
                let mm = mem(m);
                let Some(site) = mm.job_site.value() else {
                    return false;
                };
                if now - *last_check < 300 {
                    return false;
                }
                if cx.level.random().next_int_bound(2).unwrap_or(0) != 0 {
                    return false;
                }
                *last_check = now;
                if !closer_to_center(site, m.position, 1.73) {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                // start
                let mm = mem(m);
                mm.last_worked_at_poi.set(now);
                mm.look_target.set(Tracker::Block(site));
                play_work_sound(m, cx);
                // WorkAtComposter.useWorkstation, before the restock.
                if *composter {
                    cx.events.push(MobEvent::WorkAtComposter {
                        mob: m.id,
                        pos: site,
                    });
                }
                restock_if_due(m, cx.w, cx.events);
                true
            }
            B::HarvestFarmland {
                t,
                next_ok,
                target,
                valid,
                ..
            } => {
                let mm = mem(m);
                if mm.look_target.present()
                    || mm.walk_target.present()
                    || !mm.secondary_job_site.present()
                {
                    return false;
                }
                if !cx.w.can_grief(m.id) || !super::villager::is_farmer(m) {
                    return false;
                }
                *valid = farmland_around(m, cx.w);
                *target = pick_farmland(valid, cx);
                let Some(p) = *target else { return false };
                t.begin(cx.level.random(), cx.now);
                if now > *next_ok {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Block(p));
                    mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
                }
                true
            }
            B::UseBonemeal {
                t,
                next_work,
                last_session,
                worked,
                crop,
                repick,
            } => {
                let mm = mem(m);
                if mm.look_target.present() || mm.walk_target.present() {
                    return false;
                }
                // Checked every 10 ticks, and not within 160 ticks of the last session.
                let age = i64::from(m.age);
                if age % 10 != 0 || (*last_session != 0 && *last_session + 160 > age) {
                    return false;
                }
                if super::villager::ext(m).is_none_or(|e| e.count_item("minecraft:bone_meal") == 0)
                {
                    return false;
                }
                *crop = pick_bonemeal_crop(m, cx);
                if crop.is_none() {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                bonemeal_target(m, *crop);
                m.extra.set_main_hand(Some("minecraft:bone_meal".into()));
                *next_work = cx.now;
                *worked = 0;
                *repick = false;
                true
            }
            // -- OneShots.
            B::InteractWithDoor {
                running,
                last_node,
                cooldown,
            } => {
                if !mem(m).path.present() {
                    return false;
                }
                let Some(path) = m.ai.nav.path.clone() else {
                    return false;
                };
                if path.next == 0 || path.is_done() {
                    return false;
                }
                let next = path.next_node();
                if *last_node == Some(next) {
                    *cooldown = 20;
                } else {
                    *cooldown -= 1;
                    if *cooldown > 0 {
                        return false;
                    }
                }
                *last_node = Some(next);
                let from = path.nodes[path.next - 1];
                interact_with_doors(m, cx, from, next);
                *running = true;
                true
            }
            B::WakeUp(r) => one(
                || {
                    if !brain_mut(m).is_active(Act::Rest) && super::villager::sleeping(m) {
                        super::villager::stop_sleeping(m, cx.w, cx.events);
                        true
                    } else {
                        false
                    }
                },
                r,
            ),
            B::ReactToBell(r) => one(
                || {
                    if !mem(m).heard_bell_time.present() {
                        return false;
                    }
                    if !super::villager::raid_nearby(m) {
                        brain_mut(m).set_active_if_possible(Act::Hide);
                    }
                    true
                },
                r,
            ),
            B::SetRaidStatus(r) => one(
                || {
                    if cx.level.random().next_int_bound(20).unwrap_or(0) != 0 {
                        return false;
                    }
                    match super::villager::raid_state(m) {
                        Some(true) => {
                            let b = brain_mut(m);
                            b.default_activity = Act::Raid;
                            b.set_active_if_possible(Act::Raid);
                        }
                        Some(false) => {
                            let b = brain_mut(m);
                            b.default_activity = Act::PreRaid;
                            b.set_active_if_possible(Act::PreRaid);
                        }
                        None => {}
                    }
                    true
                },
                r,
            ),
            B::ValidatePoi {
                running,
                filter,
                which,
            } => validate_poi(m, cx, *filter, which, running),
            B::PoiCompetitorScan(r) => one(|| scan_job_competitors(m, cx), r),
            B::GoToWantedItem(r) => one(
                || {
                    let mm = mem(m);
                    if mm.walk_target.present() {
                        return false;
                    }
                    let Some(at) = mm.nearest_wanted_item.value() else {
                        return false;
                    };
                    // item.closerThan(body, 4); a villager can pick up loot.
                    if m.position.distance_squared(Vec3::new(at[0], at[1], at[2])) >= 16.0 {
                        return false;
                    }
                    let mm = mem(m);
                    // The item's eye height is 0.2125.
                    mm.look_target
                        .set(Tracker::Point([at[0], at[1] + 0.2125, at[2]]));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Point(at),
                        speed: 0.5,
                        close: 0,
                    });
                    true
                },
                r,
            ),
            B::AcquirePoi {
                running,
                filter,
                poi,
                only_adult,
                event,
                next,
                retries,
            } => {
                let mm = mem(m);
                let (target_present, validate_present) = match poi {
                    PoiFilter::Acquirable => {
                        (mm.potential_job_site.present(), mm.job_site.present())
                    }
                    PoiFilter::Home => (mm.home.present(), false),
                    PoiFilter::Meeting => (mm.meeting_point.present(), false),
                    PoiFilter::Held => unreachable!(),
                };
                if target_present || validate_present {
                    return false;
                }
                if *only_adult && m.is_baby() {
                    return false;
                }
                if *next == 0 {
                    *next = cx.w.game_time
                        + i64::from(cx.level.random().next_int_bound(20).unwrap_or(0));
                    return false;
                }
                if cx.w.game_time < *next {
                    return false;
                }
                *next = now + 20 + i64::from(cx.level.random().next_int_bound(20).unwrap_or(0));
                retries.retain(|(_, r)| now - r.previous < 400);
                let w = cx.w;
                let mut found = pois_in_range(m, w, *filter, 48, |_| true);
                // The cache test runs on every candidate, in scan order.
                let mut keep = Vec::new();
                {
                    let order = pois_scan_order(m, &found);
                    for idx in order {
                        let p = found[idx].1;
                        let ok = match retries.iter_mut().find(|(q, _)| *q == p) {
                            None => true,
                            Some((_, r)) => {
                                if now >= r.next {
                                    r.mark(cx, now);
                                    true
                                } else {
                                    false
                                }
                            }
                        };
                        if ok {
                            keep.push(idx);
                        }
                    }
                }
                found = found
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| keep.contains(i))
                    .map(|(_, f)| f)
                    .collect();
                found.truncate(5);
                if *poi == PoiFilter::Home {
                    found.retain(|(_, p)| w.prop(p[0], p[1], p[2], "occupied") != Some("true"));
                }
                let path = if found.is_empty() {
                    None
                } else {
                    let range = found
                        .iter()
                        .map(|(t, _)| t.valid_range())
                        .max()
                        .unwrap_or(1)
                        .max(1);
                    let targets: Vec<[i32; 3]> = found.iter().map(|(_, p)| *p).collect();
                    m.create_path_to_any(w, &targets, range)
                };
                if let Some(path) = path.filter(|p| p.reached) {
                    let target = path.target;
                    if poi_at(w, target[0], target[1], target[2]).is_some() {
                        let mm = mem(m);
                        match poi {
                            PoiFilter::Acquirable => mm.potential_job_site.set(target),
                            PoiFilter::Home => mm.home.set(target),
                            PoiFilter::Meeting => mm.meeting_point.set(target),
                            PoiFilter::Held => unreachable!(),
                        }
                        if *event {
                            happy_particles(m, cx);
                        }
                        retries.clear();
                    }
                } else {
                    for (_, p) in &found {
                        if !retries.iter().any(|(q, _)| q == p) {
                            let r = Retry::new(cx, now);
                            retries.push((*p, r));
                        }
                    }
                }
                *running = true;
                true
            }
            B::YieldJobSite(r) => one(|| yield_job_site(m, cx), r),
            B::AssignProfession(r) => assign_profession(m, cx, r),
            B::ResetProfession(r) => one(|| reset_profession(m, cx), r),
            B::InteractWith { running, kind } => {
                let mm = mem(m);
                if mm.walk_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let kind = *kind;
                if kind == Interact::BreedVillager && !super::villager::can_breed_in(m, cx.w) {
                    return false;
                }
                let w = cx.w;
                let valid = move |who: Who| -> bool {
                    let Who::Mob(id) = who else { return false };
                    let Some(s) = w.seen(id) else { return false };
                    match kind {
                        Interact::Villager => s.species == Species::Villager,
                        Interact::BreedVillager => s.species == Species::Villager && s.in_love,
                        Interact::Cat => s.species == Species::Cat,
                    }
                };
                if find_closest(m, w, |_, who| valid(who)).is_none() {
                    return false;
                }
                if let Some(t) = find_closest(m, w, |m, who| {
                    valid(who)
                        && w.target_info(who)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 64.0)
                }) {
                    let mm = mem(m);
                    if kind == Interact::BreedVillager {
                        mm.breed_target.set(t);
                    } else {
                        mm.interaction_target.set(t);
                    }
                    mm.look_target.set(Tracker::Entity(t, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(t, false),
                        speed: 0.5,
                        close: 2,
                    });
                }
                *running = true;
                true
            }
            B::VillageBoundStroll {
                running,
                speed,
                xz,
                y,
            } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                let section = [here[0] >> 4, here[1] >> 4, here[2] >> 4];
                let villages = village_sections(m, w);
                let pos = if sections_to_village(&villages, section) <= 1 {
                    m.land_random_pos(w, *xz, *y)
                } else {
                    let best = closest_section(&villages, section, 2);
                    if best != section {
                        let to = [
                            f64::from(best[0] * 16 + 8) + 0.5,
                            f64::from(best[1] * 16 + 8),
                            f64::from(best[2] * 16 + 8) + 0.5,
                        ];
                        m.default_random_pos_towards(
                            w,
                            *xz,
                            *y,
                            to,
                            f64::from(std::f32::consts::FRAC_PI_2),
                        )
                    } else {
                        m.land_random_pos(w, *xz, *y)
                    }
                };
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, *speed, 0)));
                *running = true;
                true
            }
            B::WalkToLookTarget(r) => one(
                || {
                    let mm = mem(m);
                    if mm.walk_target.present() {
                        return false;
                    }
                    let Some(look) = mm.look_target.value() else {
                        return false;
                    };
                    mm.walk_target.set(WalkTarget {
                        target: look,
                        speed: 0.5,
                        close: 2,
                    });
                    true
                },
                r,
            ),
            B::SetLookAndInteract(r) => one(
                || {
                    let mm = mem(m);
                    if mm.interaction_target.present() || !mm.visible_mobs.present() {
                        return false;
                    }
                    let w = cx.w;
                    let found = find_closest(m, w, |m, who| {
                        who == Who::Player
                            && w.target_info(who)
                                .is_some_and(|t| t.position.distance_squared(m.position) <= 16.0)
                    });
                    let Some(t) = found else { return false };
                    let mm = mem(m);
                    mm.interaction_target.set(t);
                    mm.look_target.set(Tracker::Entity(t, true));
                    true
                },
                r,
            ),
            B::LookAtEntity { running, what } => {
                let mm = mem(m);
                if mm.look_target.present() || !mm.visible_mobs.present() {
                    return false;
                }
                let what = *what;
                let w = cx.w;
                let found = find_closest(m, w, |m, who| {
                    let ok = match (what, who) {
                        (LookAt::Player, Who::Player) => true,
                        (LookAt::Species(sp), Who::Mob(id)) => {
                            w.seen(id).is_some_and(|s| s.species == sp)
                        }
                        (LookAt::Category(c), Who::Mob(id)) => {
                            w.seen(id).is_some_and(|s| s.category == c)
                        }
                        _ => false,
                    };
                    ok && w
                        .target_info(who)
                        .is_some_and(|t| t.position.distance_squared(m.position) <= 64.0)
                });
                let Some(t) = found else { return false };
                mem(m).look_target.set(Tracker::Entity(t, true));
                *running = true;
                true
            }
            B::UpdateActivity(r) => {
                brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
                *r = true;
                true
            }
            B::StrollAroundPoi {
                running,
                next_ok,
                max_dist,
                which,
            } => {
                let Some(site) = which.get(m) else {
                    return false;
                };
                if !closer_to_center(site, m.position, f64::from(*max_dist)) {
                    return false;
                }
                if now <= *next_ok {
                    *running = true;
                    return true;
                }
                let pos = m.land_random_pos(cx.w, 8, 6);
                mem(m)
                    .walk_target
                    .set_opt(pos.map(|p| WalkTarget::point(p, 0.4, 1)));
                *next_ok = now + 180;
                *running = true;
                true
            }
            B::StrollToPoi { running, next_ok } => {
                let Some(site) = mem(m).job_site.value() else {
                    return false;
                };
                if !closer_to_center(site, m.position, 10.0) {
                    return false;
                }
                if now <= *next_ok {
                    *running = true;
                    return true;
                }
                mem(m).walk_target.set(WalkTarget::block(site, 0.4, 1));
                *next_ok = now + 80;
                *running = true;
                true
            }
            B::StrollToPoiList { running, next_ok } => {
                let mm = mem(m);
                let (Some(list), Some(site)) = (mm.secondary_job_site.value(), mm.job_site.value())
                else {
                    return false;
                };
                if list.is_empty() {
                    return false;
                }
                let target = list[cx
                    .level
                    .random()
                    .next_int_bound(list.len() as i32)
                    .unwrap_or(0) as usize];
                if !closer_to_center(site, m.position, 6.0) {
                    return false;
                }
                if now > *next_ok {
                    mem(m).walk_target.set(WalkTarget::block(target, 0.5, 1));
                    *next_ok = now + 100;
                }
                *running = true;
                true
            }
            B::WalkToBlockMemory {
                running,
                which,
                speed,
                close,
                too_far,
                too_long,
            } => {
                let r = walk_to_block_memory(m, cx, *which, *speed, *close, *too_far, *too_long);
                *running = r;
                r
            }
            B::SleepInBed { t, next_ok } => {
                if m.extra.mount.is_some() {
                    return false;
                }
                let mm = mem(m);
                let Some(home) = mm.home.value() else {
                    return false;
                };
                if let Some(woken) = mm.last_woken.value() {
                    let since = cx.w.game_time - woken;
                    if since > 0 && since < 100 {
                        return false;
                    }
                }
                let w = cx.w;
                if !(closer_to_center(home, m.position, 2.0)
                    && w.block_name(home[0], home[1], home[2]).ends_with("_bed")
                    && w.prop(home[0], home[1], home[2], "occupied") != Some("true"))
                {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                if now > *next_ok {
                    close_doors(m, cx, None, None);
                    if super::villager::start_sleeping(m, home, w, cx.events) {
                        mem(m).last_slept.set(now);
                    }
                    let mm = mem(m);
                    mm.walk_target.clear();
                    mm.cant_reach_walk_target_since.clear();
                }
                true
            }
            B::ClosestHomeAsWalkTarget {
                running,
                last_update,
                cache,
            } => {
                let mm = mem(m);
                if mm.walk_target.present() || mm.home.present() {
                    return false;
                }
                if cx.w.game_time - *last_update < 20 {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                let homes = homes_in_range(m, w, 48);
                let Some(closest) = homes.iter().min_by_key(|p| dist_sqr(**p, here)).copied()
                else {
                    return false;
                };
                if dist_sqr(closest, here) <= 4 {
                    return false;
                }
                let mut tried = 0;
                *last_update =
                    cx.w.game_time + i64::from(cx.level.random().next_int_bound(20).unwrap_or(0));
                let mut targets = Vec::new();
                for p in homes {
                    if cache.iter().any(|(q, _)| *q == p) {
                        continue;
                    }
                    tried += 1;
                    if tried >= 5 {
                        continue;
                    }
                    cache.push((p, *last_update + 40));
                    targets.push(p);
                }
                let path = if targets.is_empty() {
                    None
                } else {
                    m.create_path_to_any(w, &targets, 1)
                };
                if let Some(path) = path.filter(|p| p.reached) {
                    mem(m)
                        .walk_target
                        .set(WalkTarget::block(path.target, 0.5, 1));
                } else if tried < 5 {
                    let limit = *last_update;
                    cache.retain(|(_, until)| *until >= limit);
                }
                *running = true;
                true
            }
            B::InsideBrownianWalk { running, random } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                if w.can_see_sky(here[0], here[1], here[2]) {
                    return false;
                }
                let mut poses = Vec::new();
                for z in -1..=1 {
                    for y in -1..=1 {
                        for x in -1..=1 {
                            poses.push([here[0] + x, here[1] + y, here[2] + z]);
                        }
                    }
                }
                // Collections.shuffle (its own random).
                for i in (1..poses.len()).rev() {
                    let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                    poses.swap(i, j);
                }
                if let Some(p) = poses
                    .into_iter()
                    .find(|p| !w.can_see_sky(p[0], p[1], p[2]) && w.solid(p[0], p[1], p[2]))
                {
                    mem(m).walk_target.set(WalkTarget::block(p, 0.5, 0));
                }
                *running = true;
                true
            }
            B::GoToClosestVillage(r) => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let villages = village_sections(m, w);
                let section_of = |p: [i32; 3]| [p[0] >> 4, p[1] >> 4, p[2] >> 4];
                let here = sections_to_village(&villages, section_of(block_of(m.position)));
                if here <= 1 {
                    return false;
                }
                let mut target = None;
                for _ in 0..5 {
                    let pos = m.land_random_pos_scored(w, 15, 7, |p| {
                        -f64::from(sections_to_village(&villages, section_of(p)))
                    });
                    if let Some(p) = pos {
                        let b = [
                            p[0].floor() as i32,
                            p[1].floor() as i32,
                            p[2].floor() as i32,
                        ];
                        let d = sections_to_village(&villages, section_of(b));
                        if d < here {
                            target = Some(p);
                            break;
                        }
                        if d == here {
                            target = Some(p);
                        }
                    }
                }
                if let Some(p) = target {
                    mem(m).walk_target.set(WalkTarget::point(p, 0.5, 4));
                }
                *r = true;
                true
            }
            B::TriggerGate {
                running,
                random,
                kids,
            } => {
                for k in kids.iter_mut() {
                    let r = random.next_float();
                    k.2 = -sunrise_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
                }
                kids.sort_by(|a, b| a.2.total_cmp(&b.2));
                for k in kids.iter_mut() {
                    let fired = k.0.try_start(m, cx);
                    k.0.one_shot_stop();
                    if fired {
                        break;
                    }
                }
                *running = true;
                true
            }
            B::SocializeAtBell(r) => one(
                || {
                    let mm = mem(m);
                    let Some(bell) = mm.meeting_point.value() else {
                        return false;
                    };
                    if !mm.visible_mobs.present() || mm.interaction_target.present() {
                        return false;
                    }
                    let w = cx.w;
                    let is_villager = |who: Who| matches!(who, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Villager));
                    if cx.level.random().next_int_bound(100).unwrap_or(1) != 0
                        || !closer_to_center(bell, m.position, 4.0)
                        || find_closest(m, w, |_, who| is_villager(who)).is_none()
                    {
                        return false;
                    }
                    if let Some(t) = find_closest(m, w, |m, who| {
                        is_villager(who)
                            && w.target_info(who)
                                .is_some_and(|i| i.position.distance_squared(m.position) <= 32.0)
                    }) {
                        let mm = mem(m);
                        mm.interaction_target.set(t);
                        mm.look_target.set(Tracker::Entity(t, true));
                        mm.walk_target.set(WalkTarget {
                            target: Tracker::Entity(t, false),
                            speed: 0.3,
                            close: 1,
                        });
                    }
                    true
                },
                r,
            ),
            B::CalmDown(r) => {
                let w = cx.w;
                let mm = mem(m);
                let scared = mm.hurt_by.present()
                    || mm.nearest_hostile.present()
                    || mm.hurt_by_entity.value().is_some_and(|e| {
                        w.target_info(e)
                            .is_some_and(|t| t.position.distance_squared(m.position) <= 36.0)
                    });
                if !scared {
                    let mm = mem(m);
                    mm.hurt_by.clear();
                    mm.hurt_by_entity.clear();
                    brain_mut(m).update_from_schedule(w.game_time, w.day_time);
                }
                *r = true;
                true
            }
            B::WalkAwayFrom { running, hostile } => {
                let w = cx.w;
                let mm = mem(m);
                let from = if *hostile {
                    mm.nearest_hostile.value()
                } else {
                    mm.hurt_by_entity.value()
                };
                let Some(from) = from else { return false };
                if mm.walk_target.present() {
                    return false;
                }
                let Some(avoid) = w.target_info(from).map(|t| t.position) else {
                    return false;
                };
                if m.position.distance_squared(avoid) >= 36.0 {
                    return false;
                }
                for _ in 0..10 {
                    if let Some(p) = m.land_random_pos_away(w, 16, 7, [avoid.x, avoid.y, avoid.z]) {
                        mem(m).walk_target.set(WalkTarget::point(p, 0.75, 0));
                        break;
                    }
                }
                *running = true;
                true
            }
            B::RingBell(r) => one(
                || {
                    let Some(bell) = mem(m).meeting_point.value() else {
                        return false;
                    };
                    if cx.level.random().next_float() <= 0.95 {
                        return false;
                    }
                    if dist_sqr(bell, block_of(m.position)) < 9
                        && cx.w.block_name(bell[0], bell[1], bell[2]) == "minecraft:bell"
                    {
                        cx.events.push(MobEvent::RingBell { bell });
                    }
                    true
                },
                r,
            ),
            B::ResetRaidStatus(r) => one(
                || {
                    if cx.level.random().next_int_bound(20).unwrap_or(0) != 0 {
                        return false;
                    }
                    if super::villager::raid_over_or_lost(m) {
                        let b = brain_mut(m);
                        b.default_activity = Act::Idle;
                        b.update_from_schedule(cx.w.game_time, cx.w.day_time);
                    }
                    true
                },
                r,
            ),
            B::RaidSequence {
                running,
                victory,
                then,
            } => {
                let state = super::villager::raid_status(m);
                let ok = if *victory {
                    state == super::villager::RAID_VICTORY
                } else {
                    state == super::villager::RAID || state == super::villager::PRE_RAID
                };
                if !ok {
                    return false;
                }
                let fired = then.try_start(m, cx);
                then.one_shot_stop();
                *running = fired;
                fired
            }
            B::MoveToSkySeeingSpot { running, speed } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let here = block_of(m.position);
                if w.can_see_sky(here[0], here[1], here[2]) {
                    return false;
                }
                for _ in 0..10 {
                    let p = [
                        here[0] + m.random.next_int_bound(20).unwrap_or(0) - 10,
                        here[1] + m.random.next_int_bound(6).unwrap_or(0) - 3,
                        here[2] + m.random.next_int_bound(20).unwrap_or(0) - 10,
                    ];
                    if no_blocks_above(m, w, p) {
                        mem(m).walk_target.set(WalkTarget::block(p, *speed, 0));
                        break;
                    }
                }
                *running = true;
                true
            }
            B::Celebrate(t) => {
                let here = block_of(m.position);
                if super::villager::raid_status(m) != super::villager::RAID_VICTORY
                    || !no_blocks_above(m, cx.w, here)
                {
                    return false;
                }
                t.begin(cx.level.random(), cx.now);
                true
            }
            B::LocateHidingPlace {
                running,
                radius,
                speed,
                close,
            } => {
                if mem(m).walk_target.present() {
                    return false;
                }
                let w = cx.w;
                let near = homes_in_range(m, w, *close + 1)
                    .into_iter()
                    .next()
                    .filter(|p| closer_to_center(*p, m.position, f64::from(*close)));
                let place = near
                    .or_else(|| {
                        let mut all = homes_in_range(m, w, *radius);
                        // Util.toShuffledList with the mob's random.
                        for i in (2..=all.len()).rev() {
                            let j = m.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                            all.swap(i - 1, j);
                        }
                        all.into_iter().next()
                    })
                    .or_else(|| mem(m).home.value());
                if let Some(p) = place {
                    let far = !closer_to_center(p, m.position, f64::from(*close));
                    let mm = mem(m);
                    mm.path.clear();
                    mm.look_target.clear();
                    mm.breed_target.clear();
                    mm.interaction_target.clear();
                    mm.hiding_place.set(p);
                    if far {
                        mm.walk_target.set(WalkTarget::block(p, *speed, *close));
                    }
                }
                *running = true;
                true
            }
            B::SetHiddenState { running, hidden } => {
                let mm = mem(m);
                let (Some(place), Some(heard)) =
                    (mm.hiding_place.value(), mm.heard_bell_time.value())
                else {
                    return false;
                };
                let timed_out = heard + 300 <= now;
                if *hidden <= 300 && !timed_out {
                    if dist_sqr(place, block_of(m.position)) < 9 {
                        *hidden += 1;
                    }
                } else {
                    let mm = mem(m);
                    mm.heard_bell_time.clear();
                    mm.hiding_place.clear();
                    brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
                    *hidden = 0;
                }
                *running = true;
                true
            }
            B::PlayTag(r) => one(
                || {
                    let mm = mem(m);
                    let Some(friends) = mm.visible_villager_babies.value() else {
                        return false;
                    };
                    if mm.walk_target.present() {
                        return false;
                    }
                    if cx.level.random().next_int_bound(10).unwrap_or(0) != 0 {
                        return false;
                    }
                    // What a friend chases is its INTERACTION_TARGET.
                    let chasing = |who: Who| match who {
                        Who::Mob(id) => {
                            cx.w.seen(id)
                                .and_then(|s| s.villager.as_ref())
                                .and_then(|v| v.interaction_target)
                        }
                        Who::Player => None,
                    };
                    let me = Who::Mob(m.id);
                    if !friends.iter().any(|&f| chasing(f) == Some(me)) {
                        // The kid chased by the fewest (one to five) chasers, else any friend.
                        let mut chased: Vec<(Who, i32)> = Vec::new();
                        for &f in &friends {
                            if let Some(c) = chasing(f) {
                                match chased.iter_mut().find(|(k, _)| *k == c) {
                                    Some(e) => e.1 += 1,
                                    None => chased.push((c, 1)),
                                }
                            }
                        }
                        let target = chased
                            .iter()
                            .filter(|(_, n)| *n > 0 && *n <= 5)
                            .min_by_key(|(_, n)| *n)
                            .map(|&(k, _)| k)
                            .or_else(|| friends.first().copied());
                        if let Some(kid) = target {
                            let mm = mem(m);
                            mm.interaction_target.set(kid);
                            mm.look_target.set(Tracker::Entity(kid, true));
                            mm.walk_target.set(WalkTarget {
                                target: Tracker::Entity(kid, false),
                                speed: 0.6,
                                close: 1,
                            });
                        }
                    } else {
                        // Chased: run to a spot inside the village.
                        let villages = village_sections(m, cx.w);
                        for _ in 0..10 {
                            let Some(p) = m.land_random_pos(cx.w, 20, 8) else {
                                continue;
                            };
                            let at = [
                                p[0].floor() as i32,
                                p[1].floor() as i32,
                                p[2].floor() as i32,
                            ];
                            let section = [at[0] >> 4, at[1] >> 4, at[2] >> 4];
                            if sections_to_village(&villages, section) <= 1 {
                                mem(m).walk_target.set(WalkTarget::point(p, 0.6, 0));
                                break;
                            }
                        }
                    }
                    true
                },
                r,
            ),
        }
    }

    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot_stop() {
            return;
        }
        let now = cx.now;
        match self {
            B::Gate(g) => g.tick_or_stop(m, cx),
            B::Mod(e) => e.tick_or_stop(m, cx.w, cx.now),
            B::Swim(t) => {
                if now > t.end || !swim_condition(m) {
                    self.do_stop(m, cx);
                } else if m.random.next_float() < 0.8 {
                    m.ai.jump = true;
                }
            }
            B::LookAtTargetSink(t) => {
                let visible = match mem(m).look_target.value() {
                    Some(look) => tracker_visible(m, cx.w, look),
                    None => false,
                };
                if now > t.end || !visible {
                    self.do_stop(m, cx);
                } else if let Some(at) = mem(m).look_target.value().and_then(|l| l.position(cx.w)) {
                    m.look_at(at);
                }
            }
            B::PanicTrigger(t) => {
                if now > t.end || !panic_condition(m) {
                    self.do_stop(m, cx);
                } else if now % 100 == 0 {
                    cx.events.push(MobEvent::GolemCheck {
                        mob: m.id,
                        needed: 3,
                    });
                }
            }
            B::MoveToTargetSink {
                t,
                path,
                last_target,
                speed,
                ..
            } => {
                let still = path.is_some()
                    && last_target.is_some()
                    && mem(m)
                        .walk_target
                        .value()
                        .is_some_and(|walk| !m.ai.nav.is_done() && !reached(m, cx.w, &walk));
                if now > t.end || !still {
                    self.do_stop(m, cx);
                    return;
                }
                // tick
                let new_path = m.ai.nav.path.clone();
                if !same_path(path.as_ref(), new_path.as_ref()) {
                    *path = new_path.clone();
                    mem(m).path.set_opt(new_path.clone());
                }
                if new_path.is_some()
                    && let Some(last) = *last_target
                    && let Some(walk) = mem(m).walk_target.value()
                    && let Some(now_target) = walk.target.block(cx.w)
                {
                    let d = manhattan_sq(now_target, last);
                    if d > 4 && compute_path(m, cx.w, cx.now, &walk, path, speed, cant_reach) {
                        *last_target = Some(now_target);
                        mem(m).path.set_opt(path.clone());
                        m.nav_move_to_path(cx.w, path.clone(), f64::from(*speed));
                    }
                }
            }
            B::FollowTradingPlayer(_) => {
                if !follow_trading_condition(m, cx.w) {
                    self.do_stop(m, cx);
                } else {
                    follow_trading_player(m);
                }
            }
            B::GoToPotentialJobSite(t) => {
                let site = mem(m).potential_job_site.value();
                if now > t.end || site.is_none() {
                    self.do_stop(m, cx);
                } else if let Some(p) = site {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Block(p));
                    mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
                }
            }
            B::JumpOnBed {
                bed,
                time_left,
                jumps,
                cooldown,
                ..
            } => {
                let w = cx.w;
                let here = block_of(m.position);
                let jumpable = |p: [i32; 3]| w.block_name(p[0], p[1], p[2]).ends_with("_bed");
                let on_or_over = jumpable(here) || jumpable([here[0], here[1] - 1, here[2]]);
                let still = m.is_baby()
                    && bed.is_some_and(jumpable)
                    && (on_or_over || *time_left > 0)
                    && (!on_or_over || *jumps > 0);
                if !still {
                    self.do_stop(m, cx);
                    return;
                }
                if !on_or_over {
                    *time_left -= 1;
                } else if *cooldown > 0 {
                    *cooldown -= 1;
                } else if jumpable(here) {
                    m.ai.jump = true;
                    *jumps -= 1;
                    *cooldown = 5;
                }
            }
            B::DoNothing(t) => {
                if now > t.end {
                    self.do_stop(m, cx);
                }
            }
            B::GiveGiftToHero {
                t, given, started, ..
            } => {
                if now > t.end || !hero_visible(m, cx.w) || *given {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(p) = cx.w.selected_player().cloned() else {
                    return;
                };
                mem(m).look_target.set(Tracker::Entity(Who::Player, true));
                let close = {
                    let a = block_of(p.position);
                    let b = block_of(m.position);
                    let d = (a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2);
                    f64::from(d) < 25.0
                };
                if close {
                    if now - *started > 20 {
                        cx.events.push(MobEvent::ThrowGift {
                            table: super::villager::gift_table(m, cx.w.mob_maps),
                            from: [m.position.x, m.eye_y() - 0.3, m.position.z],
                            to: [p.position.x, p.position.y, p.position.z],
                        });
                        *given = true;
                    }
                } else {
                    let mm = mem(m);
                    mm.look_target.set(Tracker::Entity(Who::Player, true));
                    mm.walk_target.set(WalkTarget {
                        target: Tracker::Entity(Who::Player, true),
                        speed: 0.5,
                        close: 5,
                    });
                }
            }
            B::ShowTrades {
                t,
                look_time,
                player_item,
                items,
                cycle,
                index,
            } => {
                let still = show_trades_condition(m, cx.w)
                    && *look_time > 0
                    && mem(m).interaction_target.present();
                if now > t.end || !still {
                    self.do_stop(m, cx);
                    return;
                }
                if let Some(target) = mem(m).interaction_target.value() {
                    mem(m).look_target.set(Tracker::Entity(target, true));
                }
                // findItemsToDisplay: the trades the player's item pays for.
                let current = cx.w.selected_player().and_then(|p| p.main_hand.clone());
                // getOffers makes the trades on first use: the level rolls
                // them, and the cost is compared from the next tick on.
                let unset = m.extra.trade.as_ref().is_none_or(|t| t.unset);
                if current.is_some() && unset {
                    cx.events.push(MobEvent::VillagerOffers { mob: m.id });
                } else if player_item.as_ref() != Some(&current) {
                    *player_item = Some(current.clone());
                    items.clear();
                    if let Some(held) = &current {
                        if let Some(trade) = &m.extra.trade {
                            for offer in &trade.offers {
                                if !offer.out_of_stock()
                                    && (offer.base_cost_a.stack.item == *held
                                        || offer
                                            .cost_b
                                            .as_ref()
                                            .is_some_and(|c| c.stack.item == *held))
                                {
                                    items.push(offer.result.clone());
                                }
                            }
                        }
                        if let Some(first) = items.first() {
                            *look_time = 900;
                            m.extra.set_main_hand_stack(Some(first.clone()));
                            super::finalize::set_drop_chance(
                                m,
                                super::equip::Slot::MainHand.index(),
                                0.0,
                            );
                        }
                    }
                }
                if !items.is_empty() {
                    // displayCyclingItems
                    if items.len() >= 2 {
                        *cycle += 1;
                        if *cycle >= 40 {
                            *index += 1;
                            *cycle = 0;
                            if *index > items.len() - 1 {
                                *index = 0;
                            }
                            m.extra.set_main_hand_stack(Some(items[*index].clone()));
                            super::finalize::set_drop_chance(
                                m,
                                super::equip::Slot::MainHand.index(),
                                0.0,
                            );
                        }
                    }
                } else {
                    // clearHeldItem; no offer matches: look for a while only.
                    m.extra.set_main_hand(None);
                    super::finalize::set_drop_chance(
                        m,
                        super::equip::Slot::MainHand.index(),
                        0.085,
                    );
                    *look_time = (*look_time).min(40);
                }
                *look_time -= 1;
            }
            B::TradeWithVillager(t) => {
                if now > t.end || !villager_target_valid(m, cx.w, Mem::InteractionTarget) {
                    self.do_stop(m, cx);
                    return;
                }
                if let Some(target) = mem(m).interaction_target.value()
                    && let Some(i) = cx.w.target_info(target)
                    && i.position.distance_squared(m.position) <= 5.0
                {
                    let Who::Mob(other) = target else { return };
                    lock_gaze(m, target, cx);
                    cx.events.push(MobEvent::VillagerMeet { mob: m.id, other });
                }
            }
            B::MakeLove { birth, .. } => {
                if now > *birth || !breeding_possible(m, cx.w) {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(target) = mem(m).breed_target.value() else {
                    return;
                };
                let Who::Mob(partner) = target else { return };
                if cx
                    .w
                    .target_info(target)
                    .is_some_and(|t| t.position.distance_squared(m.position) <= 5.0)
                {
                    lock_gaze(m, target, cx);
                    if now >= *birth {
                        // The parent's half happens in its own tick: it eats,
                        // takes a bed for the child, picks its type and rests.
                        super::villager::eat_and_digest_in(m, cx.w);
                        let child = take_bed_for_child(m, cx.w).map(|bed| {
                            // Villager.getBreedOffspring: half the biome's type,
                            // a quarter each parent's.
                            let roll = m.random.next_double();
                            let variant = if roll < 0.5 {
                                None
                            } else if roll < 0.75 {
                                Some(m.extra.variant)
                            } else {
                                Some(cx.w.seen(partner).map_or(m.extra.variant, |s| s.variant))
                            };
                            m.extra.age = 6000;
                            (bed, variant)
                        });
                        cx.events.push(MobEvent::VillagerBirth {
                            parent: m.id,
                            partner,
                            child,
                        });
                    } else if m.random.next_int_bound(35).unwrap_or(1) == 0 {
                        // Entity event 12 for the partner, then this villager:
                        // AbstractVillager.addParticlesAroundSelf(HEART).
                        let mut bodies = Vec::new();
                        if let Some(t) = cx.w.target_info(target) {
                            let (min, max) = (t.bounds.min_position(), t.bounds.max_position());
                            bodies.push((partner, t.position, max.x - min.x, max.y - min.y));
                        }
                        bodies.push((
                            m.id,
                            m.position,
                            f64::from(m.width()),
                            f64::from(m.height()),
                        ));
                        for (mob, feet, width, height) in bodies {
                            cx.events.push(MobEvent::AroundSelf {
                                mob,
                                event: 12,
                                kind: "heart",
                                feet: [feet.x, feet.y, feet.z],
                                width,
                                height,
                                burst: sunrise_level::particle_options::Burst::VILLAGER,
                            });
                        }
                    }
                }
            }
            B::WorkAtPoi { t, .. } => {
                let still = mem(m)
                    .job_site
                    .value()
                    .is_some_and(|site| closer_to_center(site, m.position, 1.73));
                if now > t.end || !still {
                    self.do_stop(m, cx);
                }
            }
            B::HarvestFarmland {
                t,
                next_ok,
                target,
                valid,
                worked,
            } => {
                if now > t.end || *worked >= 200 {
                    self.do_stop(m, cx);
                    return;
                }
                if target.is_none_or(|p| closer_to_center(p, m.position, 1.0)) {
                    harvest_tick(m, cx, next_ok, target, valid);
                    *worked += 1;
                }
            }
            B::SleepInBed { .. } => {
                let still = mem(m).home.value().is_some_and(|bed| {
                    brain_mut(m).is_active(Act::Rest)
                        && m.position.y > f64::from(bed[1]) + 0.4
                        && closer_to_center(bed, m.position, 1.14)
                });
                if !still {
                    self.do_stop(m, cx);
                }
            }
            B::Celebrate(t) => {
                if now > t.end || !super::villager::raid_nearby(m) {
                    self.do_stop(m, cx);
                    return;
                }
                if m.random.next_int_bound(100).unwrap_or(1) == 0 {
                    m.voice("entity.villager.celebrate", cx.events);
                }
                if m.random.next_int_bound(200).unwrap_or(1) == 0
                    && no_blocks_above(m, cx.w, block_of(m.position))
                {
                    let color = m.random.next_int_bound(16).unwrap_or(0) as usize;
                    let flight = m.random.next_int_bound(3).unwrap_or(0) as u8;
                    cx.events.push(MobEvent::Firework {
                        at: [m.position.x, m.eye_y(), m.position.z],
                        color,
                        flight,
                    });
                }
            }
            B::UseBonemeal {
                t,
                next_work,
                worked,
                crop,
                repick,
                ..
            } => {
                if *repick {
                    *repick = false;
                    *crop = pick_bonemeal_crop(m, cx);
                    bonemeal_target(m, *crop);
                }
                // canStillUse: fewer than 80 work cycles and a crop to work on.
                if now > t.end || *worked >= 80 || crop.is_none() {
                    self.do_stop(m, cx);
                    return;
                }
                let Some(p) = *crop else { return };
                if now >= *next_work && closer_to_center(p, m.position, 1.0) {
                    // BoneMealItem.growCrop with the villager's bone meal; the
                    // game grows it and, when it did, calls `bonemeal_grown`.
                    if super::villager::ext(m)
                        .is_some_and(|e| e.count_item("minecraft:bone_meal") > 0)
                    {
                        cx.events.push(MobEvent::Bonemeal { mob: m.id, pos: p });
                    }
                    *worked += 1;
                }
            }
            _ => {}
        }
    }

    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        if self.one_shot_stop() {
            return;
        }
        let now = cx.now;
        match self {
            B::Gate(g) => g.do_stop(m, cx),
            B::Mod(e) => e.do_stop(m, cx.w),
            B::Swim(t) | B::PanicTrigger(t) | B::DoNothing(t) => t.running = false,
            B::JumpOnBed {
                t,
                bed,
                time_left,
                jumps,
                cooldown,
            } => {
                t.running = false;
                *bed = None;
                *time_left = 0;
                *jumps = 0;
                *cooldown = 0;
            }
            B::SleepInBed { t, next_ok } => {
                t.running = false;
                if super::villager::sleeping(m) {
                    super::villager::stop_sleeping(m, cx.w, cx.events);
                    *next_ok = now + 40;
                }
            }
            B::Celebrate(t) => {
                t.running = false;
                brain_mut(m).update_from_schedule(cx.w.game_time, cx.w.day_time);
            }
            B::LookAtTargetSink(t) => {
                t.running = false;
                mem(m).look_target.clear();
            }
            B::MoveToTargetSink {
                t, cooldown, path, ..
            } => {
                t.running = false;
                if let Some(walk) = mem(m).walk_target.value()
                    && !reached(m, cx.w, &walk)
                    && m.ai.nav.stuck
                {
                    *cooldown = cx.level.random().next_int_bound(40).unwrap_or(0);
                }
                m.nav_stop();
                let mm = mem(m);
                mm.walk_target.clear();
                mm.path.clear();
                *path = None;
            }
            B::FollowTradingPlayer(t) => {
                t.running = false;
                let mm = mem(m);
                mm.walk_target.clear();
                mm.look_target.clear();
            }
            B::GoToPotentialJobSite(t) => {
                t.running = false;
                mem(m).potential_job_site.clear();
            }
            B::GiveGiftToHero { t, until_next, .. } => {
                t.running = false;
                *until_next = 600 + cx.level.random().next_int_bound(6001).unwrap_or(0);
                let mm = mem(m);
                mm.interaction_target.clear();
                mm.walk_target.clear();
                mm.look_target.clear();
            }
            B::ShowTrades { t, player_item, .. } => {
                t.running = false;
                mem(m).interaction_target.clear();
                // clearHeldItem
                m.extra.set_main_hand(None);
                super::finalize::set_drop_chance(m, super::equip::Slot::MainHand.index(), 0.085);
                *player_item = None;
            }
            B::TradeWithVillager(t) => {
                t.running = false;
                mem(m).interaction_target.clear();
            }
            B::MakeLove { t, .. } => {
                t.running = false;
                mem(m).breed_target.clear();
            }
            B::UseBonemeal {
                t, last_session, ..
            } => {
                t.running = false;
                m.extra.set_main_hand(None);
                *last_session = i64::from(m.age);
            }
            B::WorkAtPoi { t, .. } => t.running = false,
            B::HarvestFarmland {
                t, next_ok, worked, ..
            } => {
                t.running = false;
                *worked = 0;
                let mm = mem(m);
                mm.look_target.clear();
                mm.walk_target.clear();
                *next_ok = now + 40;
            }
            _ => {}
        }
    }
}

fn manhattan_sq(a: [i32; 3], b: [i32; 3]) -> i64 {
    let d = [
        i64::from(a[0] - b[0]),
        i64::from(a[1] - b[1]),
        i64::from(a[2] - b[2]),
    ];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}
fn same_path(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.nodes == b.nodes && a.target == b.target,
        _ => false,
    }
}
/// `Swim`: deeper than the jump threshold, or in lava.
fn swim_condition(m: &Mob) -> bool {
    m.floats()
}
fn panic_condition(m: &Mob) -> bool {
    mem_ref(m).is_some_and(|mm| mm.hurt_by.present() || mm.nearest_hostile.present())
}
/// `PositionTracker.isVisibleBy`.
fn tracker_visible(m: &mut Mob, w: &MobWorld, t: Tracker) -> bool {
    match t {
        Tracker::Entity(who, _) => {
            w.target_info(who).is_some_and(|i| i.alive) && visible_contains(m, w, who)
        }
        _ => true,
    }
}
/// `MoveToTargetSink.reachedTarget`.
pub(super) fn reached(m: &Mob, w: &MobWorld, walk: &WalkTarget) -> bool {
    walk.target
        .block(w)
        .is_some_and(|p| manhattan(p, block_of(m.position)) <= walk.close)
}
/// `MoveToTargetSink.tryComputePath`, for the brains on these memories
/// (villager, monster, warden): `cant_reach` is the brain's
/// `CANT_REACH_WALK_TARGET_SINCE`.
pub(super) fn compute_path(
    m: &mut Mob,
    w: &MobWorld,
    now: i64,
    walk: &WalkTarget,
    path: &mut Option<Path>,
    speed: &mut f32,
    cant_reach: fn(&mut Mob) -> &mut Slot<i64>,
) -> bool {
    let Some(target) = walk.target.block(w) else {
        return false;
    };
    *path = m.create_path_to(w, target, 0);
    *speed = walk.speed;
    if reached(m, w, walk) {
        cant_reach(m).clear();
    } else {
        let can_reach = path.as_ref().is_some_and(|p| p.reached);
        let since = cant_reach(m);
        if can_reach {
            since.clear();
        } else if !since.present() {
            since.set(now);
        }
        if path.is_some() {
            return true;
        }
        let to = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]),
            f64::from(target[2]) + 0.5,
        ];
        if let Some(step) =
            m.default_random_pos_towards(w, 10, 7, to, f64::from(std::f32::consts::FRAC_PI_2))
        {
            let p = [
                step[0].floor() as i32,
                step[1].floor() as i32,
                step[2].floor() as i32,
            ];
            *path = m.create_path_to(w, p, 0);
            return path.is_some();
        }
    }
    false
}
/// `LookAndFollowTradingPlayerSink.checkExtraStartConditions`.
fn follow_trading_condition(m: &Mob, w: &MobWorld) -> bool {
    m.extra.flag2
        && m.alive()
        && !m.fluids.in_water
        && m.hurt_time <= 0
        && w.selected_player()
            .is_some_and(|p| m.distance_sqr(p.position) <= 16.0)
}
fn follow_trading_player(m: &mut Mob) {
    let mm = mem(m);
    mm.walk_target.set(WalkTarget {
        target: Tracker::Entity(Who::Player, false),
        speed: 0.5,
        close: 2,
    });
    mm.look_target.set(Tracker::Entity(Who::Player, true));
}
/// `GiveGiftToHero.isHeroVisible`.
fn hero_visible(m: &Mob, w: &MobWorld) -> bool {
    mem_ref(m).is_some_and(|mm| mm.nearest_visible_player.get() == Some(&Who::Player))
        && w.selected_player().is_some_and(|p| p.hero)
}
/// `ShowTradesToPlayer.checkExtraStartConditions`.
fn show_trades_condition(m: &Mob, w: &MobWorld) -> bool {
    let Some(target) = mem_ref(m).and_then(|mm| mm.interaction_target.value()) else {
        return false;
    };
    target == Who::Player
        && m.alive()
        && !m.is_baby()
        && w.target_info(target)
            .is_some_and(|t| t.alive && m.distance_sqr(t.position) <= 17.0)
}
/// `BehaviorUtils.targetIsValid` for a villager.
fn villager_target_valid(m: &mut Mob, w: &MobWorld, which: Mem) -> bool {
    let target = mem_ref(m).and_then(|mm| match which {
        Mem::InteractionTarget => mm.interaction_target.value(),
        Mem::BreedTarget => mm.breed_target.value(),
        _ => None,
    });
    let Some(Who::Mob(id)) = target else {
        return false;
    };
    w.seen(id)
        .is_some_and(|s| s.species == Species::Villager && s.alive)
        && visible_contains(m, w, Who::Mob(id))
}
/// `Villager.playWorkSound`.
fn play_work_sound(m: &mut Mob, cx: &mut Bx) {
    let Some(index) = cx.w.professions.state_index(&m.extra.profession) else {
        return;
    };
    if index < 15 {
        if let Some(sound) = super::villager::work_sound(index as u8) {
            m.voice(sound, cx.events);
        }
    } else if let Some(sound) = cx.w.professions.entry(index).and_then(|p| p.sound.clone()) {
        let pitch = m.voice_pitch();
        if !m.silent {
            cx.events.push(MobEvent::ProfessionWorkSound(
                sound,
                [m.position.x, m.position.y, m.position.z],
                super::ai::sound_volume(m),
                pitch,
            ));
        }
    }
}
/// The live inputs to `Villager.updateSpecialPrices` belonging to the trading player.
pub struct TradingPlayer {
    pub uuid: Option<[i32; 4]>,
    pub hero: Option<i32>,
    /// `MerchantContainer.updateSellItem` plays feedback only with a payment.
    pub paying: bool,
}

pub trait TradingPlayers {
    fn trading_player(&self, villager: u32) -> Option<TradingPlayer>;
}

/// What a work-step restock changes in the trading screen, with the player's prices and
/// the trade sound's pitch as chosen at the work step.
#[derive(Clone, Debug)]
pub struct RestockNotice {
    pub mob: u32,
    pub step: crate::merchant::RestockStep,
    pub reputation: i32,
    pub hero: Option<i32>,
    pub pitch: Option<f32>,
}

/// `WorkAtPoi.start`'s restock.
fn restock_if_due(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if w.professions.state_index(&m.extra.profession).is_none() {
        return;
    }
    let day = w.day_time.div_euclid(24000);
    let Some(t) = m.extra.trade.as_mut() else {
        return;
    };
    // Each step with a trading player runs `updateSpecialPrices`, which
    // updates and resends its screen (a new day can send two); the trade
    // sound's pitch is drawn here, in work-step order.
    let catch_up = t.catch_up(w.game_time, day);
    let restock = t.restock_due(w.game_time).then(|| {
        t.restock(w.game_time);
        crate::merchant::RestockStep::Restock
    });
    if (catch_up.is_none() && restock.is_none()) || !m.extra.flag2 || t.offers.is_empty() {
        return;
    }
    let Some(player) = w
        .trading_players
        .and_then(|players| players.trading_player(m.id))
    else {
        return;
    };
    let reputation = player
        .uuid
        .zip(super::villager::ext(m))
        .map_or(0, |(uuid, ext)| ext.gossips.reputation(uuid));
    let interval = m.ambient_interval();
    for step in catch_up.into_iter().chain(restock) {
        let pitch = (player.paying
            && crate::merchant::reset_trade_update_clock(&mut m.ambient_timer, interval))
        .then(|| m.voice_pitch());
        events.push(MobEvent::VillagerRestockStep(RestockNotice {
            mob: m.id,
            step,
            reputation,
            hero: player.hero,
            pitch,
        }));
    }
}
