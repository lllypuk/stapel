use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu_textures::DEPTH_FORMAT;
use crate::sprite_pipeline::{grow_capacity, instance_buffer, wgpu_color};
use crate::{
    Assets, Color, GpuTextures, MAX_LIGHTS, MeshInstance, PointLight, Scene, Vertex, batch_meshes,
};

/// Начальный размер буфера инстансов в байтах.
const INITIAL_CAPACITY: u64 = 1024;

/// Раскладка `Light` из `mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct LightUniform {
    pos: [f32; 3],
    range: f32,
    color: [f32; 4],
}

/// Раскладка `SceneUniform` из `mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    eye: [f32; 3],
    fog_start: f32,
    fog_end: f32,
    light_count: u32,
    /// WGSL выравнивает массив структур по 16 байт.
    _pad: [u32; 2],
    lights: [LightUniform; MAX_LIGHTS],
}

#[expect(clippy::cast_possible_truncation, reason = "GPU берёт цвет в f32")]
fn rgba(c: Color) -> [f32; 4] {
    [c.r as f32, c.g as f32, c.b as f32, c.a as f32]
}

impl SceneUniform {
    /// Uniform кадра; из `lights` берутся первые `MAX_LIGHTS`.
    fn new(scene: &Scene, aspect: f32) -> Self {
        let mut lights = [LightUniform::zeroed(); MAX_LIGHTS];
        for (dst, l) in lights.iter_mut().zip(&scene.lights) {
            *dst = LightUniform::from(l);
        }
        #[expect(clippy::cast_possible_truncation, reason = "не больше MAX_LIGHTS")]
        let light_count = scene.lights.len().min(MAX_LIGHTS) as u32;
        Self {
            view_proj: scene.camera.view_proj(aspect).to_cols_array_2d(),
            ambient: rgba(scene.ambient),
            fog_color: rgba(scene.fog.color),
            eye: scene.camera.pos.into(),
            fog_start: scene.fog.start,
            fog_end: scene.fog.end,
            light_count,
            _pad: [0; 2],
            lights,
        }
    }
}

impl From<&PointLight> for LightUniform {
    fn from(l: &PointLight) -> Self {
        Self {
            pos: l.pos.into(),
            range: l.range,
            color: rgba(l.color),
        }
    }
}

/// `true` при первом переполнении `MAX_LIGHTS` за жизнь `warned`.
fn first_overflow(warned: &mut bool, lights: usize) -> bool {
    let first = lights > MAX_LIGHTS && !*warned;
    *warned |= first;
    first
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// 0 — рисовать нечего: пустой буфер нельзя привязать.
    count: u32,
}

/// GPU-путь мешей: буферы мешей по хэндлу, инстансы и 3D-проход с глубиной.
pub struct MeshPipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    scene_buf: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: u64,
    /// Индекс — `Handle<Mesh>::index`.
    meshes: Vec<GpuMesh>,
    lights_warned: bool,
}

