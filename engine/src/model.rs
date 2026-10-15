use std::path::Path;

use glam::{Mat4, Quat, Vec3};
use gltf::animation::util::ReadOutputs;
use gltf::animation::{Interpolation, Property};
use gltf::mesh::Mode;
use gltf::{Gltf, Semantic};

use crate::assets::{Assets, Handle, Texture};
use crate::mesh::{Mesh, Vertex};

/// Модель glTF: узлы, скины и анимации; её меши и текстуры лежат в `Assets`.
#[derive(Debug, Default, PartialEq)]
pub struct Model {
    /// Узлы в порядке файла: на эти индексы ссылаются скины и каналы.
    pub nodes: Vec<Node>,
    /// Скины.
    pub skins: Vec<Skin>,
    /// Анимации.
    pub animations: Vec<Animation>,
    /// Поза из TRS узлов без анимации.
    pub rest: Pose,
    /// Индексы узлов, родитель раньше детей: glTF такого порядка не обещает.
    order: Vec<usize>,
}

/// Матрицы модели в одной позе; буфер для `Model::pose`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    /// Узел → модель по индексам `Model::nodes`.
    pub globals: Vec<Mat4>,
    /// По скину: `globals[сустав] × IBM` в порядке `Skin::joints`.
    pub palettes: Vec<Vec<Mat4>>,
    locals: Vec<(Vec3, Quat, Vec3)>,
}

/// Узел иерархии с локальным TRS.
#[derive(Debug, PartialEq)]
pub struct Node {
    /// Имя из файла, пустое без имени.
    pub name: String,
    /// Индекс родителя в `Model::nodes`.
    pub parent: Option<usize>,
    /// Локальный перенос.
    pub translation: Vec3,
    /// Локальный поворот.
    pub rotation: Quat,
    /// Локальный масштаб.
    pub scale: Vec3,
    /// Части меша узла.
    pub primitives: Vec<Primitive>,
    /// Индекс в `Model::skins`.
    pub skin: Option<usize>,
}

/// Часть меша со своей текстурой.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primitive {
    /// Вершины и индексы на GPU.
    pub mesh: Handle<Mesh>,
    /// Базовый цвет; без текстуры в файле — `Handle::WHITE`.
    pub texture: Handle<Texture>,
}

/// Скин: `Vertex::joints` индексирует `joints`, а не узлы модели.
#[derive(Debug, PartialEq)]
pub struct Skin {
    /// Узлы-суставы в порядке `skin.joints` файла.
    pub joints: Vec<usize>,
    /// Обратные bind-матрицы по одной на сустав.
    pub inverse_bind: Vec<Mat4>,
}

/// Анимация с линейной интерполяцией ключей.
#[derive(Debug, PartialEq)]
pub struct Animation {
    /// Имя из файла.
    pub name: String,
    /// Последний ключ среди каналов, с.
    pub duration: f32,
    /// Каналы.
    pub channels: Vec<Channel>,
}

/// Ключи одного свойства одного узла.
#[derive(Debug, PartialEq)]
pub struct Channel {
    /// Индекс в `Model::nodes`.
    pub node: usize,
    /// Времена ключей по возрастанию, с.
    pub times: Vec<f32>,
    /// Значения ключей, по одному на время.
    pub values: Values,
}

/// Значения канала по свойству.
#[derive(Debug, PartialEq)]
pub enum Values {
    /// Перенос.
    Translation(Vec<Vec3>),
    /// Поворот.
    Rotation(Vec<Quat>),
    /// Масштаб.
    Scale(Vec<Vec3>),
}

impl Model {
    fn new(nodes: Vec<Node>, skins: Vec<Skin>, animations: Vec<Animation>) -> Result<Self, String> {
        let mut depths = Vec::with_capacity(nodes.len());
        for n in &nodes {
            let mut depth = 0;
            let mut parent = n.parent;
            while let Some(p) = parent {
                depth += 1;
                if depth == nodes.len() {
                    return Err(format!("node {:?}: hierarchy has a cycle", n.name));
                }
                parent = nodes[p].parent;
            }
            depths.push(depth);
        }
        let mut order: Vec<usize> = (0..nodes.len()).collect();
        order.sort_by_key(|&i| depths[i]);
        let mut model = Self {
            nodes,
            skins,
            animations,
            rest: Pose::default(),
            order,
        };
        let mut rest = Pose::default();
        rest.locals.extend(model.nodes.iter().map(Node::trs));
        model.compose(&mut rest);
        model.rest = rest;
        Ok(model)
    }

