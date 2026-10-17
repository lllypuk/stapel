use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::gpu_textures::DEPTH_FORMAT;
use crate::scene::rgba;
use crate::sprite_pipeline::{grow_capacity, instance_buffer, wgpu_color};
use crate::{
    Assets, Camera3D, Color, ExpandedModels, GpuTextures, MAX_JOINTS, MAX_LIGHTS, MeshDraw,
    MeshInstance, PointLight, Scene, Vertex, batch_meshes,
};

/// Начальный размер буфера инстансов в байтах.
const INITIAL_CAPACITY: u64 = 1024;
/// Шаг палитр в буфере и размер привязки одной палитры: `array<mat4x4<f32>, 64>` из `mesh.wgsl`.
const PALETTE_STRIDE: u64 = (MAX_JOINTS * size_of::<[[f32; 4]; 4]>()) as u64;

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
    sun_color: [f32; 4],
    eye: [f32; 3],
    fog_start: f32,
    sun_dir: [f32; 3],
    fog_end: f32,
    light_count: u32,
    /// WGSL выравнивает массив структур по 16 байт.
    _pad: [u32; 3],
    lights: [LightUniform; MAX_LIGHTS],
}

impl SceneUniform {
    /// Uniform прохода с камерой `camera`; из `lights` берутся первые `MAX_LIGHTS`.
    fn new(scene: &Scene, camera: &Camera3D, aspect: f32) -> Self {
        let mut lights = [LightUniform::zeroed(); MAX_LIGHTS];
        for (dst, l) in lights.iter_mut().zip(&scene.lights) {
            *dst = LightUniform::from(l);
        }
        #[expect(clippy::cast_possible_truncation, reason = "не больше MAX_LIGHTS")]
        let light_count = scene.lights.len().min(MAX_LIGHTS) as u32;
        let (sun_dir, sun_color) = scene.sun.map_or((Vec3::ZERO, [0.0; 4]), |s| {
            (s.dir.normalize_or_zero(), rgba(s.color))
        });
        Self {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            ambient: rgba(scene.ambient),
            fog_color: rgba(scene.fog.color),
            sun_color,
            eye: camera.pos.into(),
            fog_start: scene.fog.start,
            sun_dir: sun_dir.into(),
            fog_end: scene.fog.end,
            light_count,
            _pad: [0; 3],
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

/// `true` при первом `n > max` за жизнь `warned`.
fn first_overflow(warned: &mut bool, n: usize, max: usize) -> bool {
    let first = n > max && !*warned;
    *warned |= first;
    first
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// 0 — рисовать нечего: пустой буфер нельзя привязать.
    count: u32,
}

/// Проход мира.
const WORLD: usize = 0;
/// Проход слоя оружия.
const VIEWMODEL: usize = 1;

/// Uniform, инстансы и палитры одного прохода: `write_buffer` слоя в общие буферы затёр бы мир до `submit`.
struct PassBuffers {
    scene_buf: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: u64,
    palettes: wgpu::Buffer,
    palette_group: wgpu::BindGroup,
    palette_capacity: u64,
}

impl PassBuffers {
    fn new(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        palette_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let palettes = palette_buffer(device, PALETTE_STRIDE);
        let palette_group = palette_group(device, palette_layout, &palettes);
        let scene_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buf.as_entire_binding(),
            }],
        });
        Self {
            scene_buf,
            scene_group,
            instances: instance_buffer(device, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
            palettes,
            palette_group,
            palette_capacity: PALETTE_STRIDE,
        }
    }

    /// Пишет `instances`, растя буфер.
    fn write_instances(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if let Some(capacity) = grow_capacity(self.capacity, bytes.len() as u64) {
            self.instances = instance_buffer(device, capacity);
            self.capacity = capacity;
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, bytes);
        }
    }

    /// Пишет `palettes` с шагом `PALETTE_STRIDE`; от длинной палитры — первые `MAX_JOINTS`.
    fn write_palettes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        palettes: &[Vec<Mat4>],
    ) {
        let mut data = vec![[[0.0f32; 4]; 4]; palettes.len() * MAX_JOINTS];
        for (dst, p) in data.chunks_mut(MAX_JOINTS).zip(palettes) {
            for (d, m) in dst.iter_mut().zip(p) {
                *d = m.to_cols_array_2d();
            }
        }
        let bytes: &[u8] = bytemuck::cast_slice(&data);
        if let Some(capacity) = grow_capacity(self.palette_capacity, bytes.len() as u64) {
            self.palettes = palette_buffer(device, capacity);
            self.palette_group = palette_group(device, layout, &self.palettes);
            self.palette_capacity = capacity;
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.palettes, 0, bytes);
        }
    }
}

