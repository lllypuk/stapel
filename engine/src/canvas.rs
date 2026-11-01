use crate::Gpu;

/// Формат цвета `Canvas`.
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Даёт ли устройство второй формат вида для текстуры.
pub(crate) fn has_view_formats(gpu: &Gpu) -> bool {
    gpu.adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::VIEW_FORMATS)
}

/// Цель рендера без окна: `Rgba8UnormSrgb` и, где можно, линейный вид для egui.
pub struct Canvas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    ui_view: Option<wgpu::TextureView>,
    size: (u32, u32),
}

impl Canvas {
    /// # Panics
    /// Если сторона нулевая или больше лимита устройства.
    #[must_use]
    pub fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        let linear = has_view_formats(gpu);
        let view_formats: &[wgpu::TextureFormat] = if linear {
            &[CANVAS_FORMAT.remove_srgb_suffix()]
        } else {
            &[]
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ui_view = linear.then(|| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(CANVAS_FORMAT.remove_srgb_suffix()),
                ..Default::default()
            })
        });
        Self {
            texture,
            view,
            ui_view,
            size,
        }
    }

    /// Пересоздаёт цель под новый размер.
    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        *self = Self::new(gpu, size);
    }

    /// Размер в пикселях.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Вид `*Srgb`: сюда рисуются мир и спрайты.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Вид для egui: линейный, если устройство даёт второй формат, иначе тот же `view`.
    #[must_use]
    pub fn ui_view(&self) -> &wgpu::TextureView {
        self.ui_view.as_ref().unwrap_or(&self.view)
    }

    /// Формат `ui_view`.
    #[must_use]
    pub fn ui_format(&self) -> wgpu::TextureFormat {
        if self.ui_view.is_some() {
            CANVAS_FORMAT.remove_srgb_suffix()
        } else {
            CANVAS_FORMAT
        }
    }

    /// Есть ли линейный вид.
    #[must_use]
    pub fn has_linear_view(&self) -> bool {
        self.ui_view.is_some()
    }

    /// RGBA построчно сверху, без выравнивания строк; альфа 255: кадр непрозрачен.
    ///
    /// # Panics
    /// Если не отобразился буфер чтения.
    #[must_use]
    pub fn read_rgba(&self, gpu: &Gpu) -> Vec<u8> {
        let mut out = read_texture(gpu, &self.texture, self.size);
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        out
    }
}

/// Байты 4-байтовой текстуры построчно сверху, без выравнивания строк.
pub(crate) fn read_texture(gpu: &Gpu, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
    let (w, h) = size;
    let row = 4 * w;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map readback"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let mapped = readback.get_mapped_range(..).expect("mapped range");
    let mut out = Vec::with_capacity((row * h) as usize);
    for line in mapped.chunks_exact(padded as usize) {
        out.extend_from_slice(&line[..row as usize]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::{TOL, gpu_full};

    #[test]
    fn clear_reads_back_srgb_bytes_with_row_padding() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        // 70 × 4 = 280 байт в строке: не кратно 256, строка читается с паддингом.
        let canvas = Canvas::new(&gpu, (70, 5));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: canvas.view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.5,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        gpu.queue.submit([encoder.finish()]);

        let img = canvas.read_rgba(&gpu);
        assert_eq!(img.len(), 70 * 5 * 4);
        for px in img.as_chunks::<4>().0 {
            assert!(px[0].abs_diff(188) <= TOL, "{px:?}");
            assert_eq!((px[1], px[2], px[3]), (0, 255, 255), "{px:?}");
        }
    }

    #[test]
    fn views_follow_device_view_formats() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let canvas = Canvas::new(&gpu, (4, 4));
        assert_eq!(canvas.size(), (4, 4));
        let expected = if has_view_formats(&gpu) {
            CANVAS_FORMAT.remove_srgb_suffix()
        } else {
            CANVAS_FORMAT
        };
        assert_eq!(canvas.ui_format(), expected);
    }

    #[test]
    fn resize_changes_size() {
        let Some(gpu) = gpu_full() else {
            return;
        };
        let mut canvas = Canvas::new(&gpu, (4, 4));
        canvas.resize(&gpu, (8, 2));
        assert_eq!(canvas.size(), (8, 2));
        assert_eq!(canvas.read_rgba(&gpu).len(), 8 * 2 * 4);
    }
}
