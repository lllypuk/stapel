use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::gpu_textures::DEPTH_FORMAT;
use crate::sprite_pipeline::{grow_capacity, instance_buffer, wgpu_color};
use crate::{Assets, Color, GpuTextures, MeshInstance, Scene, Vertex, batch_meshes};

/// Начальный размер буфера инстансов в байтах.
const INITIAL_CAPACITY: u64 = 1024;

/// Раскладка `SceneUniform` из `mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    ambient: [f32; 4],
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
        let aspect = depth.width() as f32 / depth.height() as f32;
        let a = scene.ambient;
        #[expect(clippy::cast_possible_truncation, reason = "GPU берёт цвет в f32")]
        let uniform = SceneUniform {
            view_proj: scene.camera.view_proj(aspect).to_cols_array_2d(),
            ambient: [a.r as f32, a.g as f32, a.b as f32, 1.0],
        };
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
        Camera2D, Gfx, Handle, Mesh, MeshDraw, Sprite, SpritePipeline, Texture, Vec2, Vec3,
        Viewport, vec2, vec3,
    };
    use glam::Mat4;

    const BLUE: [u8; 3] = [0, 0, 255];

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
}
