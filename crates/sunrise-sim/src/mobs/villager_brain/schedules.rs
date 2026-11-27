// Included in villager_brain.rs to retain its private scope.
/// The `villager_activity` and `baby_villager_activity` timelines.
fn schedule(baby: bool, day_time: i64) -> Act {
    match super::villager::activity_at(baby, day_time) {
        super::villager::Activity::Idle => Act::Idle,
        super::villager::Activity::Work => Act::Work,
        super::villager::Activity::Play => Act::Play,
        super::villager::Activity::Meet => Act::Meet,
        super::villager::Activity::Rest => Act::Rest,
    }
}

/// `Brain.Provider.makeBrain` and `Villager.registerBrainGoals`: a new brain
/// from the packed memories (the sensors' first scans are delayed by draws
/// from the mob's random), on the timeline's activity.
fn make_brain(m: &mut Mob, packed: Memories, shuffle_seed: i64, w_times: (i64, i64)) -> Brain {
    let generation = m.next_brain_generation();
    let mut b = Brain::new(
        generation,
        m.is_baby(),
        shuffle_seed,
        super::villager::is_farmer(m),
    );
    for (i, rate) in SENSOR_RATES.iter().enumerate() {
        b.sensors[i] = i64::from(m.random.next_int_bound(*rate as i32).unwrap_or(0));
    }
    b.mem = packed;
    b.update_from_schedule(w_times.0, w_times.1);
    m.stage_brain_behaviors(super::mod_behaviors::BehaviorTarget::Villager(generation));
    b
}

fn initial_memories(m: &Mob) -> Memories {
    let mut memories = Memories::default();
    memories.job_site.set_opt(m.extra.home);
    memories.home.set_opt(m.extra.bed);
    memories.meeting_point.set_opt(m.extra.meeting);
    memories
        .last_slept
        .set_opt(super::villager::ext(m).and_then(|e| e.last_slept));
    memories
}

/// Builds the villager's brain as its constructor does, at this game time:
/// the memories it remembers (job site, bed, bell) come from the mob.
pub fn init(m: &mut Mob, game_time: i64, day_time: i64, shuffle_seed: i64) {
    let saved = super::villager::ext_mut(m).saved_memories.take();
    let packed = saved.map_or_else(|| initial_memories(m), |saved| *saved);
    let b = make_brain(m, packed, shuffle_seed, (game_time, day_time));
    super::villager::ext_mut(m).brain = Some(Box::new(b));
}

/// What behaviours read and emit during a brain tick.
pub struct Bx<'a, 'w, 'l> {
    pub w: &'a MobWorld<'w>,
    pub level: &'a mut DamageLevel<'l>,
    pub events: &'a mut Vec<MobEvent>,
    pub now: i64,
    /// `Villager.refreshBrain` was asked for.
    refresh: bool,
}

/// Ensure the villager's brain is initialized before a brain tick so any staged
/// `register_behaviors` dispatch can run on the owning `Game` first.
pub fn ensure_brain(m: &mut Mob, game_time: i64, day_time: i64) {
    if brain(m).is_none() {
        let seed = i64::from(m.id);
        init(m, game_time, day_time, seed);
    }
}

/// `Brain.tick` (from `Villager.customServerAiStep`).
pub fn tick(m: &mut Mob, w: &MobWorld, level: &mut DamageLevel<'_>, events: &mut Vec<MobEvent>) {
    ensure_brain(m, w.game_time, w.day_time);
    // forgetOutdatedMemories
    mem(m).tick();
    tick_sensors(m, w, level.random());
    let mut list = std::mem::take(&mut brain_mut(m).list);
    let mut cx = Bx {
        w,
        level,
        events,
        now: w.game_time,
        refresh: false,
    };
    // The brain whose behaviours these are: after a refresh they keep running
    // against the new brain's memories, gated by the old brain's activities.
    let mut old_activity: Option<Act> = None;
    let mut refreshed = false;
    for i in 0..list.len() {
        let active = match old_activity {
            Some(a) => list[i].act == Act::Core || list[i].act == a,
            None => brain_mut(m).is_active(list[i].act),
        };
        if active && !list[i].b.running() {
            list[i].b.try_start(m, &mut cx);
            if cx.refresh {
                cx.refresh = false;
                old_activity = Some(brain_mut(m).activity);
                refresh(m, &mut list, i, &mut cx);
                refreshed = true;
            }
        }
    }
    for e in list.iter_mut() {
        if e.b.running() {
            e.b.tick_or_stop(m, &mut cx);
        }
    }
    if !refreshed {
        brain_mut(m).list = list;
    }
    sync(m, w.game_time);
}

/// `Villager.ageBoundaryReached`: a villager that grew up gets a new brain
/// (the schedule it follows is the adult's), as `refreshBrain` builds it.
pub fn age_boundary_reached(
    m: &mut Mob,
    w: &MobWorld,
    level: &mut DamageLevel<'_>,
    events: &mut Vec<MobEvent>,
) {
    if brain(m).is_none() {
        return;
    }
    let mut list = std::mem::take(&mut brain_mut(m).list);
    let mut cx = Bx {
        w,
        level,
        events,
        now: w.game_time,
        refresh: false,
    };
    refresh(m, &mut list, usize::MAX, &mut cx);
}

/// `Villager.refreshBrain`: stop every running behaviour, then rebuild the
/// brain from the remembered memories.
fn refresh(m: &mut Mob, list: &mut [Entry], current: usize, cx: &mut Bx) {
    for (i, e) in list.iter_mut().enumerate() {
        if i != current && e.b.running() {
            e.b.do_stop(m, cx);
        }
    }
    let b = brain_mut(m);
    let packed = b.mem.pack();
    let seed = b.shuffle_seed;
    let nb = make_brain(m, packed, seed, (cx.w.game_time, cx.w.day_time));
    super::villager::ext_mut(m).brain = Some(Box::new(nb));
}

/// Mirrors the remembered positions onto the mob (saved, seen by others),
/// and the sleep and golem memories onto `villager::Ext`.
fn sync(m: &mut Mob, now: i64) {
    let (job, home, meeting, slept, golem) = {
        let b = brain_mut(m);
        (
            b.mem.job_site.value(),
            b.mem.home.value(),
            b.mem.meeting_point.value(),
            b.mem.last_slept.value(),
            b.mem
                .golem_detected_recently
                .get()
                .map(|_| now + b.mem.golem_detected_recently.ttl + 1),
        )
    };
    m.extra.home = job;
    m.extra.bed = home;
    m.extra.meeting = meeting;
    let e = super::villager::ext_mut(m);
    e.last_slept = slept;
    e.golem_until = golem.unwrap_or(0);
}

// ---- Sensors ------------------------------------------------------------

fn tick_sensors(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
    for i in 0..9 {
        let fire = {
            let b = brain_mut(m);
            b.sensors[i] -= 1;
            if b.sensors[i] <= 0 {
                b.sensors[i] = SENSOR_RATES[i];
                true
            } else {
                false
            }
        };
        if !fire {
            continue;
        }
        match i {
            0 => sense_living(m, w),
            1 => sense_players(m, w),
            2 => brain_mut(m).scan_items = true,
            3 => sense_bed(m, w, lr),
            4 => sense_hurt(m, w),
            5 => sense_hostiles(m, w),
            6 => sense_babies(m, w),
            7 => sense_secondary(m, w),
            _ => sense_golem(m, w),
        }
    }
}
