//! Офскрин-хелперы GPU-тестов: адаптер, цель рендера, чтение пикселей.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::{Assets, Gpu, Handle, Texture};

/// Сторона квадратной цели рендера в пикселях.
pub(crate) const SIZE: u32 = 64;
/// Допуск на канал при сравнении пикселей.
pub(crate) const TOL: u8 = 2;

/// Устройство на адаптере из окружения, одно на процесс; `None` — адаптера нет и `REQUIRE_GPU` не стоит.
/// Одно — потому что параллельное создание экземпляров Vulkan роняет загрузчик в SIGSEGV.
pub(crate) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    gpu_full().map(|g| (g.device, g.queue))
}

/// То же, что `gpu`, вместе с адаптером.
pub(crate) fn gpu_full() -> Option<Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(open_gpu).clone()
}

fn open_gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(gpu) => {
            eprintln!("adapter: {:?}", gpu.adapter.get_info());
            Some(gpu)
        }
        Err(e) => {
            assert!(
                std::env::var("REQUIRE_GPU").as_deref() != Ok("1"),
                "REQUIRE_GPU=1, but no GPU adapter: {e}"
            );
            eprintln!("SKIPPED: no GPU adapter: {e}");
            None
        }
    }
}

/// Рисует `draw` в цель `SIZE × SIZE` формата `Rgba8UnormSrgb` и возвращает её RGBA построчно сверху.
pub(crate) fn offscreen(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Vec<u8> {
    let extent = wgpu::Extent3d {
        width: SIZE,
        height: SIZE,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // 64 × 4 = 256 байт — уже кратно `COPY_BYTES_PER_ROW_ALIGNMENT`.
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(4 * SIZE * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    draw(&mut encoder, &view);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * SIZE),
                rows_per_image: Some(SIZE),
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);

    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map readback"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    readback
        .get_mapped_range(..)
        .expect("mapped range")
        .to_vec()
}

/// Пиксель, накрывающий мировую точку `(x + 0.5, y + 0.5)`; мир — пиксели с Y вверх.
pub(crate) fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = (4 * ((SIZE - 1 - y) * SIZE + x)) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

/// Пиксель `pixel(img, x, y)` равен `expected` с точностью `TOL`.
pub(crate) fn assert_pixel(img: &[u8], what: &str, (x, y): (u32, u32), expected: [u8; 3]) {
    let got = pixel(img, x, y);
    let close = got.iter().zip(expected).all(|(&g, e)| g.abs_diff(e) <= TOL);
    assert!(
        close,
        "{what} at ({x}, {y}): got {got:?}, expected {expected:?}"
    );
}

/// Текстура из RGBA через временный PNG; `name` уникален в пределах процесса тестов.
pub(crate) fn load_png(
    assets: &mut Assets,
    name: &str,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> Handle<Texture> {
    let path =
        std::env::temp_dir().join(format!("engine-gpu-test-{}-{name}.png", std::process::id()));
    image::save_buffer(&path, rgba, w, h, image::ExtendedColorType::Rgba8).unwrap();
    let id = assets.load_texture(&path);
    fs::remove_file(&path).unwrap();
    id
}

/// Временный каталог процесса, который убирается и после упавшего теста.
pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("stapel-{tag}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}
