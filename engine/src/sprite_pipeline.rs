use crate::{Assets, Gfx, Instance, Viewport, batch};

/// Начальный размер буфера инстансов в байтах.
const INITIAL_CAPACITY: u64 = 1024;

/// GPU-путь спрайтов: текстуры, буфер инстансов и проход с очисткой.
pub struct SpritePipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    camera_buf: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    /// Индекс — `TextureId`.
    textures: Vec<wgpu::BindGroup>,
    instances: wgpu::Buffer,
    capacity: u64,
}

impl SpritePipeline {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("sprite.wgsl"));

        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite texture"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&camera_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: size_of::<[f32; 16]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            texture_layout,
            sampler,
            camera_buf,
            camera_group,
            textures: Vec::new(),
            instances: instance_buffer(device, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
        }
    }

    /// Заливает на GPU текстуры, загруженные с прошлого вызова.
    pub fn upload(&mut self, assets: &mut Assets) {
        for t in assets.take_pending() {
            assert_eq!(
                t.id.0 as usize,
                self.textures.len(),
                "texture ids must come from one Assets in order"
            );
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
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            self.textures
                .push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                }));
        }
    }

    /// Очищает `view` цветом `gfx.clear_color` и рисует спрайты внутри `viewport`.
    /// Буферы пишутся через `write_buffer`: второй `draw` до `submit` затрёт первый.
    pub fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: Viewport,
        gfx: &Gfx,
    ) {
        let (instances, batches) = batch(&gfx.sprites);
        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        // Сначала рост: запись за край буфера — паника валидации.
        if let Some(capacity) = grow_capacity(self.capacity, bytes.len() as u64) {
            self.instances = instance_buffer(&self.device, capacity);
            self.capacity = capacity;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytes);
        }
        self.queue.write_buffer(
            &self.camera_buf,
            0,
            bytemuck::bytes_of(&gfx.camera.view_proj().to_cols_array()),
        );

        let c = gfx.clear_color;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: c.r,
                        g: c.g,
                        b: c.b,
                        a: c.a,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if batches.is_empty() {
            return;
        }
        pass.set_viewport(viewport.x, viewport.y, viewport.w, viewport.h, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        for b in batches {
            pass.set_bind_group(1, &self.textures[b.texture.0 as usize], &[]);
            pass.draw(0..6, b.range);
        }
    }
}

