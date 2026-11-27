use crate::game::ClientGame;
use serde_json::Value;
use std::sync::atomic::Ordering;
use sunrise_client_render::gfx::RendererMemorySnapshot;
use sunrise_gameplay_common::game::memory_common::*;
use sunrise_gameplay_common::game::owner_link::{ControlRequest, OwnerRequest};
use sunrise_sim::server_perf::MemoryCost;

impl ClientGame<'_> {
    pub(in crate::game) fn client_memory_inputs(&self) -> MemoryInputs {
        let far = self.client.lod.0.as_ref().map(|lod| {
            let ((active, cached), (kept, kept_bytes)) = (lod.stats(), lod.kept());
            let tiles = FarTiles {
                active,
                cached,
                kept,
                kept_mb: mb(kept_bytes),
            };
            (tiles, kept_bytes)
        });
        MemoryInputs {
            render_distance: self.client.streaming.radius,
            client: self
                .common
                .options
                .dedicated
                .is_none()
                .then(|| self.common.world.chunks.values().cloned().collect()),
            far,
        }
    }

    pub(in crate::game) fn client_memory_report(&self) -> MemoryReport {
        let inputs = self.client_memory_inputs();
        let renderer = self.client.renderer.0.as_ref().map(|r| r.memory_snapshot());
        let server =
            self.client.integrated.as_ref().and_then(|i| {
                i.ask(|reply| OwnerRequest::Control(ControlRequest::ServerSide(reply)))
            });
        inputs
            .report(server, || true, || renderer_report(renderer))
            .expect("a manual census cannot be canceled")
    }
}

pub(in crate::game) fn start(
    game: &crate::game::ClientGame<'_>,
    cost: MemoryCost,
) -> Option<PendingMemory> {
    cost.measure(|| {
        let inputs = game.client_memory_inputs();
        let renderer = game.client.renderer.0.as_ref().map(|r| r.memory_snapshot());
        let (job, request) = match PendingMemory::spawn(
            cost.clone(),
            game.client.integrated.is_some(),
            move |server, cancelled| {
                let report = inputs.report(
                    server,
                    || !cancelled.load(Ordering::Acquire),
                    || renderer_report(renderer),
                )?;
                Some(flatten_report(
                    &serde_json::to_value(report).unwrap_or(Value::Null),
                ))
            },
        ) {
            Ok(job) => job,
            Err(error) => {
                log::warn!("Sunrise: could not start the memory census: {error}");
                return None;
            }
        };
        if let Some((server, request)) = game.client.integrated.as_ref().zip(request) {
            server.tell(OwnerRequest::Control(ControlRequest::MemorySample(request)));
        }
        Some(job)
    })
}

fn renderer_report(snapshot: Option<RendererMemorySnapshot>) -> Option<(Renderer, usize)> {
    snapshot.map(|r| {
        let [arenas, vertices] = r.section_vertex_bytes();
        let cpu = r.cpu_census();
        let cpu_bytes: usize = cpu.iter().map(|(_, v)| *v).sum();
        let gpu = r
            .gpu_memory_by_label()
            .map(|(allocated, reserved, labels)| Gpu {
                allocated_mb: mb(allocated),
                reserved_mb: mb(reserved),
                by_label_mb: labels
                    .into_iter()
                    .take(40)
                    .map(|(label, bytes)| (label, mb(bytes)))
                    .collect(),
            });
        (
            Renderer {
                section_arenas_mb: mb(arenas),
                section_vertices_mb: mb(vertices),
                cpu: cpu
                    .into_iter()
                    .map(|(k, v)| (format!("{k}_mb"), mb(v)))
                    .collect(),
                gpu,
            },
            cpu_bytes,
        )
    })
}
