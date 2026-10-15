use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::gpu_textures::depth_texture;
use crate::surface::{Action, action, pick_format, pick_present_mode, split, surface_size};
use crate::{
    Assets, ExpandedModels, Gfx, GpuTextures, MeshPipeline, SpritePipeline, UiFrame, Viewport,
    viewport,
};

/// Поверхность окна и GPU-устройство; кадр: 3D на всю поверхность, спрайты во вписанный вьюпорт, egui — поверх.
pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    textures: GpuTextures,
    meshes: MeshPipeline,
    sprites: SpritePipeline,
    depth: wgpu::Texture,
    ui: egui_wgpu::Renderer,
    ui_format: wgpu::TextureFormat,
    pixel_perfect: bool,
    sized: bool,
    occluded: bool,
}

/// Поверхность сломана так, что продолжать нельзя.
#[derive(Debug)]
pub struct RenderError;

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("surface validation error")
    }
}

impl std::error::Error for RenderError {}

impl Renderer {
    /// # Panics
    /// Без поверхности, адаптера, устройства или sRGB-формата поверхности.
    pub fn new(window: Arc<Window>, vsync: bool, pixel_perfect: bool) -> Self {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter for this surface");
        log::info!("adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        config.format = pick_format(&caps.formats).unwrap_or_else(|| {
            panic!(
                "surface has no sRGB format, linear colors would come out dark: {:?}",
                caps.formats
            )
        });
        config.present_mode = pick_present_mode(vsync, &caps.present_modes);
        // egui смешивает в гамме: в `*Srgb` вид он рисует с искажёнными полутонами и warn на старте.
        // Второй вид поверхности без `SURFACE_VIEW_FORMATS` (у wgpu — только Vulkan) валит configure.
        let views = adapter.get_downlevel_capabilities().flags;
        let ui_format = if views.contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS) {
            config.format.remove_srgb_suffix()
        } else {
            config.format
        };
        if ui_format != config.format {
            config.view_formats = vec![ui_format];
        }
        log::info!("surface: {:?}, {:?}", config.format, config.present_mode);
        let textures = GpuTextures::new(&device, &queue);
        let meshes = MeshPipeline::new(&device, &queue, config.format, &textures);
        let sprites = SpritePipeline::new(&device, &queue, config.format, &textures);
        let depth = depth_texture(&device, (config.width, config.height));
        let ui =
            egui_wgpu::Renderer::new(&device, ui_format, egui_wgpu::RendererOptions::default());

        let mut renderer = Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            textures,
            meshes,
            sprites,
            depth,
            ui,
            ui_format,
            pixel_perfect,
            sized: false,
            occluded: false,
        };
        renderer.resize(size);
        renderer
    }

    /// Можно ли сейчас брать кадр: у окна нет нулевой стороны и X11 не сообщил, что оно скрыто.
    /// Свёрнутое окно на Wayland остаётся доступным: о сворачивании там не сообщают.
    pub fn is_available(&self) -> bool {
        self.sized && !self.occluded
    }

    /// X11 сообщил, скрыто ли окно.
    pub fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    /// Перенастраивает поверхность под новый размер окна.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let max = self.device.limits().max_texture_dimension_2d;
        let fitted = surface_size(size, max);
        self.sized = fitted.is_some();
        if let Some((w, h)) = fitted {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
            self.depth = depth_texture(&self.device, (w, h));
        }
    }

    /// Вьюпорт показанного кадра; `None` — кадр не показан.
    ///
    /// # Errors
    /// Ошибка валидации поверхности.
    ///
    /// # Panics
    /// Если не пересоздалась потерянная поверхность.
    pub fn render(
        &mut self,
        gfx: &Gfx,
        assets: &mut Assets,
        mut ui: UiFrame,
    ) -> Result<Option<Viewport>, RenderError> {
        // До проверки поверхности: иначе текстуры, загруженные при свёрнутом окне, ждали бы его показа,
        // а атлас шрифта egui присылает один раз и потерялся бы.
        self.textures.upload(assets);
        self.meshes.upload(assets);
        for (id, deltas) in &ui.textures.set {
            for delta in deltas {
                self.ui
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let shown = self.present(gfx, assets, &ui);
        // После submit кадра или сразу, если кадр пропущен.
        for id in &ui.textures.free {
            self.ui.free_texture(id);
        }
        // Неочищенный `TexturesDelta` паникует при drop в debug.
        ui.textures.clear();
        shown
    }

    fn present(
        &mut self,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> Result<Option<Viewport>, RenderError> {
        if !self.is_available() {
            return Ok(None);
        }
        let (frame, status) = split(self.surface.get_current_texture());
        match (action(&status), frame) {
            (Action::Draw { reconfigure }, Some(frame)) => {
                let viewport = self.draw(frame, gfx, assets, ui);
                if reconfigure {
                    self.surface.configure(&self.device, &self.config);
                }
                Ok(Some(viewport))
            }
            (Action::Reconfigure, _) => {
                self.surface.configure(&self.device, &self.config);
                Ok(None)
            }
            (Action::Recreate, _) => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                Ok(None)
            }
            (Action::Fail, _) => Err(RenderError),
            _ => Ok(None),
        }
    }

    fn draw(
        &mut self,
        frame: wgpu::SurfaceTexture,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> Viewport {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let viewport = viewport(
            (self.config.width, self.config.height),
            &gfx.camera,
            self.pixel_perfect,
        );
        if let Some(scene) = &gfx.scene {
            self.meshes.draw(
                &mut encoder,
                &view,
                &self.depth,
                &self.textures,
                scene,
                &ExpandedModels::new(&scene.models, assets),
                gfx.clear_color,
            );
        }
        // С `Clear` спрайты стёрли бы 3D.
        self.sprites.draw(
            &mut encoder,
            &view,
            &self.textures,
            viewport,
            gfx,
            gfx.scene.is_none(),
        );
        let mut buffers = Vec::new();
        if !ui.jobs.is_empty() {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: ui.pixels_per_point,
            };
            buffers =
                self.ui
                    .update_buffers(&self.device, &self.queue, &mut encoder, &ui.jobs, &screen);
            let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.ui_format),
                ..Default::default()
            });
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &ui_view,
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
        buffers.push(encoder.finish());
        self.queue.submit(buffers);
        // На Wayland это ограничивает `RedrawRequested` частотой монитора, даже без vsync.
        if matches!(
            self.config.present_mode,
            wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed
        ) {
            self.window.pre_present_notify();
        }
        self.queue.present(frame);
        viewport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_error_names_cause() {
        assert_eq!(RenderError.to_string(), "surface validation error");
    }
}