    /// Поза анимации `anim` в момент `t` с: при `looped` — по модулю длины, иначе за краями — крайние ключи.
    ///
    /// # Panics
    /// Если `anim` вне `animations`.
    pub fn pose(&self, anim: usize, t: f32, looped: bool, pose: &mut Pose) {
        let a = &self.animations[anim];
        // При нулевой длине — NaN, и `sample` отдаёт первый ключ.
        let t = if looped { t.rem_euclid(a.duration) } else { t };
        pose.locals.clear();
        pose.locals.extend(self.nodes.iter().map(Node::trs));
        for c in &a.channels {
            let (translation, rotation, scale) = &mut pose.locals[c.node];
            match &c.values {
                Values::Translation(v) => {
                    *translation = sample(&c.times, v, t, Vec3::lerp).unwrap_or(*translation);
                }
                Values::Rotation(v) => {
                    *rotation = sample(&c.times, v, t, Quat::slerp).unwrap_or(*rotation);
                }
                Values::Scale(v) => *scale = sample(&c.times, v, t, Vec3::lerp).unwrap_or(*scale),
            }
        }
        self.compose(pose);
    }

    /// Глобальные матрицы и палитры из `pose.locals`.
    fn compose(&self, pose: &mut Pose) {
        pose.globals.clear();
        pose.globals.extend(
            pose.locals
                .iter()
                .map(|&(t, r, s)| Mat4::from_scale_rotation_translation(s, r, t)),
        );
        for &i in &self.order {
            if let Some(p) = self.nodes[i].parent {
                pose.globals[i] = pose.globals[p] * pose.globals[i];
            }
        }
        pose.palettes.resize_with(self.skins.len(), Vec::new);
        for (palette, skin) in pose.palettes.iter_mut().zip(&self.skins) {
            palette.clear();
            palette.extend(
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(&j, ibm)| pose.globals[j] * *ibm),
            );
        }
    }
}

impl Node {
    fn trs(&self) -> (Vec3, Quat, Vec3) {
        (self.translation, self.rotation, self.scale)
    }
}

/// Значение в момент `t` между соседними ключами; до первого и после последнего — крайний ключ.
fn sample<T: Copy>(times: &[f32], values: &[T], t: f32, mix: fn(T, T, f32) -> T) -> Option<T> {
    let i = times.partition_point(|&k| k <= t);
    match (i.checked_sub(1).and_then(|p| values.get(p)), values.get(i)) {
        (Some(&a), Some(&b)) => Some(mix(a, b, (t - times[i - 1]) / (times[i] - times[i - 1]))),
        (a, b) => a.or(b).copied(),
    }
}

impl Values {
    fn len(&self) -> usize {
        match self {
            Self::Translation(v) | Self::Scale(v) => v.len(),
            Self::Rotation(v) => v.len(),
        }
    }
}

/// Разобранный файл до заливки: ошибка разбора не должна оставить в `Assets` полмодели.
pub(crate) struct Parsed {
    model: Model,
    node_meshes: Vec<Option<usize>>,
    meshes: Vec<Vec<RawPrimitive>>,
}

struct RawPrimitive {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    image: Option<String>,
}

