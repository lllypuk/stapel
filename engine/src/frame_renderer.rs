use crate::gpu_textures::depth_texture;
use crate::{
    Assets, Gfx, Gpu, GpuTextures, MeshPipeline, Shown, ShownScene, SpritePipeline, UVec2, UiFrame,
    viewport,
};

/// Кадр в переданные виды: 3D на всю цель, спрайты во вписанный вьюпорт, egui поверх; окна не знает.
pub struct FrameRenderer {
    gpu: Gpu,
    textures: GpuTextures,
    meshes: MeshPipeline,
    sprites: SpritePipeline,
    depth: wgpu::Texture,
    ui: egui_wgpu::Renderer,
    size: (u32, u32),
    pixel_perfect: bool,
}

impl FrameRenderer {
    /// `format` — цель мира и спрайтов, `ui_format` — вид для egui.
    #[must_use]
    pub fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        ui_format: wgpu::TextureFormat,
        size: (u32, u32),
        pixel_perfect: bool,
    ) -> Self {
        let textures = GpuTextures::new(&gpu.device, &gpu.queue);
        let meshes = MeshPipeline::new(&gpu.device, &gpu.queue, format, &textures);
        let sprites = SpritePipeline::new(&gpu.device, &gpu.queue, format, &textures);
        let depth = depth_texture(&gpu.device, size);
        let ui = egui_wgpu::Renderer::new(
            &gpu.device,
            ui_format,
            egui_wgpu::RendererOptions::default(),
        );
        Self {
            gpu: gpu.clone(),
            textures,
            meshes,
            sprites,
            depth,
            ui,
            size,
            pixel_perfect,
        }
    }

    /// Пересоздаёт глубину под новый размер цели.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        self.depth = depth_texture(&self.gpu.device, size);
    }

    /// Заливает новые и изменённые ресурсы; зовётся каждый кадр, в том числе когда кадр не рисуется.
    pub fn upload(&mut self, assets: &mut Assets, ui: &UiFrame) {
        self.textures.upload(assets);
        self.meshes.upload(assets);
        for (id, deltas) in &ui.textures.set {
            for delta in deltas {
                self.ui
                    .update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
            }
        }
    }

    /// Освобождает текстуры egui и очищает дельту: неочищенный `TexturesDelta` паникует при drop в debug.
    pub fn release(&mut self, ui: &mut UiFrame) {
        for id in &ui.textures.free {
            self.ui.free_texture(id);
        }
        ui.textures.clear();
    }

    /// Пишет кадр в `encoder`; `ui_view` — вид той же цели, что `view`. Вторая часть результата — буферы egui,
    /// их отправляют раньше `encoder`.
    pub fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        ui_view: &wgpu::TextureView,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> (Shown, Vec<wgpu::CommandBuffer>) {
        let viewport = viewport(self.size, &gfx.camera, self.pixel_perfect);
        if let Some(scene) = &gfx.scene {
            self.meshes.draw_scene(
                encoder,
                view,
                &self.depth,
                &self.textures,
                scene,
                assets,
                gfx.clear_color,
            );
        }
        // С `Clear` спрайты стёрли бы 3D.
        self.sprites.draw(
            encoder,
            view,
            &self.textures,
            viewport,
            gfx,
            gfx.scene.is_none(),
        );
        let mut buffers = Vec::new();
        if !ui.jobs.is_empty() {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.size.0, self.size.1],
                pixels_per_point: ui.pixels_per_point,
            };
            buffers = self.ui.update_buffers(
                &self.gpu.device,
                &self.gpu.queue,
                encoder,
                &ui.jobs,
                &screen,
            );
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ui_view,
                        depth_slice: None,
                        resolve_target: None,
                        // `Clear` стёр бы спрайты под панелью.
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.ui.render(&mut pass, &ui.jobs, &screen);
        }
        let shown = Shown {
            camera: gfx.camera,
            viewport,
            scene: gfx.scene.as_ref().map(|scene| ShownScene {
                camera: scene.camera,
                size: UVec2::new(self.size.0, self.size.1),
            }),
        };
        (shown, buffers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::{TOL, gpu_full, load_png};
    use crate::{
        Camera2D, Camera3D, Canvas, Color, Mat4, Scene, Sprite, Vec2, Vertex, Viewmodel, vec2, vec3,
    };

    const W: u32 = 96;
    const H: u32 = 64;

    fn px(img: &[u8], x: u32, y: u32) -> [u8; 3] {
        let i = (4 * (y * W + x)) as usize;
        [img[i], img[i + 1], img[i + 2]]
    }

    fn near(got: [u8; 3], expected: [u8; 3]) -> bool {
        got.iter().zip(expected).all(|(&g, e)| g.abs_diff(e) <= TOL)
    }

    fn panel(w: f32, h: f32) -> UiFrame {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )),
            ..Default::default()
        });
        ctx.layer_painter(egui::LayerId::background()).rect_filled(
            egui::Rect::from_min_max(egui::pos2(80.0, 0.0), egui::pos2(96.0, 16.0)),
            0.0,
            egui::Color32::from_rgb(255, 0, 255),
        );
        let out = ctx.end_pass();
        UiFrame {
            jobs: ctx.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        }
    }

    fn wall(assets: &mut Assets) -> crate::MeshDraw {
        let vertex = |x: f32, y: f32| Vertex {
            pos: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5, 0.5],
            joints: [0; 4],
            weights: [0.0; 4],
        };
        let mesh = assets.add_mesh(
            vec![
                vertex(-1.0, -1.0),
                vertex(1.0, -1.0),
                vertex(1.0, 1.0),
                vertex(-1.0, 1.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );
        let red = load_png(assets, "fr-wall", 1, 1, &[255, 0, 0, 255]);
        crate::MeshDraw {
            mesh,
            texture: red,
            color: Color::WHITE,
            transform: Mat4::from_translation(vec3(0.0, 0.0, -1.5))
                * Mat4::from_scale(vec3(100.0, 100.0, 1.0)),
        }
    }

    fn draw_frame(gpu: &Gpu, gfx: &Gfx, assets: &mut Assets, mut ui: UiFrame) -> (Vec<u8>, Shown) {
        let canvas = Canvas::new(gpu, (W, H));
        let mut renderer =
            FrameRenderer::new(gpu, crate::CANVAS_FORMAT, canvas.ui_format(), (1, 1), false);
        renderer.resize((W, H));
        renderer.upload(assets, &ui);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let (shown, mut buffers) = renderer.draw(
            &mut encoder,
            canvas.view(),
            canvas.ui_view(),
            gfx,
            assets,
            &ui,
        );
        buffers.push(encoder.finish());
        gpu.queue.submit(buffers);
        renderer.release(&mut ui);
        (canvas.read_rgba(gpu), shown)
    }

    #[test]
    fn draws_world_viewmodel_sprites_in_letterbox_and_ui() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let mut assets = Assets::default();
        let wall = wall(&mut assets);
        let orc = assets.load_model(crate::model::tests::testdata("character-orc.glb"));
        let green = load_png(&mut assets, "fr-green", 1, 1, &[0, 255, 0, 255]);
        let mut gfx = Gfx {
            camera: Camera2D {
                center: Vec2::splat(32.0),
                size: Vec2::splat(64.0),
            },
            scene: Some(Scene {
                ambient: Color::WHITE,
                camera: Camera3D {
                    pos: vec3(0.0, 0.4, 0.0),
                    ..Default::default()
                },
                meshes: vec![wall],
                viewmodel: Some(Viewmodel {
                    fov_y: Camera3D::default().fov_y,
                    near: 0.05,
                    models: vec![crate::ModelDraw {
                        model: orc,
                        transform: Mat4::from_translation(vec3(0.0, 0.0, -2.0)),
                        pose: None,
                        tint: Color::BLACK,
                    }],
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        gfx.draw_sprite(Sprite {
            texture: green,
            pos: vec2(10.0, 10.0),
            size: Vec2::splat(8.0),
            ..Default::default()
        });

        let (img, shown) = draw_frame(&gpu, &gfx, &mut assets, panel(W as f32, H as f32));

        assert!(
            near(px(&img, 2, 32), [255, 0, 0]),
            "world {:?}",
            px(&img, 2, 32)
        );
        assert!(
            near(px(&img, 48, 32), [0, 0, 0]),
            "viewmodel {:?}",
            px(&img, 48, 32)
        );
        assert!(
            near(px(&img, 26, 53), [0, 255, 0]),
            "sprite {:?}",
            px(&img, 26, 53)
        );
        assert!(
            near(px(&img, 90, 5), [255, 0, 255]),
            "ui {:?}",
            px(&img, 90, 5)
        );
        assert!(
            near(px(&img, 14, 53), [255, 0, 0]),
            "no sprite left of letterbox"
        );
        assert_eq!((shown.viewport.x, shown.viewport.w), (16.0, 64.0));
        assert_eq!(shown.scene.unwrap().size, UVec2::new(W, H));
    }

    #[test]
    fn translucent_sprite_over_translucent_clear_gives_opaque_png() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let mut assets = Assets::default();
        let mut gfx = Gfx {
            clear_color: Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 0.5,
            },
            camera: Camera2D {
                center: Vec2::splat(32.0),
                size: Vec2::splat(64.0),
            },
            ..Default::default()
        };
        gfx.draw_sprite(Sprite {
            pos: vec2(32.0, 32.0),
            size: Vec2::splat(16.0),
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.5,
            },
            ..Default::default()
        });

        let (img, _) = draw_frame(
            &gpu,
            &gfx,
            &mut assets,
            UiFrame {
                textures: egui::TexturesDelta::default(),
                jobs: Vec::new(),
                pixels_per_point: 1.0,
            },
        );

        assert!(img.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }
}
