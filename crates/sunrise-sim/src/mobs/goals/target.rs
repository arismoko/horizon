// Included in goals.rs to retain its private scope.
/// What a target goal hunts.
#[derive(Clone, Debug)]
pub enum Prey {
    Player,
    Species(Vec<Species>),
    /// Any mob matching a predicate on its snapshot.
    Filter(fn(&Seen) -> bool),
}
/// NearestAttackableTargetGoal (+ TargetGoal's continue rules).
#[derive(Clone, Debug)]
pub struct NearestTarget {
    pub prey: Prey,
    pub must_see: bool,
    pub must_reach: bool,
    pub interval: i32,
    /// Extra condition on the attacker, checked after the random interval.
    pub when: Option<fn(&Mob, &MobWorld) -> bool>,
    /// Mobs closer than this squared distance are not picked
    /// (`GuardianAttackSelector`: `distanceToSqr > 9`).
    pub beyond_sqr: Option<f64>,
    /// A condition checked before anything else, by subclasses overriding
    /// `canUse` (SpiderTargetGoal: only in the dark).
    pub gate: Option<fn(&Mob, &MobWorld) -> bool>,
    /// The vanilla class name of a subclass (`Goal::name`).
    pub label: Option<&'static str>,
    found: Option<Who>,
    target_goal: TargetState,
}
#[derive(Clone, Debug)]
pub struct TargetState {
    unseen: i32,
    pub unseen_memory: i32,
    /// `TargetGoal.getFollowDistance` as a fraction of the follow range.
    pub follow_scale: f64,
}
impl Default for TargetState {
    fn default() -> Self {
        Self {
            unseen: 0,
            unseen_memory: 0,
            follow_scale: 1.0,
        }
    }
}
impl NearestTarget {
    pub fn new(prey: Prey, must_see: bool) -> Self {
        Self {
            prey,
            must_see,
            must_reach: false,
            interval: reduced(10),
            when: None,
            beyond_sqr: None,
            gate: None,
            label: None,
            found: None,
            target_goal: TargetState {
                unseen_memory: 60,
                ..Default::default()
            },
        }
    }
    pub fn player(must_see: bool) -> Self {
        Self::new(Prey::Player, must_see)
    }
    pub fn species(list: &[Species], must_see: bool) -> Self {
        Self::new(Prey::Species(list.to_vec()), must_see)
    }
    pub fn interval(mut self, i: i32) -> Self {
        self.interval = reduced(i);
        self
    }
    /// `TargetGoal.setUnseenMemoryTicks`.
    pub fn unseen_memory(mut self, ticks: i32) -> Self {
        self.target_goal.unseen_memory = ticks;
        self
    }
    /// Scale the follow distance (llamas hunt wolves within a quarter of it).
    pub fn follow_scale(mut self, scale: f64) -> Self {
        self.target_goal.follow_scale = scale;
        self
    }
    pub fn reach(mut self) -> Self {
        self.must_reach = true;
        self
    }
    pub fn when(mut self, f: fn(&Mob, &MobWorld) -> bool) -> Self {
        self.when = Some(f);
        self
    }
    pub fn beyond(mut self, distance_sqr: f64) -> Self {
        self.beyond_sqr = Some(distance_sqr);
        self
    }
    pub fn gate(mut self, f: fn(&Mob, &MobWorld) -> bool) -> Self {
        self.gate = Some(f);
        self
    }
    /// A subclass's name, e.g. `NearestAttackableWitchTargetGoal`.
    pub fn named(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }
}
/// `AbstractIllager.canAttack`: a baby `AbstractVillager` is never attacked.
fn illager_spares(attacker: Species, target: &Seen) -> bool {
    matches!(
        attacker,
        Species::Pillager | Species::Vindicator | Species::Evoker | Species::Illusioner
    ) && target.baby
        && matches!(target.species, Species::Villager | Species::WanderingTrader)
}
/// TargetGoal.canContinueToUse
pub fn target_continue(st: &mut TargetState, must_see: bool, m: &mut Mob, cx: &mut Cx) -> bool {
    let Some(who) = m.ai.target else { return false };
    if let Who::Mob(id) = who
        && cx
            .w
            .seen(id)
            .is_some_and(|s| illager_spares(m.species(), s))
    {
        return false;
    }
    let Some(t) = cx.w.target_info(who) else {
        return false;
    };
    if !t.alive
        || !t.can_be_seen_as_enemy
        || (t.is_player && t.creative)
        || m.spares_target(cx.w, who)
    {
        return false;
    }
    let follow = m.attrs.follow_range * st.follow_scale;
    if m.distance_sqr(t.position) > follow * follow {
        return false;
    }
    if must_see {
        if m.sees(cx.w, who) {
            st.unseen = 0;
        } else {
            st.unseen += 1;
            if st.unseen > reduced(st.unseen_memory) {
                return false;
            }
        }
    }
    m.set_ai_target(cx.w, Some(who));
    true
}
impl Goal for NearestTarget {
    goal_clone!();
    #[inline]
    fn name(&self) -> &'static str {
        self.label.unwrap_or("NearestTarget")
    }
    #[inline]
    fn flags(&self) -> u8 {
        TARGET
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if let Some(f) = self.gate
            && !f(m, cx.w)
        {
            return false;
        }
        if self.interval > 0 && m.random.next_int_bound(self.interval).unwrap_or(1) != 0 {
            return false;
        }
        if let Some(f) = self.when
            && !f(m, cx.w)
        {
            return false;
        }
        let range = m.attrs.follow_range * self.target_goal.follow_scale;
        self.found = None;
        // NearestAttackableTargetGoal.findTarget: TargetingConditions.forCombat()
        // always checks the line of sight; mustSee and mustReach only matter
        // to TargetGoal.canContinueToUse and canAttack.
        match &self.prey {
            Prey::Player => {
                if m.can_target_player(cx.w, range, true) {
                    self.found = Some(Who::Player);
                }
            }
            prey => {
                let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
                let id = m.id;
                let attacker = m.species();
                for candidate in cx.w.nearby(pos, range, |s| {
                    s.id != id
                        && !s.playing_dead
                        && !illager_spares(attacker, s)
                        && match prey {
                            Prey::Species(list) => list.contains(&s.species),
                            Prey::Filter(f) => f(s),
                            Prey::Player => false,
                        }
                }) {
                    let cid = candidate.id;
                    let distance_sqr = m.distance_sqr(candidate.position);
                    if distance_sqr > cx.w.reach_sqr(cid, m, range) {
                        continue;
                    }
                    if self.beyond_sqr.is_some_and(|d| distance_sqr <= d) {
                        continue;
                    }
                    if !m.sees(cx.w, Who::Mob(cid)) {
                        continue;
                    }
                    self.found = Some(Who::Mob(cid));
                    break;
                }
            }
        }
        self.found.is_some()
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        target_continue(&mut self.target_goal, self.must_see, m, cx)
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.set_ai_target(cx.w, self.found);
        self.target_goal.unseen = 0;
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.set_ai_target(cx.w, None);
    }
}
/// HurtByTargetGoal (optionally alerting others of the same species).
#[derive(Clone, Debug)]
pub struct HurtByTarget {
    pub alert: bool,
    /// Species that should not be alerted.
    pub alert_except: Vec<Species>,
    /// Attackers ignored (e.g. zombified piglins ignore other zombified piglins).
    pub ignore: Vec<Species>,
    /// `Bee.BeeHurtByOtherGoal`: kept only while angry, and alerting others
    /// only when the attacker is in sight.
    pub bee: bool,
    /// Whom `alertOther` calls: a filter on the mob (`PandaHurtByTargetGoal`
    /// calls aggressive pandas only).
    pub alert_if: Option<fn(&Seen) -> bool>,
    /// `PolarBear.PolarBearHurtByTargetGoal`: a hurt cub only calls the
    /// adult bears and drops the target itself.
    pub polar_bear: bool,
    timestamp: i32,
    st: TargetState,
}
impl HurtByTarget {
    pub fn new() -> Self {
        Self {
            alert: false,
            alert_except: vec![],
            ignore: vec![],
            bee: false,
            alert_if: None,
            polar_bear: false,
            timestamp: 0,
            st: TargetState {
                unseen_memory: 300,
                ..Default::default()
            },
        }
    }
    pub fn alert(mut self) -> Self {
        self.alert = true;
        self
    }
    pub fn ignoring(mut self, s: &[Species]) -> Self {
        self.ignore = s.to_vec();
        self
    }
    pub fn bee(mut self) -> Self {
        self.bee = true;
        self
    }
    pub fn alert_if(mut self, f: fn(&Seen) -> bool) -> Self {
        self.alert_if = Some(f);
        self
    }
    pub fn polar_bear(mut self) -> Self {
        self.polar_bear = true;
        self
    }
}
impl Default for HurtByTarget {
    fn default() -> Self {
        Self::new()
    }
}
impl Goal for HurtByTarget {
    goal_clone!();
    #[inline]
    fn name(&self) -> &'static str {
        if self.bee {
            "BeeHurtByOtherGoal"
        } else if self.polar_bear {
            "PolarBearHurtByTargetGoal"
        } else {
            "HurtByTarget"
        }
    }
    #[inline]
    fn flags(&self) -> u8 {
        TARGET
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(by) = m.ai.last_hurt_by else {
            return false;
        };
        if m.ai.last_hurt_by_time == self.timestamp {
            return false;
        }
        // `HurtByTargetGoal.canUse`: with universal anger on, a player's hit
        // is left to the anger goals, never a direct retaliation.
        if by == Who::Player && cx.w.universal_anger {
            return false;
        }
        if let Who::Mob(id) = by
            && cx
                .w
                .seen(id)
                .is_some_and(|s| self.ignore.contains(&s.species))
        {
            return false;
        }
        let Some(t) = cx.w.target_info(by) else {
            return false;
        };
        t.alive
            && t.can_be_seen_as_enemy
            && !(t.is_player && t.creative)
            // HurtByTargetGoal.canUse ends with canAttack(lastHurtByMob).
            && !m.spares_target(cx.w, by)
            // TargetGoal.canAttack: Mob.isWithinHome(target.blockPosition()).
            && m.within_home_at(t.position)
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.bee && !super::bee::angry(m, cx.w) {
            return false;
        }
        target_continue(&mut self.st, true, m, cx)
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.set_ai_target(cx.w, m.ai.last_hurt_by);
        self.timestamp = m.ai.last_hurt_by_time;
        self.st.unseen_memory = 300;
        self.st.unseen = 0;
        // PolarBearHurtByTargetGoal.start: a cub alerts the adults itself.
        let cub = self.polar_bear && m.is_baby();
        if (self.alert || cub)
            && let Some(by) = m.ai.target
        {
            let range = m.attrs.follow_range;
            let species = m.species();
            let id = m.id;
            let pos = m.position;
            // TamableAnimal: its owner (`getOwner`, none for a wild one) and
            // whether it is a pet at all.
            let tamable = m.is_tamable_animal();
            let owner = m.extra.tame.then_some(m.extra.owner).flatten();
            let player = cx.w.selected_player().map(|p| p.uuid);
            let ids: Vec<u32> =
                cx.w.others
                    .iter()
                    .filter(|s| {
                        s.id != id
                            && s.alive
                            && s.species == species
                            && s.target.is_none()
                            && (s.position.x - pos.x).abs() <= range + 1.0
                            && (s.position.y - pos.y).abs() <= 11.0
                            && (s.position.z - pos.z).abs() <= range + 1.0
                    })
                    .filter(|s| !self.alert_except.contains(&s.species))
                    .filter(|s| self.alert_if.is_none_or(|f| f(s)))
                    // PolarBearHurtByTargetGoal.alertOther: adults only.
                    .filter(|s| !(self.polar_bear && s.baby))
                    .filter(|s| Who::Mob(s.id) != by)
                    // HurtByTargetGoal.alertOthers: a pet is only called by a
                    // pet with the same owner.
                    .filter(|s| !tamable || s.tame.then_some(s.owner).flatten() == owner)
                    // TamableAnimal.considersEntityAsAlly: a pet tamed by the
                    // player who hurt it is allied with them, so it is not called.
                    .filter(|s| {
                        !(tamable
                            && by == Who::Player
                            && s.tame
                            && s.owner.is_some()
                            && s.owner == player)
                    })
                    .map(|s| s.id)
                    .collect();
            // Bee.BeeHurtByOtherGoal.alertOther: only with the attacker in sight.
            if self.bee && !ids.is_empty() && !m.sees(cx.w, by) {
                return;
            }
            for other in ids {
                cx.events.push(MobEvent::SetTarget {
                    mob: other,
                    target: by,
                });
            }
        }
        if cub {
            m.set_ai_target(cx.w, None);
        }
    }
    #[inline]
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.set_ai_target(cx.w, None);
    }
}
/// What an AvoidEntityGoal flees from.
#[derive(Clone, Debug)]
pub enum Avoid {
    Player,
    Species(Vec<Species>),
}
/// AvoidEntityGoal.
#[derive(Clone, Debug)]
pub struct AvoidEntity {
    pub what: Avoid,
    pub max_dist: f32,
    pub walk: f64,
    pub sprint: f64,
    /// Extra condition on the avoider (e.g. untamed cats only).
    pub when: Option<fn(&Mob) -> bool>,
    /// Extra condition on the player or mob avoided (`AvoidEntityGoal`'s
    /// predicate: a sneaking player, a tame wolf).
    pub player_when: Option<fn(&Mob, &super::PlayerView) -> bool>,
    pub seen_when: Option<fn(&Seen) -> bool>,
    from: Vec3,
    /// The mob being avoided (`toAvoid`), when it is one.
    pub found: Option<u32>,
}
impl AvoidEntity {
    pub fn new(what: Avoid, max_dist: f32, walk: f64, sprint: f64) -> Self {
        Self {
            what,
            max_dist,
            walk,
            sprint,
            when: None,
            player_when: None,
            seen_when: None,
            from: Vec3::ZERO,
            found: None,
        }
    }
    pub fn when(mut self, f: fn(&Mob) -> bool) -> Self {
        self.when = Some(f);
        self
    }
    pub fn player_when(mut self, f: fn(&Mob, &super::PlayerView) -> bool) -> Self {
        self.player_when = Some(f);
        self
    }
    pub fn seen_when(mut self, f: fn(&Seen) -> bool) -> Self {
        self.seen_when = Some(f);
        self
    }
    /// The target the goal flees from, before the random retreat position and
    /// the path. `AvoidEntityGoal` finds it with `TargetingConditions.forCombat()`:
    /// for the player that is `canAttack` (`LivingEntity.canAttack` refuses a
    /// player in Peaceful), the range scaled by `getVisibilityPercent` with a
    /// floor of 2 and line of sight required; the species arm needs the line of
    /// sight too, because `checkLineOfSight` defaults to true. Without them an
    /// animal fled a player behind a wall, a sneaking or invisible one from the
    /// full range, and in Peaceful where vanilla animals ignore the player.
    fn avoid_target(&mut self, m: &mut Mob, cx: &mut Cx) -> Option<Vec3> {
        let max = f64::from(self.max_dist);
        if matches!(self.what, Avoid::Player) {
            let wanted = cx.w.selected_player().is_none_or(|p| {
                self.player_when.is_none_or(|f| f(m, p))
                    && (p.position.y - m.position.y).abs() <= max + 3.0
            });
            // `can_target_player` refuses a player in Peaceful (`spares_players`).
            if !wanted || !m.can_target_player(cx.w, max, true) {
                return None;
            }
            return cx.w.selected_player().map(|p| p.position);
        }
        let Avoid::Species(list) = &self.what else {
            return None;
        };
        let id = m.id;
        let y = m.position.y;
        let seen_when = self.seen_when;
        let mut hit = None;
        for s in cx.w.nearby(m.position, max, move |s| {
            s.id != id
                && list.contains(&s.species)
                && seen_when.is_none_or(|f| f(s))
                && (s.position.y - y).abs() <= 3.0 + max
        }) {
            if m.sees(cx.w, Who::Mob(s.id)) {
                hit = Some((s.id, s.position));
                break;
            }
        }
        self.found = hit.map(|h| h.0);
        hit.map(|h| h.1)
    }
}
impl Goal for AvoidEntity {
    goal_clone!();
    #[inline]
    fn flags(&self) -> u8 {
        MOVE
    }
    #[inline]
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if let Some(f) = self.when
            && !f(m)
        {
            return false;
        }
        let max = f64::from(self.max_dist);
        let Some(from) = self.avoid_target(m, cx) else {
            return false;
        };
        // The player arm already applied the visibility-scaled range (floor 2);
        // the species arm only got a box test from `nearby`.
        if !matches!(self.what, Avoid::Player) && m.distance_sqr(from) > max * max {
            return false;
        }
        let Some(pos) = m.default_random_pos_away(cx.w, 16, 7, [from.x, from.y, from.z]) else {
            return false;
        };
        let to_pos =
            (from.x - pos[0]).powi(2) + (from.y - pos[1]).powi(2) + (from.z - pos[2]).powi(2);
        if to_pos < m.distance_sqr(from) {
            return false;
        }
        self.from = from;
        let path = m.create_path_to(
            cx.w,
            [
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32,
            ],
            0,
        );
        m.ai.pending_path = path;
        m.ai.pending_path.is_some()
    }
    #[inline]
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    #[inline]
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, self.walk);
    }
    #[inline]
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let from = match &self.what {
            Avoid::Player => {
                cx.w.selected_player()
                    .map(|p| p.position)
                    .unwrap_or(self.from)
            }
            Avoid::Species(_) => self.from,
        };
        m.ai.nav.speed_modifier = if m.distance_sqr(from) < 49.0 {
            self.sprint
        } else {
            self.walk
        };
    }
}
