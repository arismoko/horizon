use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, sync_channel};
use std::time::{Duration, Instant};

use sunrise_client_gameplay::game::ClientGame;
use sunrise_client_gameplay::game::client_side::integrated_client::Integrated;
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay::game::native_guest_turn_tests::open_fixture_hub;
use sunrise_gameplay::game::server_side::integrated::{
    ClientHandoff, Serving, run_test_owner_tasks, test_owner_request,
};
use sunrise_gameplay::game::server_side::lan::Owner;
use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
use sunrise_protocol::crypto::offline_uuid;
use sunrise_sim::ticket_levels::{PLAYER_SPAWN_RADIUS, SpawnTicketKind};

use super::server_side::native_entity_refs::tests::{with_client_game, with_resident_game};

const TIMEOUT: Duration = Duration::from_secs(15);
const PLAYER: &str = "IntegratedPair";

pub(super) struct IntegratedPair<'test, 'server, 'client> {
    pub client: &'test mut ClientGame<'client>,
    server: &'test mut ServerGame<'server>,
}

pub(super) fn with_integrated_pair(test: impl FnOnce(&mut IntegratedPair<'_, '_, '_>)) {
    with_integrated_pair_at([0.5, 64.0, 0.5], test);
}

pub(super) fn with_integrated_pair_at(
    position: [f64; 3],
    test: impl FnOnce(&mut IntegratedPair<'_, '_, '_>),
) {
    with_client_game(|client| {
        with_resident_game(|server| {
            for z in -PLAYER_SPAWN_RADIUS..=PLAYER_SPAWN_RADIUS {
                for x in -PLAYER_SPAWN_RADIUS..=PLAYER_SPAWN_RADIUS {
                    let key = (x, z);
                    fixture_resident_column(&mut server.world, key);
                    std::sync::Arc::make_mut(server.world.chunks.get_mut(&key).unwrap()).lit = true;
                }
            }
            let mut saved = server.level.clone();
            saved.position = Some(position);
            saved.dimension = server.dim.id().to_owned();
            saved.player_extra = Some(saved.player_tag(server.res.items()));
            server
                .server
                .tickets
                .0
                .add_spawn((0, 0), SpawnTicketKind::Player);
            open_fixture_hub(server);
            let owner = offline_uuid(PLAYER);
            let (tasks, receiver) = mpsc::channel();
            let mut integrated = Integrated::stopped_for_test();
            integrated.tasks = tasks;
            let mut serving = Serving::fixture(0);
            serving.flags = integrated.flags.clone();
            serving.tasks = receiver;
            serving.owner = owner;
            server.server.serving = Some(serving);
            let host = server.server.lan.as_mut().unwrap();
            host.hub.singleplayer_owner = Some(owner);
            host.owner = Some(Owner {
                uuid: owner,
                saved: Some(Box::new(saved)),
            });
            // The menu fixture inserts synthetic guests on connections 1 and 2.
            host.hub.lan.shared.next_id().unwrap();
            host.hub.lan.shared.next_id().unwrap();
            let pipe = host.hub.lan.connect_local().unwrap();
            client.join_world_rest().unwrap();
            client.connect_local(pipe, PLAYER).unwrap();
            client.client.integrated = Some(integrated);

            let mut pair = IntegratedPair { client, server };
            let deadline = Instant::now() + TIMEOUT;
            while !pair
                .client
                .client
                .remote
                .as_ref()
                .is_some_and(|remote| remote.logged_in)
                || pair.client.player.is_none()
            {
                assert!(
                    Instant::now() < deadline,
                    "integrated fixture login timed out"
                );
                pair.tick_server(true);
                pair.pump_client();
                std::thread::yield_now();
            }
            test(&mut pair);
        });
    });
}

impl IntegratedPair<'_, '_, '_> {
    pub fn call<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut ServerGame<'_>) -> T + Send + 'static,
    ) -> T {
        let tasks = self.client_sender();
        run(
            self.server,
            &mut |_| panic!("owner callback requires a client handoff"),
            move |server, _| f(server),
            tasks,
        )
    }

    pub fn call_with_client<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut ServerGame<'_>, &mut ClientHandoff<'_>) -> T + Send + 'static,
        mut observe: impl FnMut(&mut ClientGame<'_>),
    ) -> T {
        let tasks = self.client_sender();
        let client = &mut *self.client;
        let mut handoff = |server: &mut ServerGame<'_>| {
            observe_published_packets(client, server);
            observe(client);
        };
        run(self.server, &mut handoff, f, tasks)
    }

    fn client_sender(
        &self,
    ) -> mpsc::Sender<sunrise_gameplay_common::game::owner_link::OwnerRequest> {
        self.client
            .client
            .integrated
            .as_ref()
            .unwrap()
            .tasks
            .clone()
    }

    pub fn tick_server(&mut self, paused: bool) {
        run_test_owner_tasks(self.server, &mut |_| panic!("unexpected client handoff"));
        self.server.tick_server(paused).unwrap();
    }

    pub fn pump_client(&mut self) {
        self.client.poll_remote().unwrap();
    }
}

fn run<T: Send + 'static>(
    server: &mut ServerGame<'_>,
    handoff: &mut ClientHandoff<'_>,
    f: impl FnOnce(&mut ServerGame<'_>, &mut ClientHandoff<'_>) -> T + Send + 'static,
    tasks: mpsc::Sender<sunrise_gameplay_common::game::owner_link::OwnerRequest>,
) -> T {
    let (answer, result) = sync_channel(1);
    let sent = tasks.send(test_owner_request(move |server, handoff| {
        assert!(answer.send(f(server, handoff)).is_ok());
    }));
    assert!(sent.is_ok(), "send owner test task");
    run_test_owner_tasks(server, handoff);
    result.try_recv().expect("owner test task reply")
}

fn observe_published_packets(client: &mut ClientGame<'_>, server: &ServerGame<'_>) {
    let owner = server.server.serving.as_ref().unwrap().owner;
    let host = server.server.lan.as_ref().unwrap();
    let guest = host
        .hub
        .online_players()
        .find(|guest| guest.uuid == owner)
        .unwrap();
    let published = guest.sender.test_published_packets();
    let counters = client
        .client
        .remote
        .as_ref()
        .unwrap()
        .session
        .counters
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let ready = guest
            .sender
            .test_framed_through(published)
            .is_some_and(|wire_packets| {
                counters.play_ingress_received.load(Ordering::Acquire) >= wire_packets
            });
        // Sample reader progress before dispatch so that an ingress turn cannot miss a ready packet.
        if client.test_dispatch_ready_packets().unwrap() && ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "owner packet observation timed out"
        );
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "daily"]
fn inventory_boxes_follow_menu_indices_and_the_input_layout() {
    use sunrise_client_gameplay::game::client_side::native_menu_refs::tests::{
        check_inventory_boxes_follow_menu_indices_and_the_input_layout, with_panel,
    };
    with_panel(
        0,
        check_inventory_boxes_follow_menu_indices_and_the_input_layout,
    );
}
