use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::canvas::has_view_formats;
use crate::surface::{Action, action, pick_format, pick_present_mode, split, surface_size};
use crate::{Assets, CANVAS_FORMAT, Canvas, FrameRenderer, Gfx, Gpu, PresentPass, Shown, UiFrame};

/// Поверхность сломана так, что продолжать нельзя.
#[derive(Debug)]
pub struct RenderError;

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("surface validation error")
    }
}

impl std::error::Error for RenderError {}

/// Поверхность окна, GPU-устройство и вывод готового `Canvas`; кадр не рисует.
pub(crate) struct Output {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    pub(crate) gpu: Gpu,
    config: wgpu::SurfaceConfiguration,
    present: PresentPass,
    present_linear: bool,
    sized: bool,
    occluded: bool,
}

/// Кадр рисуется в `Canvas`: 3D на всё, спрайты во вписанный вьюпорт, egui поверх, затем вывод на поверхность.
pub struct Renderer {
    out: Output,
    canvas: Canvas,
    frame: FrameRenderer,
}

impl Output {
    /// # Panics
    /// Без поверхности, адаптера, устройства или sRGB-формата поверхности.
    pub(crate) fn new(window: Arc<Window>, vsync: bool) -> Self {
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
        // Нужны оба флага: линейный источник при цели `*Srgb` дал бы лишнее кодирование и светлую картинку.
        let present_linear = has_view_formats(&gpu)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let present_format = if present_linear {
            config.format.remove_srgb_suffix()
        } else {
            config.format
        };
        if present_linear {
            config.view_formats = vec![present_format];
        }
        log::info!("surface: {:?}, {:?}", config.format, config.present_mode);
        let present = PresentPass::new(&gpu, present_format);

        let mut out = Self {
            window,
            instance,
            surface,
            gpu,
            config,
            present,
            present_linear,
            sized: false,
            occluded: false,
        };
        out.resize(size);
        out
    }

    /// Можно ли сейчас брать кадр: у окна нет нулевой стороны и X11 не сообщил, что оно скрыто.
    /// Свёрнутое окно на Wayland остаётся доступным: о сворачивании там не сообщают.
    pub(crate) fn is_available(&self) -> bool {
        self.sized && !self.occluded
    }

    pub(crate) fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    /// Размер поверхности после зажима под лимит текстур.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Перенастраивает поверхность; `Some(размер)` — окно годно для кадров.
    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) -> Option<(u32, u32)> {
        let max = self.gpu.device.limits().max_texture_dimension_2d;
        let fitted = surface_size(size, max);
        self.sized = fitted.is_some();
        if let Some((w, h)) = fitted {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.gpu.device, &self.config);
        }
        fitted
    }

    /// Выводит `Canvas` на поверхность; `false` — кадр не показан.
    pub(crate) fn present_canvas(&mut self, canvas: &Canvas) -> Result<bool, RenderError> {
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
}

impl Renderer {
    /// # Panics
    /// Без поверхности, адаптера, устройства или sRGB-формата поверхности.
    pub fn new(window: Arc<Window>, vsync: bool, pixel_perfect: bool) -> Self {
        let out = Output::new(window, vsync);
        let size = out.window.inner_size();
        let canvas = Canvas::new(&out.gpu, (1, 1));
        // Настоящий размер — в `resize` ниже: только он зажимает его под лимит текстур.
        let frame = FrameRenderer::new(
            &out.gpu,
            CANVAS_FORMAT,
            canvas.ui_format(),
            (1, 1),
            pixel_perfect,
        );
        let mut renderer = Self { out, canvas, frame };
        renderer.resize(size);
        renderer
    }

    /// Можно ли сейчас брать кадр: у окна нет нулевой стороны и X11 не сообщил, что оно скрыто.
    pub fn is_available(&self) -> bool {
        self.out.is_available()
    }

    /// X11 сообщил, скрыто ли окно.
    pub fn set_occluded(&mut self, occluded: bool) {
        self.out.set_occluded(occluded);
    }

    /// Перенастраивает поверхность под новый размер окна.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if let Some((w, h)) = self.out.resize(size) {
            self.canvas.resize(&self.out.gpu, (w, h));
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
        let shown = self.out.with_frame(|out, frame| {
            let target = out.target_view(&frame);
            let mut encoder = out
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let (shown, buffers) = self.frame.draw(
                &mut encoder,
                self.canvas.view(),
                self.canvas.ui_view(),
                gfx,
                assets,
                &ui,
            );
            out.present
                .draw(&mut encoder, out.source(&self.canvas), &target);
            out.submit(frame, buffers, encoder);
            shown
        });
        // После submit кадра или сразу, если кадр пропущен.
        self.frame.release(&mut ui);
        shown
    }

    /// Устройство, под которое создана поверхность.
    pub fn gpu(&self) -> &Gpu {
        &self.out.gpu
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
