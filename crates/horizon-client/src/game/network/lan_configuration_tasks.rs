//! Run configuration task registration on the server thread before admitting a guest.

use super::{Game, lan::LanHost};
use horizon_client::lan::{Lan, NetConfig, data::SyncData};
use std::{io, sync::Arc, time::Instant};

pub(super) fn start(sync: Arc<SyncData>, net: NetConfig) -> io::Result<Lan> {
    Lan::start_with_configuration_tasks(
        sync,
        net,
        mod_api::__private::listened(mod_api::HookId::RegisterConfigurationTasks),
    )
}

impl Game<'_> {
    pub(super) fn take_lan_for_packet_tick(&mut self) -> Option<LanHost> {
        if self.level_pass.first
            && let Some(host) = &self.lan
        {
            // The hook has its normal server host, but the joining profile
            // stays on the connection thread until its tasks finish.
            let requests: Vec<_> = host.hub.lan.configuration_requests().collect();
            for request in requests {
                if Instant::now() < request.deadline {
                    let mut tasks = Vec::new();
                    self.native_register_configuration_tasks(
                        request.player,
                        &request.name,
                        &request.brand,
                        &mut tasks,
                    );
                    let _ = request.reply.send(tasks);
                }
            }
        }
        self.lan.take()
    }
}