/// GLB: буфер — только бинарный блок файла, внешние и `data:` буферы — ошибка.
pub(crate) fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    let gltf = Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
    if gltf
        .buffers()
        .any(|b| matches!(b.source(), gltf::buffer::Source::Uri(_)))
    {
        return Err("external buffers are not supported".into());
    }
    let blob = gltf.blob.as_deref();
    let meshes = gltf
        .meshes()
        .map(|m| {
            m.primitives()
                .map(|p| read_primitive(&p, blob))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut parents = vec![None; gltf.nodes().len()];
    for n in gltf.nodes() {
        for c in n.children() {
            parents[c.index()] = Some(n.index());
        }
    }
    let mut nodes = Vec::new();
    let mut node_meshes = Vec::new();
    for (n, parent) in gltf.nodes().zip(parents) {
        if let (Some(_), Some(m)) = (n.skin(), n.mesh())
            && m.primitives().any(|p| {
                p.get(&Semantic::Joints(0)).is_none() || p.get(&Semantic::Weights(0)).is_none()
            })
        {
            return Err(format!(
                "skinned mesh {:?} without JOINTS_0/WEIGHTS_0",
                m.name()
            ));
        }
        let (t, r, s) = n.transform().decomposed();
        nodes.push(Node {
            name: n.name().unwrap_or_default().to_owned(),
            parent,
            translation: t.into(),
            rotation: Quat::from_array(r),
            scale: s.into(),
            primitives: Vec::new(),
            skin: n.skin().map(|s| s.index()),
        });
        node_meshes.push(n.mesh().map(|m| m.index()));
    }

    let skins = gltf
        .skins()
        .map(|s| {
            let joints: Vec<usize> = s.joints().map(|j| j.index()).collect();
            let inverse_bind: Vec<Mat4> = match s.reader(|_| blob).read_inverse_bind_matrices() {
                Some(m) => m.map(|c| Mat4::from_cols_array_2d(&c)).collect(),
                None => vec![Mat4::IDENTITY; joints.len()],
            };
            if inverse_bind.len() != joints.len() {
                return Err(format!(
                    "skin: {} inverse bind matrices for {} joints",
                    inverse_bind.len(),
                    joints.len()
                ));
            }
            Ok(Skin {
                joints,
                inverse_bind,
            })
        })
        .collect::<Result<_, String>>()?;

    let animations = gltf
        .animations()
        .map(|a| read_animation(&a, blob))
        .collect::<Result<_, _>>()?;

    Ok(Parsed {
        model: Model::new(nodes, skins, animations)?,
        node_meshes,
        meshes,
    })
}

fn read_primitive(p: &gltf::Primitive<'_>, blob: Option<&[u8]>) -> Result<RawPrimitive, String> {
    if p.mode() != Mode::Triangles {
        return Err(format!("primitive mode {:?} is not supported", p.mode()));
    }
    let reader = p.reader(|_| blob);
    let (Some(positions), Some(normals)) = (reader.read_positions(), reader.read_normals()) else {
        return Err("primitive without POSITION/NORMAL".into());
    };
    let normals: Vec<[f32; 3]> = normals.collect();
    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|t| t.into_f32().collect())
        .unwrap_or_default();
    let joints: Vec<[u16; 4]> = reader
        .read_joints(0)
        .map(|j| j.into_u16().collect())
        .unwrap_or_default();
    let weights: Vec<[f32; 4]> = reader
        .read_weights(0)
        .map(|w| w.into_f32().collect())
        .unwrap_or_default();
    let vertices: Vec<Vertex> = positions
        .enumerate()
        .map(|(i, pos)| Vertex {
            pos,
            normal: normals.get(i).copied().unwrap_or_default(),
            uv: uvs.get(i).copied().unwrap_or_default(),
            joints: joints.get(i).copied().unwrap_or_default(),
            weights: weights.get(i).copied().unwrap_or_default(),
        })
        .collect();
    let indices = match reader.read_indices() {
        Some(i) => i.into_u32().collect(),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "в glTF число вершин accessor — u32"
        )]
        None => (0..vertices.len() as u32).collect(),
    };
    let image = match p.material().pbr_metallic_roughness().base_color_texture() {
        None => None,
        Some(info) => match info.texture().source().source() {
            gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => {
                Some(uri.to_owned())
            }
            _ => return Err("embedded images are not supported".into()),
        },
    };
    Ok(RawPrimitive {
        vertices,
        indices,
        image,
    })
}

fn read_animation(a: &gltf::Animation<'_>, blob: Option<&[u8]>) -> Result<Animation, String> {
    let name = a.name().unwrap_or_default().to_owned();
    let mut channels = Vec::new();
    for c in a.channels() {
        let interpolation = c.sampler().interpolation();
        if interpolation != Interpolation::Linear {
            return Err(format!(
                "animation {name}: {interpolation:?} interpolation is not supported"
            ));
        }
        // Чтение весов морфинга из accessor не того типа паникует внутри gltf.
        if c.target().property() == Property::MorphTargetWeights {
            return Err(format!(
                "animation {name}: only T/R/S channels are supported"
            ));
        }
        let reader = c.reader(|_| blob);
        let values = match reader.read_outputs() {
            Some(ReadOutputs::Translations(t)) => Values::Translation(t.map(Vec3::from).collect()),
            Some(ReadOutputs::Rotations(r)) => {
                Values::Rotation(r.into_f32().map(Quat::from_array).collect())
            }
            Some(ReadOutputs::Scales(s)) => Values::Scale(s.map(Vec3::from).collect()),
            _ => return Err(format!("animation {name}: channel without data")),
        };
        let times: Vec<f32> = reader.read_inputs().into_iter().flatten().collect();
        if times.len() != values.len() {
            return Err(format!(
                "animation {name}: {} keys for {} values",
                times.len(),
                values.len()
            ));
        }
        channels.push(Channel {
            node: c.target().node().index(),
            times,
            values,
        });
    }
    let duration = channels
        .iter()
        .filter_map(|c| c.times.last().copied())
        .fold(0.0, f32::max);
    Ok(Animation {
        name,
        duration,
        channels,
    })
}

