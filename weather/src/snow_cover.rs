//! Lying snow on every surface that faces the sky.
//!
//! The snow goes into the deferred G-buffer, between the pass that fills it
//! and the passes that light it, so it becomes part of every surface's
//! material rather than paint over the finished frame:
//!
//! - **Every material gets it** without being changed: standard materials,
//!   node-graph materials, landscapes and virtual geometry all write the same
//!   G-buffer.
//! - **It is lit like the surface it is on.** Deferred lighting, shadows,
//!   SSAO, SSR and Solari's ray-traced GI all read the G-buffer afterwards, so
//!   snow in shadow is in shadow and snow under a street lamp is lit by it.
//!
//! How much lies comes from [`WeatherState::snow_cover`], which builds while
//! it snows and melts after. A light cover shows first on flat, exposed tops
//! (roofs, ledges, the road) in drifts; a full cover reaches gentler slopes.
//!
//! **Deferred only.** A camera rendering forward has no G-buffer, and its
//! surfaces stay bare; so do forward-shaded and transparent materials (glass,
//! water), which is right for water anyway.

use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::{
    render_resource::{
        binding_types::{texture_2d, texture_depth_2d, uniform_buffer_sized},
        encase::UniformBuffer,
        BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
        BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites,
        FragmentState, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
        RenderPipelineDescriptor, ShaderStages, ShaderType, TextureDescriptor, TextureDimension,
        TextureFormat, TextureSampleType, TextureUsages,
    },
    renderer::{RenderContext, RenderDevice, ViewQuery},
    texture::{CachedTexture, TextureCache},
    view::ExtractedView,
    Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
};

use crate::data::WeatherState;

/// Bevy's G-buffer format, which the scratch copy must match.
const GBUFFER_FORMAT: TextureFormat = TextureFormat::Rgba32Uint;

/// Below this the snow is too thin to see, and both passes are skipped.
const MIN_COVER: f32 = 0.002;

pub(crate) fn build(app: &mut App) {
    bevy::asset::embedded_asset!(app, "snow_cover.wesl");
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<SnowCover>()
        .add_systems(ExtractSchedule, extract_cover)
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
        // After everything that writes the G-buffer (Bevy's deferred prepass
        // and the custom producers, such as virtual geometry, that run in the
        // same set), and before anything that reads it to light the frame.
        .add_systems(
            Core3d,
            lay_snow
                .after(Core3dSystems::Prepass)
                .before(Core3dSystems::MainPass),
        );
}

/// This frame's snow cover, in the render world.
#[derive(Resource, Default)]
struct SnowCover(f32);

fn extract_cover(mut cover: ResMut<SnowCover>, state: Extract<Option<Res<WeatherState>>>) {
    cover.0 = state.as_ref().map_or(0.0, |s| s.snow_cover);
}

#[derive(ShaderType)]
struct SnowUniform {
    world_from_clip: Mat4,
    params: Vec4,
}

#[derive(Resource)]
struct SnowPipelines {
    apply_layout: BindGroupLayoutDescriptor,
    copy_layout: BindGroupLayoutDescriptor,
    apply: CachedRenderPipelineId,
    copy: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let apply_layout = BindGroupLayoutDescriptor::new(
        "weather_snow_apply_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, texture_2d(TextureSampleType::Uint)),
                (1, texture_depth_2d()),
                (2, uniform_buffer_sized(false, None)),
            ),
        ),
    );
    let copy_layout = BindGroupLayoutDescriptor::new(
        "weather_snow_copy_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            ((3, texture_2d(TextureSampleType::Uint)),),
        ),
    );
    let shader: Handle<Shader> = asset_server.load("embedded://weather/snow_cover.wesl");
    let pipeline = |label: &'static str, layout: &BindGroupLayoutDescriptor, entry: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format: GBUFFER_FORMAT,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    };
    let apply = pipeline("weather_snow_apply", &apply_layout, "apply");
    let copy = pipeline("weather_snow_copy", &copy_layout, "copy");
    commands.insert_resource(SnowPipelines {
        apply_layout,
        copy_layout,
        apply,
        copy,
    });
}

/// A view's per-frame snow resources.
#[derive(Component)]
struct PreparedSnow {
    uniform: Buffer,
    scratch: CachedTexture,
}

fn prepare(
    mut commands: Commands,
    cover: Res<SnowCover>,
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    views: Query<(Entity, &ExtractedView, &ViewPrepassTextures)>,
) {
    for (entity, view, prepass) in &views {
        // The render world is retained, so a view keeps what an earlier frame
        // gave it: take it away once there is nothing to lay.
        let Some(gbuffer) = prepass.deferred.as_ref().filter(|_| cover.0 >= MIN_COVER) else {
            commands.entity(entity).remove::<PreparedSnow>();
            continue;
        };
        let size = gbuffer.texture.texture.size();
        let scratch = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("weather_snow_scratch"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: GBUFFER_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        let world_from_clip = view.world_from_view.to_matrix() * view.clip_from_view.inverse();
        let uniform = SnowUniform {
            world_from_clip,
            params: Vec4::new(cover.0, 0.0, 0.0, 0.0),
        };
        let mut bytes = UniformBuffer::new(Vec::<u8>::new());
        if bytes.write(&uniform).is_err() {
            continue;
        }
        let uniform = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("weather_snow_uniform"),
            contents: bytes.as_ref(),
            usage: BufferUsages::UNIFORM,
        });
        commands.entity(entity).insert(PreparedSnow { uniform, scratch });
    }
}

fn lay_snow(
    view: ViewQuery<(&ViewPrepassTextures, &PreparedSnow)>,
    cover: Res<SnowCover>,
    pipelines: Option<Res<SnowPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    if cover.0 < MIN_COVER {
        return;
    }
    let Some(pipelines) = pipelines else {
        return;
    };
    let (prepass, prepared) = view.into_inner();
    let (Some(gbuffer), Some(depth)) = (prepass.deferred.as_ref(), prepass.depth_only_view()) else {
        return;
    };
    let (Some(apply), Some(copy)) = (
        pipeline_cache.get_render_pipeline(pipelines.apply),
        pipeline_cache.get_render_pipeline(pipelines.copy),
    ) else {
        return;
    };
    let gbuffer_view = &gbuffer.texture.default_view;
    let scratch_view = &prepared.scratch.default_view;
    let device = ctx.render_device().clone();

    let apply_group = device.create_bind_group(
        "weather_snow_apply",
        &pipeline_cache.get_bind_group_layout(&pipelines.apply_layout),
        &BindGroupEntries::with_indices((
            (0, gbuffer_view),
            (1, depth),
            (2, prepared.uniform.as_entire_binding()),
        )),
    );
    let copy_group = device.create_bind_group(
        "weather_snow_copy",
        &pipeline_cache.get_bind_group_layout(&pipelines.copy_layout),
        &BindGroupEntries::with_indices(((3, scratch_view),)),
    );

    for (label, target, pipeline, group) in [
        ("weather_snow_apply", scratch_view, apply, &apply_group),
        ("weather_snow_copy", gbuffer_view, copy, &copy_group),
    ] {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}
