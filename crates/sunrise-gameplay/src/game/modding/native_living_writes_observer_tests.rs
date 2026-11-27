#[cfg(test)]
pub(crate) mod tests {

    use mod_api::client::first_person::Hand;

    use mod_api::{
        CanContinueUsingEvent, Flow, Hooks, Host, HostSide, ItemPart,
        LivingEntityUseItemFinishEvent, LivingEntityUseItemStartEvent,
        LivingEntityUseItemStopEvent, LivingEntityUseItemTickEvent, LivingRef, OnStopUsingEvent,
        OnUseTickEvent, Registrar, ReleaseUsingEvent,
    };

    use std::sync::{Arc, Mutex};

    /// What a mod heard of item uses, and what it does back.
    #[derive(Default)]
    pub(in crate::game) struct Uses {
        /// Each use hook heard, with the `entity` it was given.
        pub(in crate::game) heard: Vec<(&'static str, Option<i32>)>,
        pub(in crate::game) stops: Vec<(Option<i32>, Hand, i32)>,
        pub(in crate::game) releases: Vec<(Option<i32>, i32)>,
        /// Refuses every Start.
        pub(in crate::game) refuse: bool,
        pub(in crate::game) start_duration: Option<i32>,
        pub(in crate::game) tick_duration: Option<i32>,
        pub(in crate::game) cancel_tick: bool,
        pub(in crate::game) cancel_stop: bool,
        pub(in crate::game) finishes: usize,
        pub(in crate::game) clear_finish_result: bool,
        pub(in crate::game) effects: usize,
        pub(in crate::game) tick_remaining: Vec<i32>,
        /// Who to release from a Tick hook, and from an `Item.onUseTick`.
        pub(in crate::game) release_on_tick: Option<LivingRef>,
        pub(in crate::game) release_on_use_tick: Option<LivingRef>,
        pub(in crate::game) remaining_on_use_tick: Option<(LivingRef, i32)>,
        /// Who to release once more from the Stop hook, and what that answered.
        pub(in crate::game) release_on_stop: Option<LivingRef>,
        pub(in crate::game) released_again: Vec<bool>,
    }
    pub(in crate::game) type Shared = Arc<Mutex<Uses>>;
    #[derive(Clone)]
    pub(in crate::game) struct UsePart(pub(in crate::game) Shared);
    #[mod_api::sunrise_part]
    impl ItemPart for UsePart {
        fn on_use_tick(&self, host: HostSide, event: &mut OnUseTickEvent<'_>) {
            self.0.lock().unwrap().effects += 1;
            self.heard("on_use_tick", event.entity);
            let (release, remaining) = {
                let mut uses = self.0.lock().unwrap();
                (uses.release_on_use_tick, uses.remaining_on_use_tick.take())
            };
            if let HostSide::Server(host) = host {
                if let Some((who, remaining)) = remaining {
                    assert!(who.set_item_use_remaining(host, remaining));
                }
                if let Some(who) = release {
                    assert!(who.release_using_item(host));
                }
            }
        }
        fn release_using(&self, _: HostSide, event: &mut ReleaseUsingEvent<'_>) {
            self.heard("release_using", event.entity);
            let mut uses = self.0.lock().unwrap();
            uses.releases.push((event.entity, event.time_left));
        }
        fn can_continue_using(&self, _: &dyn Host, event: &mut CanContinueUsingEvent<'_>) {
            self.heard("can_continue_using", event.entity);
        }
        fn on_stop_using(&self, _: HostSide, event: &mut OnStopUsingEvent<'_>) {
            self.heard("on_stop_using", event.entity);
        }
    }
    impl UsePart {
        pub(in crate::game) fn heard(&self, hook: &'static str, entity: Option<i32>) {
            self.0.lock().unwrap().heard.push((hook, entity));
        }
    }
    pub(in crate::game) struct UseMod(pub(in crate::game) Shared);
    #[mod_api::__subscriptions]
    impl Hooks for UseMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            for id in [
                "minecraft:bow",
                "minecraft:apple",
                "minecraft:potion",
                "minecraft:shield",
                "minecraft:crossbow",
            ] {
                let item = r.item_key(id);
                r.item_part(item, UsePart(self.0.clone()));
            }
        }
        fn living_entity_use_item_start(
            &self,
            _: HostSide,
            event: &mut LivingEntityUseItemStartEvent<'_>,
        ) -> Flow {
            UsePart(self.0.clone()).heard("start", event.entity);
            if let Some(duration) = self.0.lock().unwrap().start_duration {
                event.duration = duration;
            }
            if self.0.lock().unwrap().refuse {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn living_entity_use_item_tick(
            &self,
            host: HostSide,
            event: &mut LivingEntityUseItemTickEvent<'_>,
        ) -> Flow {
            UsePart(self.0.clone()).heard("tick", event.entity);
            {
                let mut uses = self.0.lock().unwrap();
                uses.tick_remaining.push(event.duration);
                if let Some(duration) = uses.tick_duration {
                    event.duration = duration;
                }
                if uses.cancel_tick {
                    return Flow::Cancel;
                }
            }
            let who = self.0.lock().unwrap().release_on_tick;
            if let (Some(who), HostSide::Server(host)) = (who, host) {
                assert!(who.release_using_item(host));
            }
            Flow::Continue
        }
        fn living_entity_use_item_finish(
            &self,
            _: HostSide,
            event: &mut LivingEntityUseItemFinishEvent<'_>,
        ) {
            UsePart(self.0.clone()).heard("finish", event.entity);
            let mut uses = self.0.lock().unwrap();
            uses.finishes += 1;
            if uses.clear_finish_result {
                *event.result = None;
            }
        }
        fn living_entity_use_item_stop(
            &self,
            host: HostSide,
            event: &mut LivingEntityUseItemStopEvent<'_>,
        ) -> Flow {
            UsePart(self.0.clone()).heard("stop", event.entity);
            let who = {
                let mut uses = self.0.lock().unwrap();
                uses.stops.push((event.entity, event.hand, event.duration));
                uses.release_on_stop
            };
            if let (Some(who), HostSide::Server(host)) = (who, host) {
                let again = who.release_using_item(host);
                self.0.lock().unwrap().released_again.push(again);
            }
            if self.0.lock().unwrap().cancel_stop {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
}