/// GPU-путь мешей: буферы мешей по хэндлу, инстансы и 3D-проходы мира и слоя с глубиной.
pub struct MeshPipeline {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    palette_layout: wgpu::BindGroupLayout,
    /// Индекс — `WORLD` или `VIEWMODEL`.
    passes: [PassBuffers; 2],
    /// Индекс — `Handle<Mesh>::index`.
    meshes: Vec<GpuMesh>,
    lights_warned: bool,
    joints_warned: bool,
}

impl MeshPipeline {
    /// Пайплайн для рендера в цель формата `format` с текстурами из `textures`.
    ///
    /// # Panics
    /// Если шаг палитр не кратен `min_uniform_buffer_offset_alignment` устройства.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        textures: &GpuTextures,
    ) -> Self {
        let align = device.limits().min_uniform_buffer_offset_alignment;
        assert!(
            PALETTE_STRIDE.is_multiple_of(u64::from(align)),
            "palette stride {PALETTE_STRIDE} is not a multiple of uniform offset alignment {align}"
        );
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
        let pipeline = render_pipeline(
            device,
            &shader,
            &layout,
            format,
            "vs_static",
            &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
        );
        let palette_layout = palette_layout(device);
        let skinned_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skinned mesh"),
            bind_group_layouts: &[
                Some(&scene_layout),
                Some(textures.layout()),
                Some(&palette_layout),
            ],
            immediate_size: 0,
        });
        let skinned = render_pipeline(
            device,
            &shader,
            &skinned_layout,
            format,
            "vs_skinned",
            &wgpu::vertex_attr_array![
                0 => Float32x3,
                1 => Float32x3,
                2 => Float32x2,
                10 => Uint16x4,
                11 => Float32x4,
            ],
        );
        let passes = [(); 2].map(|()| PassBuffers::new(device, &scene_layout, &palette_layout));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            skinned,
            palette_layout,
            passes,
            meshes: Vec::new(),
            lights_warned: false,
            joints_warned: false,
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

    /// Рисует мир `scene` с очисткой цвета `clear`, затем её `viewmodel` поверх мира; aspect камеры — `depth`, он размера цели.
    #[expect(clippy::too_many_arguments, reason = "цель, глубина, текстуры и кадр")]
    pub fn draw_scene(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::Texture,
        textures: &GpuTextures,
        scene: &Scene,
        assets: &Assets,
        clear: Color,
    ) {
        if first_overflow(&mut self.lights_warned, scene.lights.len(), MAX_LIGHTS) {
            log::warn!(
                "{} point lights, only the first {MAX_LIGHTS} are drawn",
                scene.lights.len()
            );
        }
        let world = ExpandedModels::new(&scene.models, assets);
        self.pass(
            WORLD,
            encoder,
            view,
            depth,
            textures,
            scene,
            &scene.camera,
            &scene.meshes,
            &world,
            Some(clear),
        );
        if let Some(vm) = &scene.viewmodel {
            let camera = Camera3D {
                fov_y: vm.fov_y,
                near: vm.near,
                ..scene.camera
            };
            let models = ExpandedModels::new(&vm.models, assets);
            self.pass(
                VIEWMODEL,
                encoder,
                view,
                depth,
                textures,
                scene,
                &camera,
                &[],
                &models,
                None,
            );
        }
    }

    /// Проход `index`: `meshes` и `models` под светом `scene` с камерой `camera`; глубина чистится, цвет — при `Some(clear)`.
    #[expect(
        clippy::too_many_arguments,
        reason = "проход, цель, глубина, текстуры и кадр"
    )]
    fn pass(
        &mut self,
        index: usize,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::Texture,
        textures: &GpuTextures,
        scene: &Scene,
        camera: &Camera3D,
        meshes: &[MeshDraw],
        models: &ExpandedModels,
        clear: Option<Color>,
    ) {
        let (mut instances, batches) = batch_meshes(meshes.iter().chain(&models.meshes));
        let longest = models.palettes.iter().map(Vec::len).max().unwrap_or(0);
        if first_overflow(&mut self.joints_warned, longest, MAX_JOINTS) {
            log::warn!("skin with {longest} joints, over {MAX_JOINTS} are not drawn");
        }
        let skinned: Vec<_> = models
            .skinned
            .iter()
            .filter(|d| models.palettes[d.palette].len() <= MAX_JOINTS)
            .collect();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX мешей не поместится в память"
        )]
        let first_skinned = instances.len() as u32;
        instances.extend(
            skinned
                .iter()
                .map(|d| MeshInstance::new(d.transform, d.color)),
        );
        let buf = &mut self.passes[index];
        buf.write_palettes(
            &self.device,
            &self.queue,
            &self.palette_layout,
            &models.palettes,
        );
        buf.write_instances(&self.device, &self.queue, bytemuck::cast_slice(&instances));
        let aspect = depth.width() as f32 / depth.height() as f32;
        let uniform = SceneUniform::new(scene, camera, aspect);
        self.queue
            .write_buffer(&buf.scene_buf, 0, bytemuck::bytes_of(&uniform));

        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let load = clear.map_or(wgpu::LoadOp::Load, |c| wgpu::LoadOp::Clear(wgpu_color(c)));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
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
        pass.set_bind_group(0, &buf.scene_group, &[]);
        pass.set_vertex_buffer(1, buf.instances.slice(..));
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
        pass.set_pipeline(&self.skinned);
        for (i, d) in (first_skinned..).zip(skinned) {
            let mesh = &self.meshes[d.mesh.index as usize];
            if mesh.count == 0 {
                continue;
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "смещение ограничено max_buffer_size"
            )]
            let offset = (d.palette as u64 * PALETTE_STRIDE) as u32;
            pass.set_bind_group(1, textures.group(d.texture), &[]);
            pass.set_bind_group(2, &buf.palette_group, &[offset]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, i..i + 1);
        }
    }
}

