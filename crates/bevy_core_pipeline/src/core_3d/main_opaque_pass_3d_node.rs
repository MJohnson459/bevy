use crate::{
    clear_color::{ClearColor, ClearColorConfig},
    core_3d::{Camera3d, Opaque3d},
    prepass::{DeferredPrepass, DepthPrepass, MotionVectorPrepass, NormalPrepass},
    skybox::{SkyboxBindGroup, SkyboxPipelineId},
};
use bevy_ecs::{prelude::*, query::QueryItem};
use bevy_render::{
    camera::ExtractedCamera,
    render_graph::{NodeRunError, RenderGraphContext, ViewNode},
    render_phase::RenderPhase,
    render_resource::{
        LoadOp, Operations, PipelineCache, RenderPassDepthStencilAttachment, RenderPassDescriptor,
        StoreOp,
    },
    renderer::RenderContext,
    view::{ViewDepthTexture, ViewTarget, ViewUniformOffset},
};
#[cfg(feature = "trace")]
use bevy_utils::tracing::info_span;

use super::{AlphaMask3d, Camera3dDepthLoadOp};

/// A [`bevy_render::render_graph::Node`] that runs the [`Opaque3d`] and [`AlphaMask3d`] [`RenderPhase`].
#[derive(Default)]
pub struct MainOpaquePass3dNode;
impl ViewNode for MainOpaquePass3dNode {
    type ViewData = (
        &'static ExtractedCamera,
        &'static RenderPhase<Opaque3d>,
        &'static RenderPhase<AlphaMask3d>,
        &'static Camera3d,
        &'static ViewTarget,
        &'static ViewDepthTexture,
        Option<&'static DepthPrepass>,
        Option<&'static NormalPrepass>,
        Option<&'static MotionVectorPrepass>,
        Option<&'static DeferredPrepass>,
        Option<&'static SkyboxPipelineId>,
        Option<&'static SkyboxBindGroup>,
        &'static ViewUniformOffset,
    );

    fn run(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (
            camera,
            opaque_phase,
            alpha_mask_phase,
            camera_3d,
            target,
            depth,
            depth_prepass,
            normal_prepass,
            motion_vector_prepass,
            deferred_prepass,
            skybox_pipeline,
            skybox_bind_group,
            view_uniform_offset,
        ): QueryItem<Self::ViewData>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let load = if deferred_prepass.is_none() {
            match camera_3d.clear_color {
                ClearColorConfig::Default => LoadOp::Clear(world.resource::<ClearColor>().0.into()),
                ClearColorConfig::Custom(color) => LoadOp::Clear(color.into()),
                ClearColorConfig::None => LoadOp::Load,
            }
        } else {
            // If the deferred lighting pass has run, don't clear again in this pass.
            LoadOp::Load
        };

        // Run the opaque pass, sorted front-to-back
        // NOTE: Scoped to drop the mutable borrow of render_context
        #[cfg(feature = "trace")]
        let _main_opaque_pass_3d_span = info_span!("main_opaque_pass_3d").entered();

        // Setup render pass
        let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("main_opaque_pass_3d"),
            // NOTE: The opaque pass loads the color
            // buffer as well as writing to it.
            color_attachments: &[Some(target.get_color_attachment(Operations {
                load,
                store: StoreOp::Store,
            }))],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth.view,
                // NOTE: The opaque main pass loads the depth buffer and possibly overwrites it
                depth_ops: Some(Operations {
                    load: if depth_prepass.is_some()
                        || normal_prepass.is_some()
                        || motion_vector_prepass.is_some()
                        || deferred_prepass.is_some()
                    {
                        // if any prepass runs, it will generate a depth buffer so we should use it,
                        // even if only the normal_prepass is used.
                        Camera3dDepthLoadOp::Load
                    } else {
                        // NOTE: 0.0 is the far plane due to bevy's use of reverse-z projections.
                        camera_3d.depth_load_op.clone()
                    }
                    .into(),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if let Some(viewport) = camera.viewport.as_ref() {
            render_pass.set_camera_viewport(viewport);
        }

        let view_entity = graph.view_entity();

        // Opaque draws
        opaque_phase.render(&mut render_pass, world, view_entity);

        // Alpha draws
        if !alpha_mask_phase.items.is_empty() {
            alpha_mask_phase.render(&mut render_pass, world, view_entity);
        }

        // Draw the skybox using a fullscreen triangle
        if let (Some(skybox_pipeline), Some(skybox_bind_group)) =
            (skybox_pipeline, skybox_bind_group)
        {
            let pipeline_cache = world.resource::<PipelineCache>();
            if let Some(pipeline) = pipeline_cache.get_render_pipeline(skybox_pipeline.0) {
                render_pass.set_render_pipeline(pipeline);
                render_pass.set_bind_group(0, &skybox_bind_group.0, &[view_uniform_offset.offset]);
                render_pass.draw(0..3, 0..1);
            }
        }

        Ok(())
    }
}
