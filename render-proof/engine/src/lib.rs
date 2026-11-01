use horizon_client::mod_api::{
    BlockEntityDraw, BlockStateId, CubeFace, Hooks, Host, ModBlockEntity, RenderHooks, RenderNeeds,
    WorldDrawn, wgpu,
};

struct Proof;
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Proof)
}
impl Hooks for Proof {
    fn render(&self) -> Option<&dyn RenderHooks> {
        Some(self)
    }
    fn block_entity_submit(
        &self,
        host: &dyn Host,
        _: [i32; 3],
        block: &str,
        _: BlockStateId,
        _: &ModBlockEntity,
        draw: &mut BlockEntityDraw,
    ) {
        if block == "machine_sample:machine" {
            eprintln!("render-proof: block_entity_submit");
            let face = Some(CubeFace {
                sprite: "block/stone",
                uv: None,
            });
            draw.translate([0.0, 1.0, 0.0]);
            host.draw_cube(draw, [0.0; 3], [16.0; 3], [face; 6], 0xffffff);
        }
    }
}
impl RenderHooks for Proof {
    fn render_needs(&self, needs: &mut RenderNeeds) {
        eprintln!("render-proof: render_needs");
        needs.world_depth = true;
    }
    fn world_drawn(&self, world: &mut WorldDrawn<'_>) {
        eprintln!("render-proof: world_drawn");
        let shader = world.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl("@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4f { var p = array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.)); return vec4f(p[i],0.,1.); } @fragment fn fs()->@location(0) vec4f { return vec4f(1.,0.,1.,1.); }".into()) });
        let pipeline = world
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(world.color_format.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
        let mut pass = world
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: world.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(&pipeline);
        pass.set_viewport(0., 0., 96., 96., 0., 1.);
        pass.draw(0..3, 0..1);
    }
}
