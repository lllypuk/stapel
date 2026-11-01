use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::surface::{Action, action, pick_format, pick_present_mode, split, surface_size};
use crate::{Assets, FrameRenderer, Gfx, Gpu, Shown, UiFrame};

/// Поверхность окна и GPU-устройство; кадр: 3D на всю поверхность, спрайты во вписанный вьюпорт, egui — поверх.
pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    gpu: Gpu,
    config: wgpu::SurfaceConfiguration,
    frame: FrameRenderer,
    ui_format: wgpu::TextureFormat,
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
        let gpu = Gpu::new(adapter);
        let adapter = &gpu.adapter;

        let size = window.inner_size();
        let caps = surface.get_capabilities(adapter);
        let mut config = surface
            .get_default_config(adapter, size.width.max(1), size.height.max(1))
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
        // Настоящий размер — в `resize` ниже: только он зажимает его под лимит текстур.
        let frame = FrameRenderer::new(&gpu, config.format, ui_format, (1, 1), pixel_perfect);

        let mut renderer = Self {
            window,
            instance,
            surface,
            gpu,
            config,
            frame,
            ui_format,
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
        let max = self.gpu.device.limits().max_texture_dimension_2d;
        let fitted = surface_size(size, max);
        self.sized = fitted.is_some();
        if let Some((w, h)) = fitted {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.gpu.device, &self.config);
            self.frame.resize((w, h));
        }
    }

    /// Что показано; `None` — кадр не показан.
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
    ) -> Result<Option<Shown>, RenderError> {
        // До проверки поверхности: иначе текстуры, загруженные при свёрнутом окне, ждали бы его показа,
        // а атлас шрифта egui присылает один раз и потерялся бы.
        self.frame.upload(assets, &ui);
        let shown = self.present(gfx, assets, &ui);
        // После submit кадра или сразу, если кадр пропущен.
        self.frame.release(&mut ui);
        shown
    }

    fn present(
        &mut self,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> Result<Option<Shown>, RenderError> {
        if !self.is_available() {
            return Ok(None);
        }
        let (frame, status) = split(self.surface.get_current_texture());
        match (action(&status), frame) {
            (Action::Draw { reconfigure }, Some(frame)) => {
                let shown = self.draw(frame, gfx, assets, ui);
                if reconfigure {
                    self.surface.configure(&self.gpu.device, &self.config);
                }
                Ok(Some(shown))
            }
            (Action::Reconfigure, _) => {
                self.surface.configure(&self.gpu.device, &self.config);
                Ok(None)
            }
            (Action::Recreate, _) => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.gpu.device, &self.config);
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
    ) -> Shown {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.ui_format),
            ..Default::default()
        });
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let (shown, mut buffers) = self
            .frame
            .draw(&mut encoder, &view, &ui_view, gfx, assets, ui);
        buffers.push(encoder.finish());
        self.gpu.queue.submit(buffers);
        // На Wayland это ограничивает `RedrawRequested` частотой монитора, даже без vsync.
        if matches!(
            self.config.present_mode,
            wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed
        ) {
            self.window.pre_present_notify();
        }
        self.gpu.queue.present(frame);
        shown
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