impl Parsed {
    /// Заливает меши и грузит текстуры по URI относительно `dir`.
    pub(crate) fn upload(self, assets: &mut Assets, dir: &Path) -> Model {
        let meshes: Vec<Vec<Primitive>> = self
            .meshes
            .into_iter()
            .map(|m| {
                m.into_iter()
                    .map(|p| Primitive {
                        mesh: assets.add_mesh(p.vertices, p.indices),
                        texture: p
                            .image
                            .map_or(Handle::WHITE, |uri| assets.load_texture(dir.join(uri))),
                    })
                    .collect()
            })
            .collect();
        let mut model = self.model;
        for (node, mesh) in model.nodes.iter_mut().zip(self.node_meshes) {
            if let Some(m) = mesh {
                node.primitives.clone_from(&meshes[m]);
            }
        }
        model
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;
    use std::path::PathBuf;

    pub(crate) fn testdata(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/testdata")
            .join(name)
    }

    /// Модель без заливки: меши и текстуры проверяются через `Assets`.
    fn load(name: &str) -> Model {
        parse(&std::fs::read(testdata(name)).unwrap())
            .unwrap()
            .model
    }

    /// Файл из testdata с первой заменой `from` на `to` в JSON-чанке для каждой пары.
    pub(crate) fn patched(name: &str, edits: &[(&str, &str)]) -> Vec<u8> {
        let bytes = std::fs::read(testdata(name)).unwrap();
        let mut glb = gltf::Glb::from_slice(&bytes).unwrap();
        let mut json = String::from_utf8(glb.json.to_vec()).unwrap();
        for (from, to) in edits {
            assert!(json.contains(from), "{from}");
            json = json.replacen(from, to, 1);
        }
        glb.json = json.into_bytes().into();
        glb.to_vec().unwrap()
    }

    fn parse_err(name: &str, from: &str, to: &str) -> String {
        match parse(&patched(name, &[(from, to)])) {
            Ok(_) => panic!("{from} → {to} parsed"),
            Err(e) => e,
        }
    }

    fn names(model: &Model, idx: impl IntoIterator<Item = usize>) -> Vec<&str> {
        idx.into_iter()
            .map(|i| model.nodes[i].name.as_str())
            .collect()
    }

    fn kinds(a: &Animation) -> Vec<&str> {
        a.channels
            .iter()
            .map(|c| match c.values {
                Values::Translation(_) => "T",
                Values::Rotation(_) => "R",
                Values::Scale(_) => "S",
            })
            .collect()
    }

    fn node(parent: Option<usize>, translation: Vec3) -> Node {
        Node {
            name: String::new(),
            parent,
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            primitives: Vec::new(),
            skin: None,
        }
    }

    fn animation(channels: Vec<Channel>) -> Animation {
        Animation {
            name: String::new(),
            duration: channels
                .iter()
                .filter_map(|c| c.times.last().copied())
                .fold(0.0, f32::max),
            channels,
        }
    }

    /// Узел 0 едет по X от 0 до 4 и растёт от 1 до 3 за ключи 1..3 с; узел 1 стоит в (0, 5, 0).
    fn slider() -> Model {
        let anim = animation(vec![
            Channel {
                node: 0,
                times: vec![1.0, 3.0],
                values: Values::Translation(vec![Vec3::ZERO, Vec3::X * 4.0]),
            },
            Channel {
                node: 0,
                times: vec![1.0, 3.0],
                values: Values::Scale(vec![Vec3::ONE, Vec3::splat(3.0)]),
            },
        ]);
        Model::new(
            vec![node(None, Vec3::ZERO), node(None, Vec3::Y * 5.0)],
            Vec::new(),
            vec![anim],
        )
        .unwrap()
    }

    fn x_and_scale(model: &Model, t: f32, looped: bool) -> (f32, f32) {
        let mut pose = Pose::default();
        model.pose(0, t, looped, &mut pose);
        (pose.globals[0].w_axis.x, pose.globals[0].x_axis.x)
    }

    #[test]
    fn lerps_between_keys_and_holds_edge_keys() {
        let model = slider();
        assert_eq!(x_and_scale(&model, 2.0, false), (2.0, 2.0));
        assert_eq!(x_and_scale(&model, 2.5, false), (3.0, 2.5));
        assert_eq!(x_and_scale(&model, 0.0, false), (0.0, 1.0));
        assert_eq!(x_and_scale(&model, 9.0, false), (4.0, 3.0));
    }

    #[test]
    fn node_without_channels_keeps_its_trs() {
        let mut pose = Pose::default();
        slider().pose(0, 2.0, false, &mut pose);
        assert_eq!(pose.globals[1], Mat4::from_translation(Vec3::Y * 5.0));
    }

    #[test]
    fn looped_time_wraps_by_duration() {
        let model = slider();
        assert_eq!(x_and_scale(&model, 5.0, true), (2.0, 2.0));
        assert_eq!(x_and_scale(&model, -1.0, true), (2.0, 2.0));
        assert_eq!(x_and_scale(&model, 3.0, true), (0.0, 1.0));

        let still = animation(vec![Channel {
            node: 0,
            times: vec![0.0],
            values: Values::Translation(vec![Vec3::X * 7.0]),
        }]);
        let model = Model::new(vec![node(None, Vec3::ZERO)], Vec::new(), vec![still]).unwrap();
        assert_eq!(x_and_scale(&model, 1.3, true), (7.0, 1.0));
    }

    #[test]
    fn rotation_takes_short_arc() {
        let turn = animation(vec![Channel {
            node: 0,
            times: vec![0.0, 1.0],
            values: Values::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_y(3.0 * FRAC_PI_2)]),
        }]);
        let model = Model::new(vec![node(None, Vec3::ZERO)], Vec::new(), vec![turn]).unwrap();
        let mut pose = Pose::default();
        model.pose(0, 0.5, false, &mut pose);
        let z = pose.globals[0].transform_vector3(Vec3::Z);
        let expected = Quat::from_rotation_y(-FRAC_PI_2 / 2.0) * Vec3::Z;
        assert!(z.abs_diff_eq(expected, 1e-5), "{z}");
    }

    #[test]
    fn child_global_is_parent_times_local_in_any_file_order() {
        let mut root = node(None, Vec3::X);
        root.rotation = Quat::from_rotation_y(FRAC_PI_2);
        let model = Model::new(
            vec![node(Some(1), Vec3::Z), node(Some(2), Vec3::Y), root],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let g = &model.rest.globals;
        let local = |i: usize| Mat4::from_translation(model.nodes[i].translation);
        assert!(g[1].abs_diff_eq(g[2] * local(1), 1e-6));
        assert!(g[0].abs_diff_eq(g[1] * local(0), 1e-6));
        assert!(
            g[0].w_axis
                .abs_diff_eq(glam::vec4(2.0, 1.0, 0.0, 1.0), 1e-6),
            "{}",
            g[0]
        );
    }

    #[test]
    fn palette_is_joint_global_times_ibm_in_skin_joints_order() {
        let ibm = [
            Mat4::from_scale(Vec3::splat(2.0)),
            Mat4::from_rotation_x(1.0),
        ];
        let model = Model::new(
            vec![
                node(None, Vec3::X),
                node(None, Vec3::Y),
                node(None, Vec3::Z),
            ],
            vec![
                Skin {
                    joints: vec![2, 0],
                    inverse_bind: ibm.to_vec(),
                },
                Skin {
                    joints: vec![1],
                    inverse_bind: vec![Mat4::IDENTITY],
                },
            ],
            Vec::new(),
        )
        .unwrap();
        let g = &model.rest.globals;
        assert_eq!(
            model.rest.palettes,
            [vec![g[2] * ibm[0], g[0] * ibm[1]], vec![g[1]]]
        );
    }

    #[test]
    fn orc_rest_palettes_are_identity() {
        let orc = load("character-orc.glb");
        assert_eq!(orc.rest.palettes.len(), 2);
        for m in orc.rest.palettes.iter().flatten() {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5), "{m}");
        }
        let mut walk = Pose::default();
        let anim = orc.animations.iter().position(|a| a.name == "walk");
        orc.pose(anim.unwrap(), 0.2, true, &mut walk);
        assert!(!walk.palettes[0][1].abs_diff_eq(Mat4::IDENTITY, 1e-3));
    }

    #[test]
    fn gate_has_door_mesh_with_widened_indices() {
        let mut assets = Assets::default();
        let h = assets.load_model(testdata("gate.glb"));
        let meshes = assets.take_pending_meshes();
        let colormap = assets.load_texture(testdata("Textures/colormap.png"));
        let gate = assets.model(h);
        assert_eq!(names(gate, 0..2), ["gate", "door"]);
        assert_eq!(gate.nodes[0].parent, None);
        assert_eq!(gate.nodes[1].parent, Some(0));
        assert_eq!(gate.nodes[0].primitives, []);
        assert_eq!(gate.nodes[1].skin, None);
        let [door] = gate.nodes[1].primitives[..] else {
            panic!("{:?}", gate.nodes[1].primitives)
        };
        assert_ne!(colormap, Handle::WHITE);
        assert_eq!(door.texture, colormap);

        let [mesh] = &meshes[..] else {
            panic!("одна часть — один меш")
        };
        assert_eq!(mesh.handle, door.mesh);
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), (132, 228));
        assert_eq!(mesh.indices[..6], [2, 1, 0, 1, 2, 3]);
        assert_eq!(mesh.indices.iter().max(), Some(&131));
        let v = mesh.vertices[0];
        assert!((v.normal[0].hypot(v.normal[1]).hypot(v.normal[2]) - 1.0).abs() < 1e-4);
        assert_ne!(v.uv, [0.0; 2]);
    }

    #[test]
    fn gate_animations_are_one_second_trs() {
        let gate = load("gate.glb");
        let anims: Vec<_> = gate.animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(anims, ["open", "close", "open-close"]);
        for a in &gate.animations[..2] {
            assert_eq!(a.duration, 1.0);
            assert_eq!(kinds(a), ["T", "R", "S"]);
            assert!(a.channels.iter().all(|c| c.node == 1));
        }
        let open = &gate.animations[0];
        assert_eq!(open.channels[0].times.len(), 30);
        let Values::Translation(t) = &open.channels[0].values else {
            unreachable!()
        };
        assert!((t[29].y - t[0].y - 0.75).abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn orc_hierarchy_and_skins() {
        let mut assets = Assets::default();
        let h = assets.load_model(testdata("character-orc.glb"));
        let meshes = assets.take_pending_meshes();
        let orc = assets.model(h);
        let parents: Vec<_> = orc.nodes.iter().map(|n| n.parent).collect();
        assert_eq!(
            parents,
            [
                None,
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(4),
                Some(4),
                Some(4),
                Some(0),
                Some(0)
            ]
        );
        let [body, head] = &orc.skins[..] else {
            panic!("2 скина")
        };
        for skin in [body, head] {
            assert_eq!(
                names(orc, skin.joints.iter().copied()),
                [
                    "root",
                    "leg-left",
                    "leg-right",
                    "torso",
                    "arm-left",
                    "arm-right",
                    "head"
                ]
            );
            assert_eq!(skin.inverse_bind.len(), 7);
        }
        assert_ne!(body.inverse_bind[4], Mat4::IDENTITY);
        assert_eq!(
            orc.nodes[4].translation,
            Vec3::new(-1.804_778_6e-15, 0.17625, -0.02375)
        );
        assert_eq!(orc.nodes[8].skin, Some(0));
        assert_eq!(orc.nodes[9].skin, Some(1));

        assert_eq!(meshes.len(), 2);
        for m in &meshes {
            assert!(m.vertices.iter().all(|v| v.joints.iter().all(|&j| j < 7)));
            assert!(
                m.vertices
                    .iter()
                    .all(|v| (v.weights.iter().sum::<f32>() - 1.0).abs() < 1e-3)
            );
        }
        assert!(meshes[0].vertices.iter().any(|v| v.joints[0] != 0));

        let walk = orc.animations.iter().find(|a| a.name == "walk").unwrap();
        assert!((walk.duration - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(walk.channels.len(), 7);
    }

    #[test]
    fn reload_gives_same_handle_without_new_meshes() {
        let mut assets = Assets::default();
        let a = assets.load_model(testdata("gate.glb"));
        assert_eq!(assets.take_pending_meshes().len(), 1);
        let b = assets.load_model(testdata("character-orc.glb"));
        assert_ne!(a, b);
        assert_eq!(assets.load_model(testdata("gate.glb")), a);
        assert_eq!(assets.take_pending_meshes().len(), 2);
        assert_eq!(assets.model(a).nodes.len(), 2);
    }

    #[test]
    fn broken_or_missing_file_gives_empty_model() {
        let path = std::env::temp_dir().join(format!("engine-model-{}.glb", std::process::id()));
        std::fs::write(&path, b"glTF garbage").unwrap();
        let mut assets = Assets::default();
        let broken = assets.load_model(&path);
        std::fs::remove_file(&path).unwrap();
        let missing = assets.load_model(testdata("missing.glb"));
        assert_ne!(broken, missing);
        assert_eq!(assets.model(broken), &Model::default());
        assert_eq!(assets.model(missing), &Model::default());
        assert_eq!(assets.take_pending_meshes(), []);
    }

    #[test]
    fn non_linear_interpolation_is_an_error() {
        for interp in ["STEP", "CUBICSPLINE"] {
            let e = parse_err(
                "gate.glb",
                r#""interpolation":"LINEAR""#,
                &format!(r#""interpolation":"{interp}""#),
            );
            assert!(e.starts_with("animation open: "), "{e}");
        }
    }

    #[test]
    fn unsupported_files_are_errors() {
        for (name, from, to, err) in [
            (
                "gate.glb",
                r#""byteLength":8700}"#,
                r#""byteLength":8700,"uri":"gate.bin"}"#,
                "external buffers",
            ),
            (
                "gate.glb",
                r#""uri":"Textures/colormap.png""#,
                r#""uri":"data:image/png;base64,AAAA""#,
                "embedded images",
            ),
            (
                "gate.glb",
                r#""material":0}"#,
                r#""material":0,"mode":1}"#,
                "primitive mode Lines",
            ),
            (
                "gate.glb",
                r#""NORMAL":1,"#,
                "",
                "primitive without POSITION/NORMAL",
            ),
            (
                "gate.glb",
                r#""output":6}"#,
                r#""output":10}"#,
                "animation open: 30 keys for 2 values",
            ),
            (
                "gate.glb",
                r#""path":"translation""#,
                r#""path":"weights""#,
                "animation open: only T/R/S",
            ),
            (
                "character-orc.glb",
                r#""joints":[1,2,3,4,5,6,7]"#,
                r#""joints":[1,2,3,4,5,6]"#,
                "skin: 7 inverse bind matrices for 6 joints",
            ),
            (
                "character-orc.glb",
                r#""children":[5,6,7]"#,
                r#""children":[5,6,7,1]"#,
                "node \"root\": hierarchy has a cycle",
            ),
            (
                "character-orc.glb",
                r#","JOINTS_0":427"#,
                "",
                "skinned mesh",
            ),
            (
                "character-orc.glb",
                r#","WEIGHTS_0":428"#,
                "",
                "skinned mesh",
            ),
        ] {
            let e = parse_err(name, from, to);
            assert!(e.starts_with(err), "{to}: {e}");
        }
    }

    #[test]
    fn missing_ibm_and_indices_and_texture_have_defaults() {
        let orc = parse(&patched(
            "character-orc.glb",
            &[(r#""inverseBindMatrices":426,"#, "")],
        ))
        .unwrap();
        assert_eq!(orc.model.skins[0].inverse_bind, [Mat4::IDENTITY; 7]);

        let gate = patched(
            "gate.glb",
            &[(r#","indices":4"#, ""), (r#","material":0}"#, "}")],
        );
        let mut assets = Assets::default();
        let parsed = parse(&gate).unwrap();
        assert_eq!(parsed.meshes[0][0].indices, (0..132).collect::<Vec<_>>());
        let model = parsed.upload(&mut assets, Path::new(""));
        assert_eq!(model.nodes[1].primitives[0].texture, Handle::WHITE);
    }
}
