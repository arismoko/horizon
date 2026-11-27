//! Control data and trace for the interaction matrix.
//! Control data: [use_block, use_item, before, block, after, right_click]. Switches are
//! Default/True/False = 0/1/2; answers are Continue/Pass/Fail/Success/
//! SuccessServer/Consume/TryWithEmptyHand = 0/1/2/3/4/5/6.
#[cfg(feature = "client")]
use mod_api::{Flow, Hand, HostSide, InteractionResult, LogicalSide, TriState, UsePhase};
#[cfg(feature = "client")]
use std::sync::Mutex;

#[cfg(feature = "client")]
static STATE: Mutex<([u8; 6], Vec<u8>)> = Mutex::new(([0; 6], Vec::new()));

#[cfg(feature = "client")]
fn answer(value: u8) -> Option<InteractionResult> {
    Some(match value {
        0 => return None,
        1 => InteractionResult::Pass,
        2 => InteractionResult::Fail,
        3 => InteractionResult::Success,
        4 => InteractionResult::SuccessServer,
        5 => InteractionResult::Consume,
        6 => InteractionResult::TryWithEmptyHand,
        _ => panic!("invalid fixture answer"),
    })
}

#[cfg(feature = "client")]
fn switch(value: u8) -> TriState {
    match value {
        0 => TriState::Default,
        1 => TriState::True,
        2 => TriState::False,
        _ => panic!("invalid fixture switch"),
    }
}

#[cfg(feature = "client")]
fn record(trace: &mut Vec<u8>, phase: u8, hand: Hand, side: LogicalSide, empty: bool) {
    trace.extend([
        phase,
        u8::from(hand == Hand::Off),
        u8::from(side == LogicalSide::Client),
        u8::from(empty),
    ]);
}

#[cfg(feature = "client")]
pub fn right_click_block(_: HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> Flow {
    let mut state = STATE.lock().unwrap();
    *event.use_block = switch(state.0[0]);
    *event.use_item = switch(state.0[1]);
    record(
        &mut state.1,
        0,
        event.click.hand,
        event.click.side,
        event.click.stack.is_none(),
    );
    if let Some(answer) = answer(state.0[5]) {
        *event.cancellation_result = answer;
        Flow::Cancel
    } else {
        Flow::Continue
    }
}

#[cfg(feature = "client")]
pub fn use_item_on_block(_: HostSide<'_>, event: &mut mod_api::UseItemOnBlockEvent<'_>) -> Flow {
    let phase = match event.phase {
        UsePhase::ItemBeforeBlock => 1,
        UsePhase::Block => 2,
        UsePhase::ItemAfterBlock => 3,
        _ => unreachable!("unknown fixture phase"),
    };
    let mut state = STATE.lock().unwrap();
    record(
        &mut state.1,
        phase,
        event.click.hand,
        event.click.side,
        event.click.stack.is_none(),
    );
    let Some(answer) = answer(state.0[phase as usize + 1]) else {
        return Flow::Continue;
    };
    *event.cancellation_result = answer;
    Flow::Cancel
}

#[cfg(feature = "client")]
pub(crate) fn begin(controls: [u8; 6]) {
    let mut state = STATE.lock().unwrap();
    state.0 = controls;
    state.1.clear();
}

#[cfg(feature = "client")]
pub(crate) fn finish() -> Vec<u8> {
    std::mem::take(&mut STATE.lock().unwrap().1)
}
