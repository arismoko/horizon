//! Client prediction controls: [path, answer, server_only]; read back packet counts.
use mod_api::{Hand, HostSide, InteractionResult as Answer};
use std::sync::Mutex;

pub(crate) static STATE: Mutex<([i32; 3], [i32; 3])> = Mutex::new(([0; 3], [0; 3]));

fn answer(path: i32, host: HostSide<'_>, hand: Hand) -> Option<Answer> {
    let state = STATE.lock().unwrap();
    if state.0[0] != path
        || hand != Hand::Main
        || (state.0[2] == 1 && matches!(host, HostSide::Client(_)))
    {
        return None;
    }
    Some(match state.0[1] {
        1 => Answer::Success,
        2 => Answer::SuccessServer,
        3 => Answer::Consume,
        4 => Answer::Fail,
        5 => Answer::Pass,
        6 => Answer::TryWithEmptyHand,
        _ => return None,
    })
}

pub fn entity_interact(
    host: HostSide<'_>,
    event: &mut mod_api::EntityInteractEvent<'_>,
) -> Option<Answer> {
    answer(3, host, event.hand)
}

pub fn packet(event: &mod_api::ClientPacketSendEvent<'_>) {
    let mut state = STATE.lock().unwrap();
    if state.0[0] == 0 {
        return;
    }
    let index = match event.packet.id.path() {
        "use_item_on" => 0,
        "use_item" => 1,
        "interact" => 2,
        _ => return,
    };
    state.1[index] += 1;
}

pub struct Part;
#[mod_api::sunrise_part]
impl mod_api::ItemPart for Part {
    fn use_on(&self, host: HostSide<'_>, event: &mut mod_api::UseOnEvent<'_>) -> Option<Answer> {
        answer(1, host, event.hand)
    }
    fn use_item(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::UseItemEvent<'_>,
    ) -> Option<Answer> {
        answer(2, host, event.hand)
    }
    fn item_interact_mob(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::ItemInteractMobEvent<'_>,
    ) -> Option<Answer> {
        answer(4, host, event.hand)
    }
}

pub(crate) fn begin(path: i32, answer: i32, server_only: bool) {
    *STATE.lock().unwrap() = ([path, answer, i32::from(server_only)], [0; 3]);
}

pub(crate) fn finish() -> [i32; 3] {
    let mut state = STATE.lock().unwrap();
    state.0 = [0; 3];
    state.1
}

pub fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:paper");
    registrar.item_part(item, Part);
}
