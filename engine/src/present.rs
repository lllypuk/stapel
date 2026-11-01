use crate::Gpu;

/// Вывод `Canvas` на цель 1:1 от левого верхнего угла; координаты зажаты в размер источника.
pub(crate) struct PresentPass {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl PresentPass {
    /// `target_format` — формат вида цели.
    #[must_use]
    pub(crate) fn new(gpu: &Gpu, target_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("present.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(target_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device: device.clone(),
            pipeline,
            layout,
        }
    }

    /// Копирует `source` в `target` проходом с `Clear`.
    pub(crate) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::read_texture;
    use crate::gpu_test::{TOL, gpu_full};
    use crate::{Assets, Camera2D, Canvas, FrameRenderer, Gfx, Sprite, UiFrame, Vec2, vec2};

    const BGRA: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

    fn target(gpu: &Gpu, size: (u32, u32)) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BGRA,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[BGRA.remove_srgb_suffix()],
        })
    }

    fn clear(gpu: &Gpu, view: &wgpu::TextureView, color: wgpu::Color) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        gpu.queue.submit([encoder.finish()]);
    }

    fn present(
        gpu: &Gpu,
        source: &wgpu::TextureView,
        texture: &wgpu::Texture,
        size: (u32, u32),
        linear: bool,
    ) -> Vec<u8> {
        let format = if linear {
            BGRA.remove_srgb_suffix()
        } else {
            BGRA
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        });
        let pass = PresentPass::new(gpu, format);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        pass.draw(&mut encoder, source, &view);
        gpu.queue.submit([encoder.finish()]);
        read_texture(gpu, texture, size)
    }

    fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
        bgra.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect()
    }

    fn assert_close(got: &[u8], expected: &[u8], tol: u8) {
        assert_eq!(got.len(), expected.len());
        for (i, (g, e)) in got.iter().zip(expected).enumerate() {
            assert!(g.abs_diff(*e) <= tol, "byte {i}: {g} != {e}");
        }
    }

    const HALFTONE: wgpu::Color = wgpu::Color {
        r: 0.2,
        g: 0.5,
        b: 0.8,
        a: 1.0,
    };

    #[test]
    fn linear_pair_copies_bytes_with_channel_order() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let canvas = Canvas::new(&gpu, (70, 5));
        assert!(canvas.has_linear_view(), "adapter without VIEW_FORMATS");
        clear(&gpu, canvas.view(), HALFTONE);
        let tex = target(&gpu, (70, 5));

        let out = present(&gpu, canvas.ui_view(), &tex, (70, 5), true);

        let rgba = bgra_to_rgba(&out);
        assert_close(&rgba, &canvas.read_rgba(&gpu), 0);
        let first = &rgba[..3];
        assert!(first[0] < first[1] && first[1] < first[2], "{first:?}");
    }

    #[test]
    fn srgb_target_keeps_halftones_for_canvas_with_linear_view() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let canvas = Canvas::new(&gpu, (8, 8));
        clear(&gpu, canvas.view(), HALFTONE);
        let tex = target(&gpu, (8, 8));

        let out = present(&gpu, canvas.view(), &tex, (8, 8), false);

        assert_close(&bgra_to_rgba(&out), &canvas.read_rgba(&gpu), TOL);
    }

    #[test]
    fn srgb_target_keeps_halftones_for_canvas_without_second_view() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let plain = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("plain"),
            size: wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source = plain.create_view(&wgpu::TextureViewDescriptor::default());
        clear(&gpu, &source, HALFTONE);
        let tex = target(&gpu, (8, 8));

        let out = present(&gpu, &source, &tex, (8, 8), false);

        let expected = read_texture(&gpu, &plain, (8, 8));
        assert_close(&bgra_to_rgba(&out), &expected, TOL);
    }

    fn pattern(gpu: &Gpu, canvas: &Canvas, frame: &mut FrameRenderer) {
        let (w, h) = canvas.size();
        let mut gfx = Gfx {
            camera: Camera2D {
                center: vec2(w as f32 / 2.0, h as f32 / 2.0),
                size: vec2(w as f32, h as f32),
            },
            ..Default::default()
        };
        gfx.draw_sprite(Sprite {
            pos: vec2(5.0, 6.0),
            size: Vec2::splat(4.0),
            ..Default::default()
        });
        let ui = UiFrame {
            textures: egui::TexturesDelta::default(),
            jobs: Vec::new(),
            pixels_per_point: 1.0,
        };
        let mut assets = Assets::default();
        frame.upload(&mut assets, &ui);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let (_, mut buffers) = frame.draw(
            &mut encoder,
            canvas.view(),
            canvas.ui_view(),
            &gfx,
            &assets,
            &ui,
        );
        buffers.push(encoder.finish());
        gpu.queue.submit(buffers);
    }

    #[test]
    fn resize_keeps_one_to_one_and_clamps_larger_target() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let mut canvas = Canvas::new(&gpu, (32, 24));
        let mut frame = FrameRenderer::new(
            &gpu,
            crate::CANVAS_FORMAT,
            canvas.ui_format(),
            canvas.size(),
            false,
        );
        pattern(&gpu, &canvas, &mut frame);
        let tex = target(&gpu, (32, 24));
        let before = bgra_to_rgba(&present(&gpu, canvas.view(), &tex, (32, 24), false));
        assert_close(&before, &canvas.read_rgba(&gpu), TOL);

        canvas.resize(&gpu, (20, 16));
        frame.resize(canvas.size());
        pattern(&gpu, &canvas, &mut frame);
        let expected = canvas.read_rgba(&gpu);
        assert!(
            expected
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p != &expected[..4]),
            "pattern is flat"
        );
        let tex = target(&gpu, (30, 22));
        let out = bgra_to_rgba(&present(&gpu, canvas.view(), &tex, (30, 22), false));

        for y in 0..22usize {
            for x in 0..30usize {
                let (sx, sy) = (x.min(19), y.min(15));
                let got = &out[4 * (y * 30 + x)..][..3];
                let want = &expected[4 * (sy * 20 + sx)..][..3];
                assert_close(got, want, TOL);
            }
        }
    }
}
