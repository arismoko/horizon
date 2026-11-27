use super::*;
use std::time::Duration;
use sunrise_protocol::connection::{Connection, Pipe};

#[test]
fn memory_payload_is_invisible_until_commit_and_keeps_large_body_exact() {
    let (a, b) = Pipe::pair();
    let (_, writer) = Connection::local(a).split();
    let writer = QueuedWriter::new(writer).unwrap();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let id = PayloadId("test:raw".into());
    let length = sunrise_protocol::frame::MAX_UNCOMPRESSED + 1;
    let mut body = vec![42; length];
    body[0] = 7;
    let pending = writer.prepare_payload(body, &id).unwrap();
    assert!(peer.recv().is_err(), "uncommitted packet reached the pipe");
    assert!(
        writer
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stats()
            .reserved_bytes
            >= length
    );
    assert_eq!(
        writer
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stats()
            .queued_items,
        0
    );
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    pending.publish().unwrap();
    let (packet, bytes) = peer.recv().unwrap();
    assert_eq!(packet, 7);
    assert_eq!(bytes.len(), length - 1);
    assert!(bytes.iter().all(|byte| *byte == 42));
}

#[test]
fn abort_refunds_journal_and_buffer_space_and_shutdown_prevents_publication() {
    let (a, _b) = Pipe::pair();
    let (_, writer) = Connection::local(a).split();
    let writer = QueuedWriter::new(writer).unwrap();
    let id = PayloadId("test:raw".into());
    let pending = writer
        .prepare_payload(Vec::with_capacity(10000), &id)
        .unwrap();
    assert!(
        writer
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stats()
            .reserved_bytes
            >= 10000 + PUBLICATION_OVERHEAD
    );
    drop(pending);
    assert_eq!(
        writer
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stats()
            .reserved_bytes,
        0
    );
    let pending = writer.prepare_payload(vec![7], &id).unwrap();
    writer.shutdown();
    assert_eq!(pending.publish(), Err(PayloadSendError::Disconnected));
    assert!(writer.sender.lock().unwrap().is_none());
}

#[test]
fn handoff_drains_order_cancels_permits_and_stale_clones_do_not_close() {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let queue = QueuedWriter::new(physical).unwrap();
    let stale = queue.clone();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let unpublished = queue
        .prepare_payload(vec![9, 99], &PayloadId("test:pending".into()))
        .unwrap();
    queue.send(&[1, 11]).unwrap();
    queue
        .prepare_payload(vec![2, 22], &PayloadId("test:ready".into()))
        .unwrap()
        .publish()
        .unwrap();
    let mut physical = queue.drain_and_take().unwrap();
    assert_eq!(unpublished.publish(), Err(PayloadSendError::Disconnected));
    assert!(stale.send(&[8]).is_err());
    drop(stale);
    drop(queue);
    physical.send(&[3, 33]).unwrap();
    assert_eq!(peer.recv().unwrap(), (1, vec![11]));
    assert_eq!(peer.recv().unwrap(), (2, vec![22]));
    assert_eq!(peer.recv().unwrap(), (3, vec![33]));
}

#[test]
fn crossed_large_memory_writes_never_hold_the_receive_loop() {
    let (a, b) = Pipe::pair();
    let (mut read_a, write_a) = Connection::local(a).split();
    let (mut read_b, write_b) = Connection::local(b).split();
    read_a
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    read_b
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let a = QueuedWriter::new(write_a).unwrap();
    let b = QueuedWriter::new(write_b).unwrap();
    let length = 9 * 1024 * 1024;
    let mut body = vec![41; length];
    body[0] = 7;
    let id = PayloadId("test:large".into());
    a.prepare_payload(body.clone(), &id)
        .unwrap()
        .publish()
        .unwrap();
    b.prepare_payload(body, &id).unwrap().publish().unwrap();
    for read in [&mut read_a, &mut read_b] {
        let (id, bytes) = read.recv().unwrap();
        assert_eq!(id, 7);
        assert_eq!(bytes.len(), length - 1);
        assert!(bytes.iter().all(|b| *b == 41));
    }
    a.shutdown();
    b.shutdown();
}

#[test]
fn terminal_disconnect_drains_even_when_last_handle_drops() {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let queue = QueuedWriter::new(physical).unwrap();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    queue.transition_protocol(Phase::Play, &[4]).unwrap();
    let pending = queue.prepare_packet(vec![3, 33], 7, None).unwrap();
    queue.send(&[1, 11]).unwrap();
    queue.disconnect(vec![2, 22]);
    assert_eq!(pending.try_publish(), Err(SendPacketError::NotConnected));
    assert!(queue.send(&[3]).is_err());
    drop(queue);
    assert_eq!(peer.recv().unwrap(), (4, vec![]));
    assert_eq!(peer.recv().unwrap(), (1, vec![11]));
    assert_eq!(peer.recv().unwrap(), (2, vec![22]));
    assert!(peer.recv().is_err());
}

/// Every packet the peer reads until a closing sentinel (id 0x7e), as (id, payload).
fn read_until_sentinel(peer: &mut Connection) -> Vec<(i32, Vec<u8>)> {
    let mut all = Vec::new();
    loop {
        let packet = peer.recv().unwrap();
        if packet.0 == 0x7e {
            return all;
        }
        all.push(packet);
    }
}