fn palette_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("palette"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(PALETTE_STRIDE),
            },
            count: None,
        }],
    })
}

fn palette_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("palettes"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn palette_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("palettes"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: wgpu::BufferSize::new(PALETTE_STRIDE),
            }),
        }],
    })
}

fn render_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    entry: &str,
    vertex: &[wgpu::VertexAttribute],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: vertex,
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
                        12 => Float32x4,
                    ],
                }),
            ],
        },
        // Без отсечения: Racing Kit не `doubleSided`, но Car Kit и Mini Dungeon — да.
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
        Camera2D, Camera3D, DirectionalLight, Fog, Gfx, Handle, Mesh, MeshDraw, SkinnedDraw,
        Sprite, SpritePipeline, Texture, Vec2, Vec3, Viewmodel, Viewport, vec2, vec3,
    };
    use std::f64::consts::FRAC_1_SQRT_2;
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
            ("sun_color", offset_of!(SceneUniform, sun_color)),
            ("eye", offset_of!(SceneUniform, eye)),
            ("fog_start", offset_of!(SceneUniform, fog_start)),
            ("sun_dir", offset_of!(SceneUniform, sun_dir)),
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
        let u = SceneUniform::new(&scene, &scene.camera, 1.5);

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
        assert_eq!(SceneUniform::new(&few, &few.camera, 1.0).light_count, 3);
    }

    #[test]
    fn uniform_sun_is_normalized_and_none_is_dark() {
        let sun = Scene {
            sun: Some(DirectionalLight {
                dir: vec3(0.0, -3.0, 4.0),
                color: gray(0.5),
            }),
            ..Default::default()
        };
        let u = SceneUniform::new(&sun, &sun.camera, 1.0);
        assert_eq!(u.sun_dir, [0.0, -0.6, 0.8]);
        assert_eq!(u.sun_color, [0.5, 0.5, 0.5, 1.0]);
        let none = SceneUniform::new(&Scene::default(), &Camera3D::default(), 1.0);
        assert_eq!((none.sun_dir, none.sun_color), ([0.0; 3], [0.0; 4]));
    }

    #[test]
    fn overflow_warns_once() {
        let mut warned = false;
        assert!(!first_overflow(&mut warned, MAX_LIGHTS, MAX_LIGHTS));
        assert!(!warned);
        assert!(first_overflow(&mut warned, MAX_LIGHTS + 1, MAX_LIGHTS));
        assert!(!first_overflow(&mut warned, MAX_LIGHTS + 1, MAX_LIGHTS));
        assert!(warned);
    }

    #[test]
    fn palette_stride_is_multiple_of_device_alignment() {
        let Some((device, _)) = gpu() else {
            return;
        };
        let align = device.limits().min_uniform_buffer_offset_alignment;
        assert_eq!(PALETTE_STRIDE % u64::from(align), 0);
        assert_eq!(PALETTE_STRIDE, 4096);
    }

    /// Куб со стороной 1 и центром в начале координат.
    fn cube(assets: &mut Assets) -> Handle<Mesh> {
        skinned_cube(assets, [0; 4], [0.0; 4])
    }

    /// `cube`, все вершины которого смешивают суставы `joints` с весами `weights`.
    fn skinned_cube(assets: &mut Assets, joints: [u16; 4], weights: [f32; 4]) -> Handle<Mesh> {
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
                    joints,
                    weights,
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
            color: Color::WHITE,
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

    /// Кадр как в `Renderer`: `draw_scene`, затем спрайты с `Load`.
    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        gfx: &Gfx,
    ) -> Vec<u8> {
        let scene = gfx.scene.as_ref().unwrap();
        frame(
            device,
            queue,
            assets,
            gfx,
            |p, encoder, view, depth, textures, assets| {
                p.draw_scene(
                    encoder,
                    view,
                    depth,
                    textures,
                    scene,
                    assets,
                    gfx.clear_color,
                );
            },
        )
    }

    /// `render` с готовыми моделями мира и без слоя.
    fn render_models(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        gfx: &Gfx,
        models: &ExpandedModels,
    ) -> Vec<u8> {
        let scene = gfx.scene.as_ref().unwrap();
        frame(
            device,
            queue,
            assets,
            gfx,
            |p, encoder, view, depth, textures, _| {
                p.pass(
                    WORLD,
                    encoder,
                    view,
                    depth,
                    textures,
                    scene,
                    &scene.camera,
                    &scene.meshes,
                    models,
                    Some(gfx.clear_color),
                );
            },
        )
    }

    /// 3D от `draw` на свежем конвейере, затем спрайты `gfx` с `Load`.
    fn frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        gfx: &Gfx,
        draw: impl FnOnce(
            &mut MeshPipeline,
            &mut wgpu::CommandEncoder,
            &wgpu::TextureView,
            &wgpu::Texture,
            &GpuTextures,
            &Assets,
        ),
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
            draw(&mut meshes, encoder, view, &depth, &textures, assets);
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
    fn unlit_color_is_ambient_texture_tint_and_sprites_stay_on_top() {
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
        let tint = Color {
            r: 0.5,
            g: 1.0,
            b: 0.25,
            a: 1.0,
        };
        // Заполнитель — за начальной ёмкостью и в том же батче, но белый; видимый куб, вокруг которого камера, — последний.
        let filler = 11;
        assert!((filler * size_of::<MeshInstance>()) as u64 > INITIAL_CAPACITY);
        let mut meshes = vec![at(cube, tex, -50.0, 0.1); filler];
        meshes.push(MeshDraw {
            color: tint,
            ..at(cube, tex, 0.0, 10.0)
        });
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
        let expected = [
            lit(200, ambient.r * tint.r),
            lit(100, ambient.g * tint.g),
            lit(50, ambient.b * tint.b),
        ];
        assert_pixel(&img, "ambient × texture × tint", (40, 40), expected);
        assert_pixel(&img, "sprite over 3d", (8, 8), [255, 255, 255]);
        assert_pixel(&img, "3d next to sprite", (16, 8), expected);
    }

    /// Центр кадра — передняя грань куба на `z = -3`, в 4 м от камеры на `z = 1`.
    /// `test` — имя временного PNG: у параллельных тестов оно разное.
    fn render_face(
        test: &str,
        ambient: f64,
        sun: Option<DirectionalLight>,
        lights: Vec<PointLight>,
        fog: Fog,
    ) -> Vec<u8> {
        let (device, queue) = gpu().unwrap();
        let mut assets = Assets::default();
        let cube = cube(&mut assets);
        let [r, g, b] = TEX;
        let tex = load_png(&mut assets, test, 1, 1, &[r, g, b, 255]);
        let scene = Scene {
            ambient: gray(ambient),
            fog,
            sun,
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
            None,
            vec![light(0.0, -1.0, Color::WHITE, 4.0)],
            Fog::default(),
        );
        assert_pixel(&toward, "face toward light", CENTER, lit(0.45));
        let away = render_face(
            "mesh-falloff",
            0.2,
            None,
            vec![light(0.0, -6.0, Color::WHITE, 10.0)],
            Fog::default(),
        );
        assert_pixel(&away, "face away from light", CENTER, lit(0.2));
        let beyond = render_face(
            "mesh-falloff",
            0.2,
            None,
            vec![light(0.0, -1.0, Color::WHITE, 0.5)],
            Fog::default(),
        );
        assert_pixel(&beyond, "beyond range", CENTER, lit(0.2));
    }

    #[test]
    fn sun_lambert_without_falloff() {
        if gpu().is_none() {
            return;
        }
        let sun = |dir| {
            let sun = DirectionalLight {
                dir,
                color: gray(0.5),
            };
            render_face("mesh-sun", 0.2, Some(sun), Vec::new(), Fog::default())
        };
        // Грань смотрит на +Z, лучи под 45° к ней: 0,5·cos 45° поверх ambient 0,2.
        let toward = sun(vec3(0.0, -1.0, -1.0));
        assert_pixel(
            &toward,
            "face toward sun",
            CENTER,
            lit(0.2 + 0.5 * FRAC_1_SQRT_2),
        );
        assert_pixel(&sun(Vec3::Z), "face away from sun", CENTER, lit(0.2));
        let none = render_face("mesh-sun", 0.2, None, Vec::new(), Fog::default());
        assert_pixel(&none, "no sun", CENTER, lit(0.2));
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
            None,
            with_white_at(MAX_LIGHTS - 1),
            Fog::default(),
        );
        assert_pixel(&sixteenth, "16th light", CENTER, lit(0.45));
        let seventeenth = render_face(
            "mesh-max-lights",
            0.2,
            None,
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
            render_face("mesh-fog", 1.0, None, Vec::new(), fog)
        };
        assert_pixel(&fogged(5.0, 10.0), "before start", CENTER, lit(1.0));
        let fog_rgb = [linear_to_srgb(0.5); 3];
        assert_pixel(&fogged(1.0, 3.0), "past end", CENTER, fog_rgb);
        // Расстояние 4 от камеры, а не 3 от начала координат: туман ровно наполовину.
        let half = TEX.map(|c| linear_to_srgb(0.5 * srgb_to_linear(c) + 0.25));
        assert_pixel(&fogged(2.0, 6.0), "halfway", CENTER, half);
    }

    fn skinned(mesh: Handle<Mesh>, texture: Handle<Texture>, palette: usize) -> SkinnedDraw {
        SkinnedDraw {
            mesh,
            texture,
            color: Color::WHITE,
            transform: Mat4::from_translation(vec3(0.0, 0.0, -3.0)),
            palette,
        }
    }

    fn lit_scene() -> Gfx {
        gfx(Scene {
            ambient: Color::WHITE,
            ..Default::default()
        })
    }

    #[test]
    fn skinned_vertex_blends_joint_matrices() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut assets = Assets::default();
        let cube = skinned_cube(&mut assets, [1, 2, 0, 0], [0.5, 0.5, 0.0, 0.0]);
        let red = load_png(&mut assets, "skin-blend", 1, 1, &[255, 0, 0, 255]);
        let models = ExpandedModels {
            skinned: vec![skinned(cube, red, 0)],
            palettes: vec![vec![
                Mat4::from_translation(vec3(-100.0, 0.0, 0.0)),
                Mat4::from_translation(vec3(2.0, 0.0, 0.0)),
                Mat4::IDENTITY,
            ]],
            ..Default::default()
        };
        let img = render_models(&device, &queue, &mut assets, &lit_scene(), &models);
        // Полусумма сдвигов 2 и 0 — куб на x = 1: правее центра на 18 пикселей.
        assert_pixel(&img, "moved by blend", (50, 32), [255, 0, 0]);
        assert_pixel(&img, "left in place", CENTER, BLUE);
    }

    #[test]
    fn each_skinned_instance_uses_its_palette() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut assets = Assets::default();
        let cube = skinned_cube(&mut assets, [0; 4], [1.0, 0.0, 0.0, 0.0]);
        let red = load_png(&mut assets, "skin-two-red", 1, 1, &[255, 0, 0, 255]);
        let green = load_png(&mut assets, "skin-two-green", 1, 1, &[0, 255, 0, 255]);
        let models = ExpandedModels {
            skinned: vec![skinned(cube, red, 0), skinned(cube, green, 1)],
            palettes: vec![
                vec![Mat4::from_translation(vec3(-1.0, 0.0, 0.0))],
                vec![Mat4::from_translation(vec3(1.0, 0.0, 0.0))],
            ],
            ..Default::default()
        };
        let img = render_models(&device, &queue, &mut assets, &lit_scene(), &models);
        assert_pixel(&img, "first palette", (14, 32), [255, 0, 0]);
        assert_pixel(&img, "second palette", (50, 32), [0, 255, 0]);
        assert_pixel(&img, "between", CENTER, BLUE);
    }

    #[test]
    fn skinned_instances_follow_rigid_ones() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut assets = Assets::default();
        let rigid = cube(&mut assets);
        let skin = skinned_cube(&mut assets, [0; 4], [1.0, 0.0, 0.0, 0.0]);
        let red = load_png(&mut assets, "skin-after-red", 1, 1, &[255, 0, 0, 255]);
        let green = load_png(&mut assets, "skin-after-green", 1, 1, &[0, 255, 0, 255]);
        let white = load_png(&mut assets, "skin-after-white", 1, 1, &[255; 4]);
        let yellow = Color {
            b: 0.0,
            ..Color::WHITE
        };
        let x = |x| Mat4::from_translation(vec3(x, 0.0, -3.0));
        let mut gfx = lit_scene();
        gfx.scene.as_mut().unwrap().meshes = vec![MeshDraw {
            transform: x(-1.0),
            ..at(rigid, red, 0.0, 1.0)
        }];
        let models = ExpandedModels {
            skinned: [(green, Color::WHITE, 0.0), (white, yellow, 1.0)]
                .map(|(texture, color, at)| SkinnedDraw {
                    color,
                    transform: x(at),
                    ..skinned(skin, texture, 0)
                })
                .into(),
            palettes: vec![vec![Mat4::IDENTITY]],
            ..Default::default()
        };
        let img = render_models(&device, &queue, &mut assets, &gfx, &models);
        assert_pixel(&img, "rigid", (14, 32), [255, 0, 0]);
        assert_pixel(&img, "first skinned", CENTER, [0, 255, 0]);
        assert_pixel(&img, "second skinned, tinted", (50, 32), [255, 255, 0]);
    }

    #[test]
    fn skinned_normal_turns_with_joint() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut assets = Assets::default();
        let cube = skinned_cube(&mut assets, [0; 4], [1.0, 0.0, 0.0, 0.0]);
        let [r, g, b] = TEX;
        let tex = load_png(&mut assets, "skin-normal", 1, 1, &[r, g, b, 255]);
        let gfx = gfx(Scene {
            ambient: gray(0.2),
            lights: vec![light(0.0, -1.0, Color::WHITE, 4.0)],
            camera: Camera3D {
                pos: vec3(0.0, 0.0, 1.0),
                ..Default::default()
            },
            ..Default::default()
        });
        // К камере повёрнута грань с нормалью −Z: без поворота нормали она осталась бы в тени.
        let models = ExpandedModels {
            skinned: vec![SkinnedDraw {
                transform: Mat4::from_translation(vec3(0.0, 0.0, -4.0))
                    * Mat4::from_scale(Vec3::splat(2.0)),
                ..skinned(cube, tex, 0)
            }],
            palettes: vec![vec![Mat4::from_rotation_y(std::f32::consts::PI)]],
            ..Default::default()
        };
        let img = render_models(&device, &queue, &mut assets, &gfx, &models);
        assert_pixel(&img, "face toward light", CENTER, lit(0.45));
    }

    #[test]
    fn max_joints_drawn_one_more_skipped() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        #[expect(clippy::cast_possible_truncation, reason = "63")]
        let last = (MAX_JOINTS - 1) as u16;
        let mut assets = Assets::default();
        let cube = skinned_cube(&mut assets, [last, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let red = load_png(&mut assets, "skin-max", 1, 1, &[255, 0, 0, 255]);
        let with_joints = |n| {
            let mut palette = vec![Mat4::ZERO; n];
            palette[MAX_JOINTS - 1] = Mat4::IDENTITY;
            ExpandedModels {
                skinned: vec![skinned(cube, red, 0)],
                palettes: vec![palette],
                ..Default::default()
            }
        };
        let full = with_joints(MAX_JOINTS);
        let img = render_models(&device, &queue, &mut assets, &lit_scene(), &full);
        assert_pixel(&img, "64 joints", CENTER, [255, 0, 0]);
        let over = with_joints(MAX_JOINTS + 1);
        let img = render_models(&device, &queue, &mut assets, &lit_scene(), &over);
        assert_pixel(&img, "65 joints", CENTER, BLUE);
    }

    #[test]
    fn translucent_is_dithered_without_depth() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        for skin in [false, true] {
            for (a, kept) in [(0.5, 8), (1.0, 16)] {
                let mut assets = Assets::default();
                let rigid = cube(&mut assets);
                let skinned_mesh = skinned_cube(&mut assets, [0; 4], [1.0, 0.0, 0.0, 0.0]);
                // `red` загружен первым: батч ближнего куба рисуется раньше дальнего.
                let red = load_png(
                    &mut assets,
                    &format!("dither-red-{skin}-{a}"),
                    1,
                    1,
                    &[255, 0, 0, 255],
                );
                let green = load_png(
                    &mut assets,
                    &format!("dither-green-{skin}-{a}"),
                    1,
                    1,
                    &[0, 255, 0, 255],
                );
                let color = Color { a, ..Color::WHITE };
                let mut gfx = lit_scene();
                let scene = gfx.scene.as_mut().unwrap();
                scene.meshes.push(at(rigid, green, -10.0, 8.0));
                let mut models = ExpandedModels::default();
                if skin {
                    models.skinned.push(SkinnedDraw {
                        color,
                        ..skinned(skinned_mesh, red, 0)
                    });
                    models.palettes.push(vec![Mat4::IDENTITY]);
                } else {
                    scene.meshes.push(MeshDraw {
                        color,
                        ..at(rigid, red, -3.0, 1.0)
                    });
                }
                let img = render_models(&device, &queue, &mut assets, &gfx, &models);
                // Блок 4×4, выровненный по сетке Байера: строки кадра 28..32 — это `y` 32..36 у `pixel`.
                let block: Vec<_> = (28..32)
                    .flat_map(|x| (32..36).map(move |y| (x, y)))
                    .map(|(x, y)| crate::gpu_test::pixel(&img, x, y))
                    .collect();
                let red_n = block.iter().filter(|&&p| p == [255, 0, 0]).count();
                let green_n = block.iter().filter(|&&p| p == [0, 255, 0]).count();
                assert_eq!(
                    (red_n, green_n),
                    (kept, 16 - kept),
                    "skin {skin}, a {a}: {block:?}"
                );
            }
        }
    }

    #[test]
    fn model_parts_are_drawn() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        for (file, z) in [("gate.glb", -3.0), ("character-orc.glb", -2.0)] {
            let mut assets = Assets::default();
            let model = assets.load_model(crate::model::tests::testdata(file));
            let mut gfx = gfx(Scene {
                ambient: Color::WHITE,
                camera: Camera3D {
                    pos: vec3(0.0, 0.4, 0.0),
                    ..Default::default()
                },
                models: vec![crate::ModelDraw {
                    model,
                    transform: Mat4::from_translation(vec3(0.0, 0.0, z)),
                    pose: None,
                    tint: Color::WHITE,
                }],
                ..Default::default()
            });
            gfx.clear_color = Color::BLACK;
            let img = render(&device, &queue, &mut assets, &gfx);
            assert_ne!(crate::gpu_test::pixel(&img, 32, 32), [0, 0, 0], "{file}");
        }
    }
    /// Красная стена с передней гранью в 1 м перед камерой на высоте 0,4 и орк `tint` за ней в 2 м — в слое `viewmodel`.
    fn wall_and_orc(
        assets: &mut Assets,
        test: &str,
        tint: Color,
        viewmodel: impl FnOnce(Vec<crate::ModelDraw>) -> Option<Viewmodel>,
    ) -> Gfx {
        let cube = cube(assets);
        let red = load_png(assets, test, 1, 1, &[255, 0, 0, 255]);
        let orc = assets.load_model(crate::model::tests::testdata("character-orc.glb"));
        let models = vec![crate::ModelDraw {
            model: orc,
            transform: Mat4::from_translation(vec3(0.0, 0.0, -2.0)),
            pose: None,
            tint,
        }];
        gfx(Scene {
            ambient: Color::WHITE,
            camera: Camera3D {
                pos: vec3(0.0, 0.4, 0.0),
                ..Default::default()
            },
            meshes: vec![MeshDraw {
                transform: Mat4::from_translation(vec3(0.0, 0.4, -1.5)),
                ..at(cube, red, 0.0, 1.0)
            }],
            viewmodel: viewmodel(models),
            ..Default::default()
        })
    }

    fn layer(fov_y: f32, near: f32) -> impl FnOnce(Vec<crate::ModelDraw>) -> Option<Viewmodel> {
        move |models| {
            Some(Viewmodel {
                fov_y,
                near,
                models,
            })
        }
    }

    #[test]
    fn viewmodel_behind_world_wall_is_drawn_over_it() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let fov = Camera3D::default().fov_y;
        for (near, expected) in [(0.05, [0, 0, 0]), (5.0, [255, 0, 0])] {
            let mut assets = Assets::default();
            let gfx = wall_and_orc(&mut assets, "vm-over-wall", Color::BLACK, layer(fov, near));
            let img = render(&device, &queue, &mut assets, &gfx);
            assert_pixel(&img, &format!("layer near {near}"), CENTER, expected);
        }
    }

    #[test]
    fn pixels_outside_viewmodel_match_frame_without_it() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let frame = |viewmodel: Box<dyn FnOnce(_) -> _>| {
            let mut assets = Assets::default();
            let gfx = wall_and_orc(&mut assets, "vm-outside", Color::BLACK, viewmodel);
            render(&device, &queue, &mut assets, &gfx)
        };
        let base = frame(Box::new(|_| None));
        let fov = Camera3D::default().fov_y;
        let mut covered = Vec::new();
        for fov_y in [fov, fov / 2.0] {
            let img = frame(Box::new(layer(fov_y, 0.05)));
            let (mut model, mut wall) = (0, 0);
            for (y, x) in (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (y, x))) {
                let (p, b) = (
                    crate::gpu_test::pixel(&img, x, y),
                    crate::gpu_test::pixel(&base, x, y),
                );
                if p == [0, 0, 0] {
                    model += 1;
                } else {
                    assert_eq!(p, b, "fov {fov_y}, pixel ({x}, {y})");
                    wall += usize::from(p == [255, 0, 0]);
                }
            }
            assert!(
                model > 0 && wall > 0,
                "fov {fov_y}: model {model}, wall {wall}"
            );
            covered.push(model);
        }
        assert!(
            covered[1] > covered[0],
            "narrower layer fov zooms: {covered:?}"
        );
    }

    #[test]
    fn scene_light_shines_on_viewmodel_and_empty_layer_changes_nothing() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let fov = Camera3D::default().fov_y;
        let lit_orc = |lights: Vec<PointLight>| {
            let mut assets = Assets::default();
            let mut gfx = wall_and_orc(&mut assets, "vm-light", Color::WHITE, layer(fov, 0.05));
            let scene = gfx.scene.as_mut().unwrap();
            scene.ambient = Color::BLACK;
            scene.lights = lights;
            render(&device, &queue, &mut assets, &gfx)
        };
        assert_pixel(&lit_orc(Vec::new()), "unlit", CENTER, [0, 0, 0]);
        let light = PointLight {
            pos: vec3(0.0, 0.4, -1.0),
            color: Color::WHITE,
            range: 3.0,
        };
        let lit = lit_orc(vec![light]);
        assert_ne!(crate::gpu_test::pixel(&lit, 32, 32), [0, 0, 0], "lit");

        let frame = |viewmodel: Box<dyn FnOnce(_) -> _>| {
            let mut assets = Assets::default();
            let gfx = wall_and_orc(&mut assets, "vm-empty", Color::BLACK, viewmodel);
            render(&device, &queue, &mut assets, &gfx)
        };
        let none = frame(Box::new(|_| None));
        let empty = frame(Box::new(move |_| layer(fov, 0.05)(Vec::new())));
        assert!(none == empty, "empty layer changed the frame");
    }

    #[test]
    fn viewmodel_skin_leaves_world_skin_palette() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let frame = |with_layer: bool| {
            let mut assets = Assets::default();
            let orc = assets.load_model(crate::model::tests::testdata("character-orc.glb"));
            let model = assets.model(orc);
            let mut collapsed = model.rest.clone();
            for node in 0..model.nodes.len() {
                collapsed.local_mut(node).scale = Vec3::ZERO;
            }
            model.recompose(&mut collapsed);
            let draw = |z: f32, pose| crate::ModelDraw {
                model: orc,
                transform: Mat4::from_translation(vec3(0.0, 0.0, z)),
                pose,
                tint: Color::BLACK,
            };
            // Сжатый в точку орк слоя позади камеры виден только через общую с миром палитру.
            let viewmodel = with_layer.then(|| Viewmodel {
                fov_y: Camera3D::default().fov_y,
                near: 0.05,
                models: vec![draw(2.0, Some(collapsed))],
            });
            let mut gfx = gfx(Scene {
                ambient: Color::WHITE,
                camera: Camera3D {
                    pos: vec3(0.0, 0.4, 0.0),
                    ..Default::default()
                },
                models: vec![draw(-2.0, None)],
                viewmodel,
                ..Default::default()
            });
            gfx.clear_color = Color::WHITE;
            render(&device, &queue, &mut assets, &gfx)
        };
        let base = frame(false);
        assert_pixel(&base, "world orc", CENTER, [0, 0, 0]);
        assert!(frame(true) == base, "layer skin changed the world orc");
    }
}
