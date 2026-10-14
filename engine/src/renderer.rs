use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::{Assets, Gfx, SpritePipeline, letterbox};

/// Поверхность окна и GPU-устройство; кадр рисует `SpritePipeline` во вписанный вьюпорт.
pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    sprites: SpritePipeline,
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
    pub fn new(window: Arc<Window>, vsync: bool) -> Self {
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
        log::info!("surface: {:?}, {:?}", config.format, config.present_mode);
        let sprites = SpritePipeline::new(&device, &queue, config.format);

        let mut renderer = Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            sprites,
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
        }
    }

    /// `Ok(true)`, если кадр показан.
    ///
    /// # Errors
    /// Ошибка валидации поверхности.
    ///
    /// # Panics
    /// Если не пересоздалась потерянная поверхность.
    pub fn render(&mut self, gfx: &Gfx, assets: &mut Assets) -> Result<bool, RenderError> {
        // До проверки поверхности: иначе текстуры, загруженные при свёрнутом окне, ждали бы его показа.
        self.sprites.upload(assets);
        if !self.is_available() {
            return Ok(false);
        }
        let (frame, status) = split(self.surface.get_current_texture());
        match (action(&status), frame) {
            (Action::Draw { reconfigure }, Some(frame)) => {
                self.draw(frame, gfx);
                if reconfigure {
                    self.surface.configure(&self.device, &self.config);
                }
                Ok(true)
            }
            (Action::Reconfigure, _) => {
                self.surface.configure(&self.device, &self.config);
                Ok(false)
            }
            (Action::Recreate, _) => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                Ok(false)
            }
            (Action::Fail, _) => Err(RenderError),
            _ => Ok(false),
        }
    }

    fn draw(&mut self, frame: wgpu::SurfaceTexture, gfx: &Gfx) {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let viewport = letterbox((self.config.width, self.config.height), gfx.camera.aspect());
        self.sprites.draw(&mut encoder, &view, viewport, gfx);
        self.queue.submit([encoder.finish()]);
        // На Wayland это ограничивает `RedrawRequested` частотой монитора, даже без vsync.
        if matches!(
            self.config.present_mode,
            wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed
        ) {
            self.window.pre_present_notify();
        }
        self.queue.present(frame);
    }
}

/// Размер для `configure`: wgpu паникует на нулевой стороне и на стороне больше лимита устройства.
fn surface_size(size: PhysicalSize<u32>, max: u32) -> Option<(u32, u32)> {
    (size.width > 0 && size.height > 0).then(|| (size.width.min(max), size.height.min(max)))
}

/// `Color` линейный, поэтому нужна `*Srgb`-поверхность: она сама кодирует в sRGB.
fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats.iter().copied().find(wgpu::TextureFormat::is_srgb)
}

/// Тот же выбор, что у `AutoVsync`/`AutoNoVsync`, но явный, чтобы в лог попал настоящий режим.
fn pick_present_mode(vsync: bool, modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
    let preferred: &[wgpu::PresentMode] = if vsync {
        &[FifoRelaxed, Fifo]
    } else {
        &[Immediate, Mailbox, Fifo]
    };
    preferred
        .iter()
        .copied()
        .find(|m| modes.contains(m))
        .unwrap_or(Fifo)
}

#[derive(Debug, PartialEq)]
enum Action {
    /// `reconfigure` — после `present`: пока текстура жива, `configure` паникует.
    Draw {
        reconfigure: bool,
    },
    Skip,
    Reconfigure,
    Recreate,
    Fail,
}

fn split(cur: wgpu::CurrentSurfaceTexture) -> (Option<wgpu::SurfaceTexture>, wgpu::SurfaceStatus) {
    use wgpu::{CurrentSurfaceTexture as C, SurfaceStatus as S};
    match cur {
        C::Success(t) => (Some(t), S::Good),
        C::Suboptimal(t) => (Some(t), S::Suboptimal),
        C::Timeout => (None, S::Timeout),
        C::Occluded => (None, S::Occluded),
        C::Outdated => (None, S::Outdated),
        C::Lost => (None, S::Lost),
        C::Validation => (None, S::Validation),
    }
}

fn action(status: &wgpu::SurfaceStatus) -> Action {
    use wgpu::SurfaceStatus as S;
    match status {
        S::Good => Action::Draw { reconfigure: false },
        S::Suboptimal => Action::Draw { reconfigure: true },
        S::Timeout | S::Occluded => Action::Skip,
        S::Outdated => Action::Reconfigure,
        S::Lost => Action::Recreate,
        S::Validation => Action::Fail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::PresentMode::*;
    use wgpu::SurfaceStatus as S;
    use wgpu::TextureFormat as F;

    #[test]
    fn zero_side_makes_surface_unavailable() {
        assert_eq!(
            surface_size(PhysicalSize::new(800, 600), 8192),
            Some((800, 600))
        );
        assert_eq!(surface_size(PhysicalSize::new(0, 600), 8192), None);
        assert_eq!(surface_size(PhysicalSize::new(800, 0), 8192), None);
        assert_eq!(surface_size(PhysicalSize::new(0, 0), 8192), None);
    }

    #[test]
    fn size_is_clamped_to_device_limit() {
        assert_eq!(
            surface_size(PhysicalSize::new(10240, 2880), 8192),
            Some((8192, 2880))
        );
        assert_eq!(
            surface_size(PhysicalSize::new(800, 9000), 8192),
            Some((800, 8192))
        );
    }

    #[test]
    fn status_to_action() {
        assert_eq!(action(&S::Good), Action::Draw { reconfigure: false });
        assert_eq!(action(&S::Suboptimal), Action::Draw { reconfigure: true });
        assert_eq!(action(&S::Timeout), Action::Skip);
        assert_eq!(action(&S::Occluded), Action::Skip);
        assert_eq!(action(&S::Outdated), Action::Reconfigure);
        assert_eq!(action(&S::Lost), Action::Recreate);
        assert_eq!(action(&S::Validation), Action::Fail);
    }

    #[test]
    fn picks_only_srgb_format() {
        assert_eq!(
            pick_format(&[F::Bgra8Unorm, F::Bgra8UnormSrgb]),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(pick_format(&[F::Rgba16Float, F::Bgra8Unorm]), None);
        assert_eq!(pick_format(&[]), None);
    }

    #[test]
    fn present_mode_follows_vsync() {
        let all = [Fifo, FifoRelaxed, Immediate, Mailbox];
        assert_eq!(pick_present_mode(true, &all), FifoRelaxed);
        assert_eq!(pick_present_mode(true, &[Fifo, Immediate]), Fifo);
        assert_eq!(pick_present_mode(false, &all), Immediate);
        assert_eq!(pick_present_mode(false, &[Fifo, Mailbox]), Mailbox);
        assert_eq!(pick_present_mode(false, &[Fifo]), Fifo);
    }
}