impl MeshPipeline {
    /// Пайплайн для рендера в цель формата `format` с текстурами из `textures`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        textures: &GpuTextures,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&scene_layout), Some(textures.layout())],
            immediate_size: 0,
        });
        let pipeline = render_pipeline(device, &shader, &layout, format);
        let scene_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buf.as_entire_binding(),
            }],
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            scene_buf,
            scene_group,
            instances: instance_buffer(device, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
            meshes: Vec::new(),
            lights_warned: false,
        }
    }

    /// Заливает на GPU меши, добавленные с прошлого вызова.
    ///
    /// # Panics
    /// Если меши пришли не из того же `Assets`, что прошлые.
    pub fn upload(&mut self, assets: &mut Assets) {
        for m in assets.take_pending_meshes() {
            assert_eq!(
                m.handle.index as usize,
                self.meshes.len(),
                "mesh handles must come from one Assets in order"
            );
            let buffer = |contents: &[u8], usage| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents,
                        usage,
                    })
            };
            #[expect(
                clippy::cast_possible_truncation,
                reason = "u32::MAX индексов не поместится в память"
            )]
            let count = if m.vertices.is_empty() {
                0
            } else {
                m.indices.len() as u32
            };
            self.meshes.push(GpuMesh {
                vertices: buffer(
                    bytemuck::cast_slice(&m.vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(bytemuck::cast_slice(&m.indices), wgpu::BufferUsages::INDEX),
                count,
            });
        }
    }

    /// Очищает `view` цветом `clear` и `depth` и рисует `scene`; aspect камеры — `depth`, он размера цели.
    /// Буферы пишутся через `write_buffer`: второй `draw` до `submit` затрёт первый.
    pub fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::Texture,
        textures: &GpuTextures,
        scene: &Scene,
        clear: Color,
    ) {
        let (instances, batches) = batch_meshes(&scene.meshes);
        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        if let Some(capacity) = grow_capacity(self.capacity, bytes.len() as u64) {
            self.instances = instance_buffer(&self.device, capacity);
            self.capacity = capacity;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytes);
        }
        if first_overflow(&mut self.lights_warned, scene.lights.len()) {
            log::warn!(
                "{} point lights, only the first {MAX_LIGHTS} are drawn",
                scene.lights.len()
            );
        }
        let aspect = depth.width() as f32 / depth.height() as f32;
        let uniform = SceneUniform::new(scene, aspect);
        self.queue
            .write_buffer(&self.scene_buf, 0, bytemuck::bytes_of(&uniform));

        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu_color(clear)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.scene_group, &[]);
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for b in batches {
            let mesh = &self.meshes[b.mesh.index as usize];
            if mesh.count == 0 {
                continue;
            }
            pass.set_bind_group(1, textures.group(b.texture), &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, b.range);
        }
    }
}

