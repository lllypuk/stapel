use crate::{Assets, Handle, Texture, TextureKind};

/// Текстуры `Assets` на GPU: bind group «вид + сэмплер» на хэндл, общие для всех конвейеров.
pub struct GpuTextures {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    /// Индекс — `Handle<Texture>::index`.
    groups: Vec<wgpu::BindGroup>,
}

impl GpuTextures {
    /// Пустая таблица; layout видим фрагментному шейдеру.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            device: device.clone(),
            queue: queue.clone(),
            layout,
            nearest: sampler("texture nearest", wgpu::FilterMode::Nearest),
            linear: sampler("texture linear", wgpu::FilterMode::Linear),
            groups: Vec::new(),
        }
    }

    /// Layout bind group текстуры для конвейеров.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Bind group залитой текстуры.
    ///
    /// # Panics
    /// Если текстура ещё не залита `upload`.
    pub fn group(&self, texture: Handle<Texture>) -> &wgpu::BindGroup {
        &self.groups[texture.index as usize]
    }

    /// Заливает на GPU текстуры, загруженные с прошлого вызова; хэндл уже залитой — замена.
    ///
    /// # Panics
    /// Если текстуры пришли не из того же `Assets`, что прошлые.
    pub fn upload(&mut self, assets: &mut Assets) {
        for t in assets.take_pending() {
            let index = t.handle.index as usize;
            assert!(
                index <= self.groups.len(),
                "texture handles must come from one Assets in order"
            );
            let (format, sampler) = match t.kind {
                TextureKind::Color => (wgpu::TextureFormat::Rgba8UnormSrgb, &self.nearest),
                TextureKind::Coverage => (wgpu::TextureFormat::Rgba8Unorm, &self.linear),
            };
            let size = wgpu::Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                texture.as_image_copy(),
                &t.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * t.width),
                    rows_per_image: Some(t.height),
                },
                size,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            if index == self.groups.len() {
                self.groups.push(group);
            } else {
                self.groups[index] = group;
            }
        }
    }
}

/// Формат буфера глубины 3D-прохода.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Буфер глубины размера цели; при смене размера цели — новый, иначе проход падает на валидации.
pub(crate) fn depth_texture(device: &wgpu::Device, (width, height): (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::{SIZE, assert_pixel, gpu, offscreen};
    use crate::{Camera2D, Color, Gfx, Sprite, SpritePipeline, Vec2, Viewport, vec2};

    #[test]
    fn reloaded_texture_replaces_its_slot() {
        let Some((device, queue)) = gpu() else {
            return;
        };

        let path = std::env::temp_dir().join(format!(
            "engine-gpu-textures-{}-reload.png",
            std::process::id()
        ));
        let save = |rgba: &[u8]| {
            image::save_buffer(&path, rgba, 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        };
        save(&[255, 0, 0, 255]);
        let mut assets = Assets::default();
        let h = assets.load_texture(&path);
        save(&[0, 255, 0, 255]);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        assets.reload_changed();
        std::fs::remove_file(&path).unwrap();

        let mut gfx = Gfx {
            clear_color: Color::BLACK,
            camera: Camera2D {
                center: Vec2::splat(SIZE as f32 / 2.0),
                size: Vec2::splat(SIZE as f32),
            },
            ..Default::default()
        };
        gfx.draw_sprite(Sprite {
            texture: h,
            pos: vec2(32.0, 32.0),
            size: Vec2::splat(16.0),
            ..Default::default()
        });

        // Одна пачка `[0, 1, 1]`: слот 1 создаётся и тут же заменяется.
        let mut textures = GpuTextures::new(&device, &queue);
        textures.upload(&mut assets);
        let mut sprites = SpritePipeline::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &textures,
        );
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: SIZE as f32,
            h: SIZE as f32,
        };
        let img = offscreen(&device, &queue, |encoder, view| {
            sprites.draw(encoder, view, &textures, viewport, &gfx, true);
        });

        assert_pixel(&img, "reloaded", (32, 32), [0, 255, 0]);
    }
}
