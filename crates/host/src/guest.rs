//! Draws guest frames into an offscreen texture and shows that texture in the host.

use eframe::egui_wgpu::{wgpu, RenderState, Renderer, RendererOptions, ScreenDescriptor};
use egui::epaint::{ClippedPrimitive, Primitive};
use pg_abi::FrameOutput;

/// The format the host renderer needs for a texture registered with `register_native_texture`.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct GuestView {
    /// Renders guest meshes onto [`GuestView::texture`], separate from the host renderer so the
    /// two texture id spaces stay apart.
    renderer: Renderer,

    texture: wgpu::Texture,
    view: wgpu::TextureView,

    /// The offscreen texture as the host renderer knows it.
    texture_id: egui::TextureId,

    /// Physical pixel size of [`GuestView::texture`].
    size: [u32; 2],
}

impl GuestView {
    pub fn new(render_state: &RenderState) -> Self {
        let device = &render_state.device;

        let renderer = Renderer::new(
            device,
            FORMAT,
            RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                predictable_texture_filtering: false,
            },
        );

        let size = [1, 1];
        let texture = create_texture(device, size);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture_id = render_state.renderer.write().register_native_texture(
            device,
            &view,
            wgpu::FilterMode::Nearest,
        );

        Self {
            renderer,
            texture,
            view,
            texture_id,
            size,
        }
    }

    /// Grows or shrinks the offscreen texture and points the host texture id at the new one.
    pub fn resize(&mut self, render_state: &RenderState, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if size == self.size {
            return;
        }

        let device = &render_state.device;
        self.texture = create_texture(device, size);
        self.view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.size = size;

        render_state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                device,
                &self.view,
                wgpu::FilterMode::Nearest,
                self.texture_id,
            );
    }

    /// Applies the frame's texture deltas and paints its meshes onto the offscreen texture.
    ///
    /// Empties `output.textures_delta`, which asserts on drop that every entry was applied.
    pub fn paint(&mut self, render_state: &RenderState, output: &mut FrameOutput) {
        let device = &render_state.device;
        let queue = &render_state.queue;

        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }

        let paint_jobs: Vec<ClippedPrimitive> = output
            .primitives
            .iter()
            .map(|(clip_rect, mesh)| ClippedPrimitive {
                clip_rect: *clip_rect,
                primitive: Primitive::Mesh(mesh.clone()),
            })
            .collect();

        let screen = ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: output.pixels_per_point,
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("guest_view"),
        });
        let user_buffers =
            self.renderer
                .update_buffers(device, queue, &mut encoder, &paint_jobs, &screen);

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("guest_view"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    multiview_mask: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();

            self.renderer.render(&mut pass, &paint_jobs, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();

        queue.submit(user_buffers.into_iter().chain([encoder.finish()]));
    }

    pub fn image(&self, size_in_points: egui::Vec2) -> egui::Image<'static> {
        egui::Image::new(egui::load::SizedTexture::new(
            self.texture_id,
            size_in_points,
        ))
    }
}

fn create_texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("guest_view"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
