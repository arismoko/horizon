use super::*;
use std::sync::atomic::Ordering::Relaxed;

fn counters() -> Arc<DropCounters> {
    Arc::default()
}
fn drain(queue: &Queue, now: Instant) -> Vec<Vec<u8>> {
    std::iter::from_fn(|| queue.pop(now)).collect()
}

#[test]
fn a_dropped_send_leaves_the_stream_as_if_never_sent() {
    let (now, c) = (Instant::now(), counters());
    // The middle send is over the budget and dropped; the stream equals one without it.
    let with = Queue::new(20, None);
    for body in [vec![1u8; 10], vec![2; 15], vec![3; 5]] {
        with.push(now, &c, body);
    }
    let without = Queue::new(20, None);
    for body in [vec![1u8; 10], vec![3; 5]] {
        without.push(now, &c, body);
    }
    assert_eq!(drain(&with, now), drain(&without, now));
    assert_eq!(c.budget.load(Relaxed), 1);
}

#[test]
fn stale_sends_go_first_oldest_first_then_the_new_one() {
    let (start, c) = (Instant::now(), counters());
    let queue = Queue::new(30, None);
    queue.push(start, &c, vec![1; 10]);
    queue.push(start, &c, vec![2; 10]);
    let later = start + STALE + Duration::from_millis(1);
    // Over budget at `later`, the two stale ones are dropped and refunded, so this fits.
    queue.push(later, &c, vec![3; 25]);
    assert_eq!(c.stale.load(Relaxed), 2);
    assert_eq!(drain(&queue, later), vec![vec![3; 25]]);
    // Nothing stale left: the new over-budget send is the one dropped.
    queue.push(later, &c, vec![4; 25]);
    assert_eq!(c.budget.load(Relaxed), 1);
}

#[test]
fn a_stale_waiting_send_is_dropped_not_sent() {
    let (start, c) = (Instant::now(), counters());
    let queue = Queue::new(1 << 20, None);
    queue.push(start, &c, vec![1; 10]);
    queue.push(start + STALE, &c, vec![2; 10]);
    let now = start + STALE + Duration::from_millis(1);
    assert_eq!(drain(&queue, now), vec![vec![2; 10]]);
    assert_eq!(c.stale.load(Relaxed), 1);
}

#[test]
fn the_watermark_drops_the_oldest_and_order_holds() {
    let (now, c) = (Instant::now(), counters());
    let queue = Queue::new(u32::MAX, None);
    let size = WATERMARK / 4;
    for n in 0..6u8 {
        queue.push(now, &c, vec![n; size]);
    }
    let kept: Vec<u8> = drain(&queue, now).iter().map(|b| b[0]).collect();
    assert_eq!(kept, [2, 3, 4, 5]);
    assert_eq!(c.watermark.load(Relaxed), 2);
}

#[test]
fn clear_counts_the_switch_and_zero_budget_turns_delivery_off() {
    let (now, c) = (Instant::now(), counters());
    let queue = Queue::new(u32::MAX, None);
    queue.push(now, &c, vec![1]);
    queue.clear();
    assert_eq!(c.switch.load(Relaxed), 1);
    assert!(queue.pop(now).is_none());
    let off = Queue::new(0, Some(0));
    off.push(now, &c, vec![1]);
    assert_eq!(c.budget.load(Relaxed), 1);
    assert!(!off.admit(now, 1));
}

#[test]
fn the_budget_refills_over_time() {
    let (now, c) = (Instant::now(), counters());
    let queue = Queue::new(100, None);
    queue.push(now, &c, vec![0; 100]);
    queue.push(now, &c, vec![0; 50]);
    assert_eq!(c.budget.load(Relaxed), 1);
    queue.push(now + Duration::from_millis(500), &c, vec![0; 50]);
    assert_eq!(c.budget.load(Relaxed), 1);
}

#[test]
fn budgets_read_their_settings_and_keep_the_default_for_a_bad_value() {
    let get = |name: &str| match name {
        "SUNRISE_DROPPABLE_CLIENTBOUND_BYTES_PER_SECOND" => Some(" 1000 ".to_owned()),
        "SUNRISE_DROPPABLE_SERVERBOUND_BYTES_PER_SECOND" => Some("lots".to_owned()),
        _ => None,
    };
    let budgets = Budgets::read(get);
    assert_eq!(budgets.clientbound, 1000);
    assert_eq!(budgets.serverbound, SERVERBOUND_BUDGET);
    let unset = Budgets::read(|_| None);
    assert_eq!(
        (unset.clientbound, unset.serverbound),
        (CLIENTBOUND_BUDGET, SERVERBOUND_BUDGET)
    );
    assert_eq!(Budgets::read(|_| Some("0".into())).clientbound, 0);
}