fn instance_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite instances"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Новая ёмкость в байтах, если `needed` не влезает: степень двойки, не меньше 4 — выравнивания копий.
fn grow_capacity(current: u64, needed: u64) -> Option<u64> {
    (needed > current).then(|| needed.next_power_of_two().max(wgpu::COPY_BUFFER_ALIGNMENT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::linear_to_srgb;
    use crate::{Camera2D, Color, Sprite, TextureId, Vec2, vec2};

    const SIZE: u32 = 64;
    const TOL: u8 = 2;

    #[test]
    fn grow_capacity_only_when_short() {
        assert_eq!(grow_capacity(1024, 1024), None);
        assert_eq!(grow_capacity(1024, 0), None);
        assert_eq!(grow_capacity(1024, 1056), Some(2048));
        assert_eq!(grow_capacity(0, 1), Some(4));
        assert_eq!(grow_capacity(0, 3), Some(4));
        assert_eq!(grow_capacity(4, 5), Some(8));
    }

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = match pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        ) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("SKIPPED: no GPU adapter: {e}");
                return None;
            }
        };
        eprintln!("adapter: {:?}", adapter.get_info());
        Some(
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device"),
        )
    }

    fn load_png(assets: &mut Assets, name: &str, w: u32, h: u32, rgba: &[u8]) -> TextureId {
        let path = std::env::temp_dir().join(format!(
            "engine-sprite-pipeline-{}-{name}.png",
            std::process::id()
        ));
        image::save_buffer(&path, rgba, w, h, image::ExtendedColorType::Rgba8).unwrap();
        let id = assets.load_texture(&path);
        std::fs::remove_file(&path).unwrap();
        id
    }

    fn sprite(texture: TextureId, pos: Vec2, size: f32, color: Color, layer: i32) -> Sprite {
        Sprite {
            texture,
            pos,
            size: Vec2::splat(size),
            color,
            layer,
        }
    }

    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        gfx: &Gfx,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut pipeline = SpritePipeline::new(device, queue, format);
        pipeline.upload(assets);

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
            format,
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
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: SIZE as f32,
            h: SIZE as f32,
        };
        pipeline.draw(&mut encoder, &view, viewport, gfx);
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
    fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 3] {
        let i = (4 * ((SIZE - 1 - y) * SIZE + x)) as usize;
        [img[i], img[i + 1], img[i + 2]]
    }

    fn assert_pixel(img: &[u8], what: &str, (x, y): (u32, u32), expected: [u8; 3]) {
        let got = pixel(img, x, y);
        let close = got.iter().zip(expected).all(|(&g, e)| g.abs_diff(e) <= TOL);
        assert!(
            close,
            "{what} at ({x}, {y}): got {got:?}, expected {expected:?}"
        );
    }

    #[test]
    fn offscreen_render() {
        let Some((device, queue)) = gpu() else {
            return;
        };

        let mut assets = Assets::default();
        let uv = load_png(
            &mut assets,
            "uv",
            2,
            2,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 0, 255,
            ],
        );
        let half = load_png(&mut assets, "half", 1, 1, &[255, 255, 255, 128]);
        let top = load_png(&mut assets, "top", 1, 1, &[255, 0, 255, 255]);
        let bottom = load_png(&mut assets, "bottom", 1, 1, &[0, 255, 255, 255]);
        assert!(top < bottom, "by texture alone bottom would be drawn last");

        let bg = Color {
            r: 0.05,
            g: 0.1,
            b: 0.2,
            a: 1.0,
        };
        let mut gfx = Gfx {
            clear_color: bg,
            camera: Camera2D {
                center: Vec2::splat(SIZE as f32 / 2.0),
                size: Vec2::splat(SIZE as f32),
            },
            sprites: Vec::new(),
        };
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };

        // Заполнитель в нижнем слое: все проверяемые батчи начинаются за начальной ёмкостью.
        let filler = 40;
        assert!(filler * size_of::<Instance>() as u64 > INITIAL_CAPACITY);
        for _ in 0..filler {
            gfx.draw_sprite(sprite(
                TextureId::WHITE,
                vec2(60.0, 4.0),
                2.0,
                Color::BLACK,
                -1,
            ));
        }
        // Левая верхняя четверть.
        gfx.draw_sprite(sprite(uv, vec2(16.0, 48.0), 32.0, white, 0));
        // Правая верхняя: `top` подан первым, но выше по слою.
        gfx.draw_sprite(sprite(top, vec2(44.0, 52.0), 16.0, white, 1));
        gfx.draw_sprite(sprite(bottom, vec2(52.0, 44.0), 16.0, white, 0));
        // Нижняя половина.
        gfx.draw_sprite(sprite(
            TextureId::WHITE,
            vec2(8.0, 24.0),
            12.0,
            Color::from_srgb8(128, 128, 128),
            0,
        ));
        let tint = Color { a: 0.5, ..white };
        gfx.draw_sprite(sprite(half, vec2(24.0, 24.0), 12.0, tint, 0));
        let last = Color::from_srgb8(200, 100, 50);
        gfx.draw_sprite(sprite(TextureId::WHITE, vec2(48.0, 16.0), 16.0, last, 2));

        let img = render(&device, &queue, &mut assets, &gfx);

        assert_pixel(&img, "uv top-left", (8, 56), [255, 0, 0]);
        assert_pixel(&img, "uv top-right", (24, 56), [0, 255, 0]);
        assert_pixel(&img, "uv bottom-left", (8, 40), [0, 0, 255]);
        assert_pixel(&img, "uv bottom-right", (24, 40), [255, 255, 0]);

        assert_pixel(&img, "layer overlap", (48, 48), [255, 0, 255]);
        assert_pixel(&img, "layer bottom only", (56, 40), [0, 255, 255]);

        assert_pixel(&img, "srgb write", (8, 24), [128, 128, 128]);

        // Премультиплицированный белый с α = 128/255 и tint.a = 0.5 поверх фона.
        let a = 128.0 / 255.0 * 0.5;
        let over = |dst: f64| linear_to_srgb(a + dst * (1.0 - a));
        assert_pixel(
            &img,
            "double premultiply",
            (24, 24),
            [over(bg.r), over(bg.g), over(bg.b)],
        );

        assert_pixel(&img, "after buffer growth", (48, 16), [200, 100, 50]);
        assert_pixel(
            &img,
            "background",
            (2, 2),
            [
                linear_to_srgb(bg.r),
                linear_to_srgb(bg.g),
                linear_to_srgb(bg.b),
            ],
        );
    }
}