fn render_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_static"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x2,
                    ],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<MeshInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        3 => Float32x4,
                        4 => Float32x4,
                        5 => Float32x4,
                        6 => Float32x4,
                        7 => Float32x3,
                        8 => Float32x3,
                        9 => Float32x3,
                    ],
                }),
            ],
        },
        // Материалы Kenney двусторонние.
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{linear_to_srgb, srgb_to_linear};
    use crate::gpu_test::{SIZE, assert_pixel, gpu, load_png, offscreen};
    use crate::gpu_textures::depth_texture;
    use crate::{
        Camera2D, Camera3D, Fog, Gfx, Handle, Mesh, MeshDraw, Sprite, SpritePipeline, Texture,
        Vec2, Vec3, Viewport, vec2, vec3,
    };
    use glam::Mat4;
    use std::mem::offset_of;
    use wgpu::naga::TypeInner;

    const BLUE: [u8; 3] = [0, 0, 255];
    /// Цвет текстуры в тестах света и тумана.
    const TEX: [u8; 3] = [200, 100, 50];
    /// Пиксель центра кадра.
    const CENTER: (u32, u32) = (32, 32);

    fn gray(v: f64) -> Color {
        Color {
            r: v,
            g: v,
            b: v,
            a: 1.0,
        }
    }

    fn light(x: f32, z: f32, color: Color, range: f32) -> PointLight {
        PointLight {
            pos: vec3(x, 0.0, z),
            color,
            range,
        }
    }

    /// Смещения полей структуры `name` из `mesh.wgsl` и её размер.
    fn wgsl_layout(name: &str) -> (Vec<(String, usize)>, usize) {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("mesh.wgsl")).unwrap();
        module
            .types
            .iter()
            .find_map(|(_, t)| match &t.inner {
                TypeInner::Struct { members, span } if t.name.as_deref() == Some(name) => {
                    let fields = members
                        .iter()
                        .map(|m| (m.name.clone().unwrap(), m.offset as usize))
                        .collect();
                    Some((fields, *span as usize))
                }
                _ => None,
            })
            .unwrap()
    }

    fn fields(f: &[(&str, usize)]) -> Vec<(String, usize)> {
        f.iter().map(|&(n, o)| (n.to_owned(), o)).collect()
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        let light = fields(&[
            ("pos", offset_of!(LightUniform, pos)),
            ("range", offset_of!(LightUniform, range)),
            ("color", offset_of!(LightUniform, color)),
        ]);
        assert_eq!(wgsl_layout("Light"), (light, size_of::<LightUniform>()));
        let scene = fields(&[
            ("view_proj", offset_of!(SceneUniform, view_proj)),
            ("ambient", offset_of!(SceneUniform, ambient)),
            ("fog_color", offset_of!(SceneUniform, fog_color)),
            ("eye", offset_of!(SceneUniform, eye)),
            ("fog_start", offset_of!(SceneUniform, fog_start)),
            ("fog_end", offset_of!(SceneUniform, fog_end)),
            ("light_count", offset_of!(SceneUniform, light_count)),
            ("lights", offset_of!(SceneUniform, lights)),
        ]);
        assert_eq!(
            wgsl_layout("SceneUniform"),
            (scene, size_of::<SceneUniform>())
        );
    }

    #[test]
    fn uniform_takes_first_max_lights() {
        #[expect(clippy::cast_precision_loss, reason = "17 источников")]
        let lights: Vec<_> = (0..=MAX_LIGHTS)
            .map(|i| light(i as f32, 0.0, gray(0.5), 2.0))
            .collect();
        let scene = Scene {
            camera: Camera3D {
                pos: vec3(1.0, 2.0, 3.0),
                ..Default::default()
            },
            ambient: gray(0.25),
            fog: Fog {
                color: gray(0.75),
                start: 4.0,
                end: 9.0,
            },
            lights: lights.clone(),
            ..Default::default()
        };
        let u = SceneUniform::new(&scene, 1.5);

        assert_eq!(u.light_count as usize, MAX_LIGHTS);
        let packed: Vec<_> = lights[..MAX_LIGHTS]
            .iter()
            .map(LightUniform::from)
            .collect();
        assert_eq!(u.lights.as_slice(), packed);
        assert_eq!(
            u.lights[1],
            LightUniform {
                pos: [1.0, 0.0, 0.0],
                range: 2.0,
                color: [0.5, 0.5, 0.5, 1.0],
            }
        );
        assert_eq!(u.view_proj, scene.camera.view_proj(1.5).to_cols_array_2d());
        assert_eq!(u.ambient, [0.25, 0.25, 0.25, 1.0]);
        assert_eq!(u.fog_color, [0.75, 0.75, 0.75, 1.0]);
        assert_eq!(u.eye, [1.0, 2.0, 3.0]);
        assert_eq!((u.fog_start, u.fog_end), (4.0, 9.0));

        let few = Scene {
            lights: lights[..3].to_vec(),
            ..Default::default()
        };
        assert_eq!(SceneUniform::new(&few, 1.0).light_count, 3);
    }

    #[test]
    fn light_overflow_warns_once() {
        let mut warned = false;
        assert!(!first_overflow(&mut warned, MAX_LIGHTS));
        assert!(!warned);
        assert!(first_overflow(&mut warned, MAX_LIGHTS + 1));
        assert!(!first_overflow(&mut warned, MAX_LIGHTS + 1));
        assert!(warned);
    }

    /// Куб со стороной 1 и центром в начале координат.
    fn cube(assets: &mut Assets) -> Handle<Mesh> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for n in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ] {
            let u = n.any_orthonormal_vector();
            let v = n.cross(u);
            #[expect(clippy::cast_possible_truncation, reason = "24 вершины")]
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                vertices.push(Vertex {
                    pos: (0.5 * (n + a * u + b * v)).into(),
                    normal: n.into(),
                    uv: [0.5, 0.5],
                    ..Default::default()
                });
            }
            indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        }
        assets.add_mesh(vertices, indices)
    }

    fn at(mesh: Handle<Mesh>, texture: Handle<Texture>, z: f32, size: f32) -> MeshDraw {
        MeshDraw {
            mesh,
            texture,
            transform: Mat4::from_translation(vec3(0.0, 0.0, z))
                * Mat4::from_scale(Vec3::splat(size)),
        }
    }

    fn gfx(scene: Scene) -> Gfx {
        Gfx {
            clear_color: Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            },
            camera: Camera2D {
                center: Vec2::splat(SIZE as f32 / 2.0),
                size: Vec2::splat(SIZE as f32),
            },
            scene: Some(scene),
            ..Default::default()
        }
    }

    /// Кадр как в `Renderer`: 3D с очисткой, затем спрайты с `Load`.
    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        gfx: &Gfx,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut textures = GpuTextures::new(device, queue);
        textures.upload(assets);
        let mut meshes = MeshPipeline::new(device, queue, format, &textures);
        meshes.upload(assets);
        let mut sprites = SpritePipeline::new(device, queue, format, &textures);
        let depth = depth_texture(device, (SIZE, SIZE));
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            w: SIZE as f32,
            h: SIZE as f32,
        };
        offscreen(device, queue, |encoder, view| {
            let scene = gfx.scene.as_ref().unwrap();
            meshes.draw(encoder, view, &depth, &textures, scene, gfx.clear_color);
            sprites.draw(encoder, view, &textures, viewport, gfx, false);
        })
    }

    #[test]
    fn near_cube_hides_far_in_any_order() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        // Батчи идут по текстуре: `red` раньше `green`, поэтому смена цветов меняет порядок рисования.
        for swap in [false, true] {
            let mut assets = Assets::default();
            let cube = cube(&mut assets);
            let red = load_png(&mut assets, "mesh-red", 1, 1, &[255, 0, 0, 255]);
            let green = load_png(&mut assets, "mesh-green", 1, 1, &[0, 255, 0, 255]);
            let (near, far, near_rgb, far_rgb) = if swap {
                (green, red, [0, 255, 0], [255, 0, 0])
            } else {
                (red, green, [255, 0, 0], [0, 255, 0])
            };
            let scene = Scene {
                ambient: Color::WHITE,
                meshes: vec![at(cube, far, -6.0, 4.0), at(cube, near, -3.0, 1.0)],
                ..Default::default()
            };
            let img = render(&device, &queue, &mut assets, &gfx(scene));
            assert_pixel(&img, "near", (32, 32), near_rgb);
            assert_pixel(&img, "far around near", (47, 32), far_rgb);
            assert_pixel(&img, "clear", (1, 1), BLUE);
        }
    }

    #[test]
    fn unlit_color_is_ambient_times_texture_and_sprites_stay_on_top() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut assets = Assets::default();
        let cube = cube(&mut assets);
        let no_vertices = assets.add_mesh(Vec::new(), vec![0, 1, 2]);
        let no_indices = assets.add_mesh(vec![Vertex::default(); 3], Vec::new());
        let tex = load_png(&mut assets, "mesh-ambient", 1, 1, &[200, 100, 50, 255]);

        let ambient = Color {
            r: 0.5,
            g: 0.25,
            b: 1.0,
            a: 1.0,
        };
        // Заполнитель — за начальной ёмкостью; видимый куб, вокруг которого камера, — последний.
        let filler = 11;
        assert!((filler * size_of::<MeshInstance>()) as u64 > INITIAL_CAPACITY);
        let mut meshes = vec![at(cube, tex, -50.0, 0.1); filler];
        meshes.push(at(cube, tex, 0.0, 10.0));
        meshes.push(at(no_vertices, tex, -3.0, 1.0));
        meshes.push(at(no_indices, tex, -3.0, 1.0));
        let mut gfx = gfx(Scene {
            ambient,
            meshes,
            ..Default::default()
        });
        gfx.draw_sprite(Sprite {
            pos: vec2(8.0, 8.0),
            size: Vec2::splat(8.0),
            ..Default::default()
        });

        let img = render(&device, &queue, &mut assets, &gfx);

        let lit = |c: u8, k: f64| linear_to_srgb(srgb_to_linear(c) * k);
        let expected = [lit(200, ambient.r), lit(100, ambient.g), lit(50, ambient.b)];
        assert_pixel(&img, "ambient × texture", (40, 40), expected);
        assert_pixel(&img, "sprite over 3d", (8, 8), [255, 255, 255]);
        assert_pixel(&img, "3d next to sprite", (16, 8), expected);
    }

    /// Центр кадра — передняя грань куба на `z = -3`, в 4 м от камеры на `z = 1`.
    /// `test` — имя временного PNG: у параллельных тестов оно разное.
    fn render_face(test: &str, ambient: f64, lights: Vec<PointLight>, fog: Fog) -> Vec<u8> {
        let (device, queue) = gpu().unwrap();
        let mut assets = Assets::default();
        let cube = cube(&mut assets);
        let [r, g, b] = TEX;
        let tex = load_png(&mut assets, test, 1, 1, &[r, g, b, 255]);
        let scene = Scene {
            ambient: gray(ambient),
            fog,
            lights,
            meshes: vec![at(cube, tex, -4.0, 2.0)],
            camera: Camera3D {
                pos: vec3(0.0, 0.0, 1.0),
                ..Default::default()
            },
            ..Default::default()
        };
        render(&device, &queue, &mut assets, &gfx(scene))
    }

    /// Текстура `TEX`, освещённая линейным множителем `k`.
    fn lit(k: f64) -> [u8; 3] {
        TEX.map(|c| linear_to_srgb(srgb_to_linear(c) * k))
    }

    #[test]
    fn point_light_lambert_and_falloff() {
        if gpu().is_none() {
            return;
        }
        // d = 2, range = 4: (1 - 2/4)² = 0.25 поверх ambient 0.2.
        let toward = render_face(
            "mesh-falloff",
            0.2,
            vec![light(0.0, -1.0, Color::WHITE, 4.0)],
            Fog::default(),
        );
        assert_pixel(&toward, "face toward light", CENTER, lit(0.45));
        let away = render_face(
            "mesh-falloff",
            0.2,
            vec![light(0.0, -6.0, Color::WHITE, 10.0)],
            Fog::default(),
        );
        assert_pixel(&away, "face away from light", CENTER, lit(0.2));
        let beyond = render_face(
            "mesh-falloff",
            0.2,
            vec![light(0.0, -1.0, Color::WHITE, 0.5)],
            Fog::default(),
        );
        assert_pixel(&beyond, "beyond range", CENTER, lit(0.2));
    }

    #[test]
    fn sixteenth_light_shines_seventeenth_dropped() {
        if gpu().is_none() {
            return;
        }
        let with_white_at = |i: usize| {
            let mut lights = vec![light(0.0, -1.0, Color::BLACK, 4.0); i];
            lights.push(light(0.0, -1.0, Color::WHITE, 4.0));
            lights
        };
        let sixteenth = render_face(
            "mesh-max-lights",
            0.2,
            with_white_at(MAX_LIGHTS - 1),
            Fog::default(),
        );
        assert_pixel(&sixteenth, "16th light", CENTER, lit(0.45));
        let seventeenth = render_face(
            "mesh-max-lights",
            0.2,
            with_white_at(MAX_LIGHTS),
            Fog::default(),
        );
        assert_pixel(&seventeenth, "17th light", CENTER, lit(0.2));
    }

    #[test]
    fn fog_by_distance_from_camera() {
        if gpu().is_none() {
            return;
        }
        let fogged = |start, end| {
            let fog = Fog {
                color: gray(0.5),
                start,
                end,
            };
            render_face("mesh-fog", 1.0, Vec::new(), fog)
        };
        assert_pixel(&fogged(5.0, 10.0), "before start", CENTER, lit(1.0));
        let fog_rgb = [linear_to_srgb(0.5); 3];
        assert_pixel(&fogged(1.0, 3.0), "past end", CENTER, fog_rgb);
        // Расстояние 4 от камеры, а не 3 от начала координат: туман ровно наполовину.
        let half = TEX.map(|c| linear_to_srgb(0.5 * srgb_to_linear(c) + 0.25));
        assert_pixel(&fogged(2.0, 6.0), "halfway", CENTER, half);
    }
}
