//! Liquid Glass widget: a wgpu shader that refracts what is drawn behind it.
//!
//! Put it under the content of a control in a `stack!`, above the page:
//! iced draws each stack child on its own layer, so when the glass renders,
//! everything behind it is already in the window texture. The glass copies
//! that area (the iced_wgpu patch in /vendor allows the copy) and refracts it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::widget::shader::{self, Viewport};
use iced::{Color, Element, Length, Rectangle, mouse, wgpu};

/// Distance the shader may sample outside the glass (refraction + blur),
/// in logical px. Matches `bend` + colour split + `blur` in glass.wgsl.
const MARGIN: f32 = 32.0;

/// A glass surface. `id` must be unique among the glass surfaces on screen.
pub fn glass<'a, Message: 'a>(id: u64, radius: f32, tint: Color) -> Element<'a, Message> {
    iced::widget::shader(Glass { id, radius, tint })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

struct Glass {
    id: u64,
    radius: f32,
    tint: Color,
}

impl<Message> shader::Program<Message> for Glass {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Primitive {
        Primitive {
            id: self.id,
            radius: self.radius,
            tint: self.tint,
        }
    }
}

#[derive(Debug)]
pub struct Primitive {
    id: u64,
    radius: f32,
    tint: Color,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    rect: [f32; 4],
    region: [f32; 4],
    params: [f32; 4],
    tint: [f32; 4],
    flags: [f32; 4],
}

impl Uniforms {
    fn bytes(&self) -> &[u8] {
        // SAFETY: plain #[repr(C)] f32 arrays, no padding.
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// GPU resources of one glass surface.
struct Instance {
    uniforms: wgpu::Buffer,
    backdrop: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    /// Area of the window copied into `backdrop`, physical px.
    region: [u32; 4],
    used: bool,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    instances: HashMap<u64, Instance>,
    /// Whether the window texture can be copied. Learned on the first render;
    /// if it cannot, the next frames draw a plain tint.
    can_copy: AtomicBool,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glass"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glass"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            format,
            instances: HashMap::new(),
            can_copy: AtomicBool::new(true),
        }
    }

    fn trim(&mut self) {
        self.instances
            .retain(|_, instance| std::mem::take(&mut instance.used));
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let scale = viewport.scale_factor();
        let size = viewport.physical_size();
        let rect = *bounds * scale;

        // The window area the shader samples: the glass plus a margin, inside the window.
        let margin = MARGIN * scale;
        let x0 = (rect.x - margin).floor().clamp(0.0, size.width as f32) as u32;
        let y0 = (rect.y - margin).floor().clamp(0.0, size.height as f32) as u32;
        let x1 = (rect.x + rect.width + margin)
            .ceil()
            .clamp(0.0, size.width as f32) as u32;
        let y1 = (rect.y + rect.height + margin)
            .ceil()
            .clamp(0.0, size.height as f32) as u32;
        let region = [x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)];

        let stale = pipeline.instances.get(&self.id).is_none_or(|instance| {
            instance.region[2] != region[2] || instance.region[3] != region[3]
        });
        if stale {
            let instance = new_instance(pipeline, device, region);
            pipeline.instances.insert(self.id, instance);
        }
        let instance = pipeline
            .instances
            .get_mut(&self.id)
            .expect("inserted above");
        instance.region = region;
        instance.used = true;

        let uniforms = Uniforms {
            rect: [rect.x, rect.y, rect.width, rect.height],
            region: region.map(|v| v as f32),
            params: [
                size.width as f32,
                size.height as f32,
                self.radius * scale,
                scale,
            ],
            tint: [self.tint.r, self.tint.g, self.tint.b, self.tint.a],
            flags: [
                f32::from(u8::from(pipeline.can_copy.load(Ordering::Relaxed))),
                0.0,
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&instance.uniforms, 0, uniforms.bytes());
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(instance) = pipeline.instances.get(&self.id) else {
            return;
        };
        let window = target.texture();
        if window.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            let [x, y, width, height] = instance.region;
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: window,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                instance.backdrop.as_image_copy(),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        } else {
            pipeline.can_copy.store(false, Ordering::Relaxed);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("glass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        });
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &instance.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

fn new_instance(pipeline: &Pipeline, device: &wgpu::Device, region: [u32; 4]) -> Instance {
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass uniforms"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let backdrop = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass backdrop"),
        size: wgpu::Extent3d {
            width: region[2],
            height: region[3],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: pipeline.format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = backdrop.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glass"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
            },
        ],
    });
    Instance {
        uniforms,
        backdrop,
        bind_group,
        region,
        used: true,
    }
}
