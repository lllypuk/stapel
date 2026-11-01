use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::surface::{Action, action, pick_format, pick_present_mode, split, surface_size};
use crate::{
    Assets, CANVAS_FORMAT, Canvas, FrameRenderer, Gfx, Gpu, PresentPass, Shown, UiFrame,
    linear_present,
};

/// Поверхность окна и GPU-устройство; кадр рисуется в `Canvas`: 3D на всё, спрайты во вписанный вьюпорт, egui поверх, затем вывод на поверхность.
pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    gpu: Gpu,
    config: wgpu::SurfaceConfiguration,
    canvas: Canvas,
    frame: FrameRenderer,
    present: PresentPass,
    present_linear: bool,
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
        let canvas = Canvas::new(&gpu, (1, 1));
        let views = adapter.get_downlevel_capabilities().flags;
        let present_linear = linear_present(
            canvas.has_linear_view(),
            views.contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS),
        );
        let present_format = if present_linear {
            config.format.remove_srgb_suffix()
        } else {
            config.format
        };
        if present_linear {
            config.view_formats = vec![present_format];
        }
        log::info!("surface: {:?}, {:?}", config.format, config.present_mode);
        // Настоящий размер — в `resize` ниже: только он зажимает его под лимит текстур.
        let frame = FrameRenderer::new(
            &gpu,
            CANVAS_FORMAT,
            canvas.ui_format(),
            (1, 1),
            pixel_perfect,
        );
        let present = PresentPass::new(&gpu, present_format);

        let mut renderer = Self {
            window,
            instance,
            surface,
            gpu,
            config,
            canvas,
            frame,
            present,
            present_linear,
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
            self.canvas.resize(&self.gpu, (w, h));
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

    /// Выводит внешний `Canvas` на поверхность; `false` — кадр не показан.
    ///
    /// # Errors
    /// Ошибка валидации поверхности.
    pub fn present_canvas(&mut self, canvas: &Canvas) -> Result<bool, RenderError> {
        let shown = self.with_frame(|this, frame| {
            let target = this.target_view(&frame);
            let mut encoder = this
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            this.present
                .draw(&mut encoder, this.source(canvas), &target);
            this.submit(frame, Vec::new(), encoder);
        })?;
        Ok(shown.is_some())
    }

    /// Устройство, под которое создана поверхность.
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    fn present(
        &mut self,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> Result<Option<Shown>, RenderError> {
        self.with_frame(|this, frame| this.draw(frame, gfx, assets, ui))
    }

    fn with_frame<T>(
        &mut self,
        draw: impl FnOnce(&mut Self, wgpu::SurfaceTexture) -> T,
    ) -> Result<Option<T>, RenderError> {
        if !self.is_available() {
            return Ok(None);
        }
        let (frame, status) = split(self.surface.get_current_texture());
        match (action(&status), frame) {
            (Action::Draw { reconfigure }, Some(frame)) => {
                let shown = draw(self, frame);
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

    fn target_view(&self, frame: &wgpu::SurfaceTexture) -> wgpu::TextureView {
        frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(if self.present_linear {
                self.config.format.remove_srgb_suffix()
            } else {
                self.config.format
            }),
            ..Default::default()
        })
    }

    fn source<'a>(&self, canvas: &'a Canvas) -> &'a wgpu::TextureView {
        if self.present_linear {
            canvas.ui_view()
        } else {
            canvas.view()
        }
    }

    fn submit(
        &self,
        frame: wgpu::SurfaceTexture,
        mut buffers: Vec<wgpu::CommandBuffer>,
        encoder: wgpu::CommandEncoder,
    ) {
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
    }

    fn draw(
        &mut self,
        frame: wgpu::SurfaceTexture,
        gfx: &Gfx,
        assets: &Assets,
        ui: &UiFrame,
    ) -> Shown {
        let target = self.target_view(&frame);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let (shown, buffers) = self.frame.draw(
            &mut encoder,
            self.canvas.view(),
            self.canvas.ui_view(),
            gfx,
            assets,
            ui,
        );
        self.present
            .draw(&mut encoder, self.source(&self.canvas), &target);
        self.submit(frame, buffers, encoder);
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