#[test]
fn the_writer_thread_frames_waiting_droppable_packets() {
    let (a, b) = Pipe::pair();
    let (_, writer) = Connection::local(a).split();
    let writer = QueuedWriter::new(writer).unwrap();
    let queue = crate::raw_payloads::droppable::Queue::new(u32::MAX, None);
    writer.attach_droppable(queue.clone());
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    for n in 0..3u8 {
        queue.push(
            std::time::Instant::now(),
            &Arc::default(),
            vec![0x40 + n, n],
        );
    }
    for n in 0..3u8 {
        assert_eq!(peer.recv().unwrap(), (i32::from(0x40 + n), vec![n]));
    }
}

#[test]
fn a_dropped_droppable_leaves_the_framed_stream_as_if_never_sent() {
    let run = |push: bool| {
        let (a, b) = Pipe::pair();
        let (_, writer) = Connection::local(a).split();
        let writer = QueuedWriter::new(writer).unwrap();
        // A zero budget drops every droppable send at the queue.
        let queue = crate::raw_payloads::droppable::Queue::new(0, None);
        writer.attach_droppable(queue.clone());
        let mut peer = Connection::local(b);
        peer.reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        writer.send(&[3, 1, 2]).unwrap();
        if push {
            queue.push(std::time::Instant::now(), &Arc::default(), vec![0x40, 9]);
        }
        writer.send(&[4, 5]).unwrap();
        writer.send(&[0x7e]).unwrap();
        read_until_sentinel(&mut peer)
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn closing_the_writer_seals_prepared_droppable_publication_in_its_drain() {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let writer = QueuedWriter::new(physical).unwrap();
    let lane = crate::raw_payloads::droppable::Queue::new(u32::MAX, None);
    writer.attach_droppable(lane.clone());
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // The early reliable frame exceeds the 64 KiB pipe; it cannot drain until
    // this test starts reading, after the cutoff and attempted publication.
    let mut early = vec![42; 128 * 1024];
    early[0] = 1;
    writer.send(&early).unwrap();
    let late = crate::raw_payloads::Prepared::Droppable(lane.clone(), Arc::default(), vec![2, 21]);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while writer.probe[3].load(Ordering::Relaxed) == 0 {
        assert!(std::time::Instant::now() < deadline, "writer did not start");
        std::thread::yield_now();
    }
    writer.close_after_pending();
    let before_publish = std::time::Instant::now();
    late.publish().unwrap();
    assert!(lane.pop(before_publish).is_none());
    let (id, bytes) = peer.recv().unwrap();
    assert_eq!(id, 1);
    assert_eq!(bytes, early[1..]);
    assert!(peer.recv().unwrap_err().is_end_of_stream());
}

#[test]
fn exhausted_held_play_credit_still_serves_control_replies_and_reconfiguration_ack() {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let writer = QueuedWriter::new(physical).unwrap();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writer.transition_protocol(Phase::Play, &[4]).unwrap();
    assert_eq!(peer.recv().unwrap(), (4, vec![]));

    let mut held = writer.exhaust_play_credit(7);
    assert!(matches!(
        writer.prepare_packet(vec![1], 7, None),
        Err(PrepareRefusal::Held)
    ));

    // Excluded replies still drain through the existing writer while held permits stay unpublished.
    writer.send_bypassing_held(&[11, 1]).unwrap();
    assert_eq!(peer.recv().unwrap(), (11, vec![1]));
    writer.send_bypassing_held(&[12, 2]).unwrap();
    assert_eq!(peer.recv().unwrap(), (12, vec![2]));

    // Reconfiguration revokes unpublished old-play permits before admitting the ACK,
    // even before the actor drops any held PreparedPackets.
    writer
        .transition_protocol(Phase::Configuration, &[9])
        .unwrap();
    assert_eq!(peer.recv().unwrap(), (9, vec![]));
    assert_eq!(
        held.pop().unwrap().try_publish(),
        Err(SendPacketError::NotInPlay)
    );

    writer.transition_protocol(Phase::Play, &[10]).unwrap();
    assert_eq!(peer.recv().unwrap(), (10, vec![]));
    drop(held);

    let stats = writer.sender.lock().unwrap().as_ref().unwrap().stats();
    assert_eq!(stats.queued_items, 0);
    assert_eq!(stats.reserved_bytes, 0);

    writer
        .prepare_packet(vec![5, 55], 7, Some(3))
        .unwrap()
        .try_publish()
        .unwrap();
    assert_eq!(peer.recv().unwrap(), (5, vec![55]));
}

#[test]
fn close_and_wait_drains_pending_writes_and_closes_transport_before_returning() {
    let (a, b) = Pipe::pair();
    let (_, physical) = Connection::local(a).split();
    let writer = QueuedWriter::new(physical).unwrap();
    let mut peer = Connection::local(b);
    peer.reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    writer.send(&[1, 11]).unwrap();
    writer.close_and_wait();
    assert!(!writer.accepts_packets());
    assert_eq!(peer.recv().unwrap(), (1, vec![11]));
    assert!(peer.recv().unwrap_err().is_end_of_stream());
    writer.close_and_wait();
}
