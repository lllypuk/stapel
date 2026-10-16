use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use glam::Vec3;

use crate::audio::{Sound, decode};
use crate::frame::{linear_to_srgb, srgb_to_linear};
use crate::mesh::{Mesh, PendingMesh, Vertex};
use crate::model::{self, Model};
use crate::text::{self, FontData};

/// Индекс ресурса типа `T`; без поколения, пока индексы append-only.
pub struct Handle<T> {
    pub(crate) index: u32,
    _marker: PhantomData<T>,
}

/// Маркер текстурного хэндла.
pub enum Texture {}

/// Маркер хэндла шрифта.
pub enum Font {}

impl<T> Handle<T> {
    pub(crate) const fn new(index: u32) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }
}

impl Handle<Texture> {
    /// Белая 1×1, резервируется в `Assets::default`.
    pub const WHITE: Self = Self::new(0);
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for Handle<T> {}

impl<T> PartialOrd for Handle<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Handle<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({})", self.index)
    }
}

/// Вид текстуры: от него зависят формат и сэмплер на GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureKind {
    /// `Rgba8UnormSrgb`, сэмплер `Nearest`.
    Color,
    /// Линейное покрытие `[c, c, c, c]`: `Rgba8Unorm`, сэмплер `Linear`.
    Coverage,
}

/// Декодированная текстура, ждущая заливки на GPU; `rgba` премультиплицирован.
#[derive(Debug, PartialEq)]
pub struct PendingTexture {
    /// Чей это хэндл.
    pub handle: Handle<Texture>,
    /// Формат и сэмплер на GPU.
    pub kind: TextureKind,
    /// Ширина в texel.
    pub width: u32,
    /// Высота в texel.
    pub height: u32,
    /// Пиксели построчно, 4 байта на texel.
    pub rgba: Vec<u8>,
}

/// Позиции и индексы меша на CPU, как их получил `Assets::add_mesh`.
#[derive(Debug, Default, PartialEq)]
pub struct MeshGeometry {
    /// Позиции вершин в пространстве меша.
    pub positions: Vec<Vec3>,
    /// Тройки индексов треугольников.
    pub indices: Vec<u32>,
}

/// Версия файла для перезагрузки; правка с тем же `mtime` и длиной не заметна.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    mtime: SystemTime,
    len: u64,
}

impl Stamp {
    /// `None` — файла нет или он не читается.
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            mtime: meta.modified().ok()?,
            len: meta.len(),
        })
    }
}

/// Хэндл файла и версия, с которой он загружен последним.
struct Watched<T> {
    handle: Handle<T>,
    stamp: Option<Stamp>,
}

/// Реестр текстур, мешей, моделей, шрифтов и звуков; новые текстуры и меши ждут заливки на GPU до `take_pending*`.
pub struct Assets {
    next_index: u32,
    pending: Vec<PendingTexture>,
    geometry: Vec<MeshGeometry>,
    pending_meshes: Vec<PendingMesh>,
    models: Vec<Model>,
    model_paths: HashMap<PathBuf, Handle<Model>>,
    paths: HashMap<PathBuf, Watched<Texture>>,
    fonts: Vec<FontData>,
    font_keys: HashMap<(PathBuf, u32), Handle<Font>>,
    sounds: Vec<Sound>,
    /// Заменённые перезагрузкой: последний `Arc` у голоса освободился бы в аудиоколбэке.
    retired: Vec<Sound>,
    sound_paths: HashMap<PathBuf, Watched<Sound>>,
    sample_rate: u32,
}

impl Default for Assets {
    fn default() -> Self {
        let mut assets = Self {
            next_index: 0,
            pending: Vec::new(),
            geometry: Vec::new(),
            pending_meshes: Vec::new(),
            models: Vec::new(),
            model_paths: HashMap::new(),
            paths: HashMap::new(),
            fonts: Vec::new(),
            font_keys: HashMap::new(),
            sounds: Vec::new(),
            retired: Vec::new(),
            sound_paths: HashMap::new(),
            sample_rate: 48_000,
        };
        assets.push(TextureKind::Color, 1, 1, vec![255; 4]);
        assets
    }
}

impl Assets {
    /// Повторный путь отдаёт прежний хэндл; ошибка чтения или декодирования — шахматку и `warn`.
    pub fn load_texture(&mut self, path: impl AsRef<Path>) -> Handle<Texture> {
        let path = path.as_ref();
        if let Some(w) = self.paths.get(path) {
            return w.handle;
        }
        // Версия — до чтения: правка во время декода увидится следующим опросом.
        let stamp = Stamp::of(path);
        let (w, h, rgba) = decode_png(path).unwrap_or_else(|e| {
            log::warn!("texture {}: {e}", path.display());
            checkerboard()
        });
        let handle = self.push(TextureKind::Color, w, h, rgba);
        self.paths
            .insert(path.to_path_buf(), Watched { handle, stamp });
        handle
    }

    /// Перечитывает изменившиеся текстуры (в pending) и звуки под прежними хэндлами.
    /// Версия запоминается и при ошибке: битый файл даёт один `warn` и оставляет прежний ресурс.
    pub fn reload_changed(&mut self) {
        for (path, w) in &mut self.paths {
            let stamp = Stamp::of(path);
            if stamp == w.stamp {
                continue;
            }
            w.stamp = stamp;
            match decode_png(path) {
                Ok((width, height, rgba)) => self.pending.push(PendingTexture {
                    handle: w.handle,
                    kind: TextureKind::Color,
                    width,
                    height,
                    rgba,
                }),
                Err(e) => log::warn!("texture {}: {e}", path.display()),
            }
        }
        for (path, w) in &mut self.sound_paths {
            let stamp = Stamp::of(path);
            if stamp == w.stamp {
                continue;
            }
            w.stamp = stamp;
            match decode::load(path, self.sample_rate) {
                Ok(frames) => {
                    let new = Sound {
                        frames: frames.into(),
                    };
                    let old = std::mem::replace(&mut self.sounds[w.handle.index as usize], new);
                    self.retired.push(old);
                }
                Err(e) => log::warn!("sound {}: {e}", path.display()),
            }
        }
    }

    /// Кэш по `(path, px)`: метрики зависят от размера. Ошибка — шрифт-заглушка без атласа и `warn`.
    pub fn load_font(&mut self, path: impl AsRef<Path>, px: f32) -> Handle<Font> {
        let path = path.as_ref();
        let key = (path.to_path_buf(), px.to_bits());
        if let Some(&h) = self.font_keys.get(&key) {
            return h;
        }
        let raster = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|ttf| text::rasterize(&ttf, px).map_err(str::to_owned));
        let font = match raster {
            Ok(r) => FontData {
                atlas: Some(self.add_coverage(r.width, r.height, &r.coverage)),
                line_height: r.line_height,
                glyphs: r.glyphs,
            },
            Err(e) => {
                log::warn!("font {}: {e}", path.display());
                FontData::stub(px)
            }
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX шрифтов не поместится в память"
        )]
        let handle = Handle::new(self.fonts.len() as u32);
        self.fonts.push(font);
        self.font_keys.insert(key, handle);
        handle
    }

    /// Данные шрифта по хэндлу.
    pub fn font(&self, handle: Handle<Font>) -> &FontData {
        &self.fonts[handle.index as usize]
    }

    /// Повторный путь отдаёт прежний хэндл; ошибка чтения или формата — пустой звук и `warn`.
    pub fn load_sound(&mut self, path: impl AsRef<Path>) -> Handle<Sound> {
        let path = path.as_ref();
        if let Some(w) = self.sound_paths.get(path) {
            return w.handle;
        }
        let stamp = Stamp::of(path);
        let frames = decode::load(path, self.sample_rate).unwrap_or_else(|e| {
            log::warn!("sound {}: {e}", path.display());
            Vec::new()
        });
        let handle = self.push_sound(frames);
        self.sound_paths
            .insert(path.to_path_buf(), Watched { handle, stamp });
        handle
    }

    /// Звук из стерео-интерлива в `sample_rate()`; живёт, пока живут `Assets`.
    ///
    /// # Panics
    /// На нечётную длину или неконечный отсчёт.
    pub fn add_sound(&mut self, frames: Vec<f32>) -> Handle<Sound> {
        assert!(
            frames.len().is_multiple_of(2),
            "add_sound: нечётная длина {}",
            frames.len()
        );
        assert!(
            frames.iter().all(|s| s.is_finite()),
            "add_sound: неконечный отсчёт"
        );
        self.push_sound(frames)
    }

    /// Частота звуков `Assets` — частота устройства.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn push_sound(&mut self, frames: Vec<f32>) -> Handle<Sound> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX звуков не поместится в память"
        )]
        let handle = Handle::new(self.sounds.len() as u32);
        self.sounds.push(Sound {
            frames: frames.into(),
        });
        handle
    }

    /// Звук по хэндлу.
    pub fn sound(&self, handle: Handle<Sound>) -> &Sound {
        &self.sounds[handle.index as usize]
    }

    /// Частота, в которую ресемплируются звуки; после загрузки кэш хранил бы звук чужой частоты.
    pub(crate) fn set_sample_rate(&mut self, rate: u32) {
        debug_assert!(self.sounds.is_empty(), "set_sample_rate after load_sound");
        self.sample_rate = rate;
    }

    /// Текстура покрытия из байта на texel: белый, уже премультиплицированный.
    pub(crate) fn add_coverage(
        &mut self,
        width: u32,
        height: u32,
        coverage: &[u8],
    ) -> Handle<Texture> {
        assert_eq!(coverage.len(), (width * height) as usize);
        let rgba = coverage.iter().flat_map(|&c| [c; 4]).collect();
        self.push(TextureKind::Coverage, width, height, rgba)
    }

    /// Забирает всё, что накопилось с прошлого вызова, в порядке поступления: замена слота идёт после его создания.
    pub fn take_pending(&mut self) -> Vec<PendingTexture> {
        std::mem::take(&mut self.pending)
    }

    /// Меш из вершин и индексов треугольников.
    pub fn add_mesh(&mut self, vertices: Vec<Vertex>, indices: Vec<u32>) -> Handle<Mesh> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX мешей не поместится в память"
        )]
        let handle = Handle::new(self.geometry.len() as u32);
        self.geometry.push(MeshGeometry {
            positions: vertices.iter().map(|v| Vec3::from(v.pos)).collect(),
            indices: indices.clone(),
        });
        self.pending_meshes.push(PendingMesh {
            handle,
            vertices,
            indices,
        });
        handle
    }

    /// Геометрия меша на CPU; хранится одна на меш и после заливки на GPU.
    pub fn mesh_geometry(&self, handle: Handle<Mesh>) -> &MeshGeometry {
        &self.geometry[handle.index as usize]
    }

    /// Забирает меши, добавленные с прошлого вызова, в порядке хэндлов.
    pub fn take_pending_meshes(&mut self) -> Vec<PendingMesh> {
        std::mem::take(&mut self.pending_meshes)
    }

    /// Повторный путь отдаёт прежний хэндл; горячо не перезагружается. Ошибка — пустая модель и `warn`.
    pub fn load_model(&mut self, path: impl AsRef<Path>) -> Handle<Model> {
        let path = path.as_ref();
        if let Some(&h) = self.model_paths.get(path) {
            return h;
        }
        let parsed = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| model::parse(&bytes));
        let model = match parsed {
            Ok(p) => p.upload(self, path.parent().unwrap_or(Path::new(""))),
            Err(e) => {
                log::warn!("model {}: {e}", path.display());
                Model::default()
            }
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX моделей не поместится в память"
        )]
        let handle = Handle::new(self.models.len() as u32);
        self.models.push(model);
        self.model_paths.insert(path.to_path_buf(), handle);
        handle
    }

    /// Модель по хэндлу.
    pub fn model(&self, handle: Handle<Model>) -> &Model {
        &self.models[handle.index as usize]
    }

    fn push(
        &mut self,
        kind: TextureKind,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Handle<Texture> {
        let handle = Handle::new(self.next_index);
        self.next_index += 1;
        self.pending.push(PendingTexture {
            handle,
            kind,
            width,
            height,
            rgba,
        });
        handle
    }
}

/// RGB умножается на α в линейном пространстве; α остаётся без гаммы.
pub fn premultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = f64::from(px[3]) / 255.0;
        for c in &mut px[..3] {
            *c = linear_to_srgb(srgb_to_linear(*c) * a);
        }
    }
}

fn decode_png(path: &Path) -> image::ImageResult<(u32, u32, Vec<u8>)> {
    let img = image::open(path)?.into_rgba8();
    let (w, h) = img.dimensions();
    let mut rgba = img.into_raw();
    premultiply(&mut rgba);
    Ok((w, h, rgba))
}

fn checkerboard() -> (u32, u32, Vec<u8>) {
    const N: u32 = 8;
    let rgba = (0..N)
        .flat_map(|y| (0..N).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            if (x + y).is_multiple_of(2) {
                [255, 0, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        })
        .collect();
    (N, N, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("engine-assets-{}-{name}", std::process::id()))
    }

    fn assert_checkerboard(t: &PendingTexture) {
        assert_eq!((t.width, t.height), (8, 8));
        assert_eq!(&t.rgba[..8], &[255, 0, 255, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn premultiply_is_linear() {
        let mut px = [255, 255, 255, 128, 10, 20, 30, 0, 10, 20, 30, 255];
        premultiply(&mut px);
        assert_eq!(px, [188, 188, 188, 128, 0, 0, 0, 0, 10, 20, 30, 255]);
    }

    #[test]
    fn default_queues_white_first() {
        let pending = Assets::default().take_pending();
        assert_eq!(
            pending,
            [PendingTexture {
                handle: Handle::WHITE,
                kind: TextureKind::Color,
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            }]
        );
    }

    #[test]
    fn meshes_are_pending_in_handle_order() {
        let mut assets = Assets::default();
        let tri = vec![Vertex::default(); 3];
        let a = assets.add_mesh(tri.clone(), vec![0, 1, 2]);
        let b = assets.add_mesh(Vec::new(), Vec::new());
        assert_eq!((a, b), (Handle::new(0), Handle::new(1)));
        assert_eq!(
            assets.take_pending_meshes(),
            [
                PendingMesh {
                    handle: a,
                    vertices: tri,
                    indices: vec![0, 1, 2],
                },
                PendingMesh {
                    handle: b,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                },
            ]
        );
        assert_eq!(assets.take_pending_meshes(), []);
        assert_eq!(assets.add_mesh(Vec::new(), Vec::new()), Handle::new(2));
    }

    #[test]
    fn mesh_geometry_keeps_positions_and_indices_after_upload() {
        let mut assets = Assets::default();
        let tri: Vec<Vertex> = [0.0, 1.0, 2.0]
            .into_iter()
            .map(|x| Vertex {
                pos: [x, 1.0, 2.0],
                ..Vertex::default()
            })
            .collect();
        let a = assets.add_mesh(tri, vec![2, 1, 0]);
        let b = assets.add_mesh(Vec::new(), Vec::new());
        assets.take_pending_meshes();
        assert_eq!(
            assets.mesh_geometry(a),
            &MeshGeometry {
                positions: vec![
                    Vec3::new(0.0, 1.0, 2.0),
                    Vec3::new(1.0, 1.0, 2.0),
                    Vec3::new(2.0, 1.0, 2.0),
                ],
                indices: vec![2, 1, 0],
            }
        );
        assert_eq!(assets.mesh_geometry(b), &MeshGeometry::default());
    }

    #[test]
    fn reloaded_model_does_not_duplicate_geometry() {
        let path = model::tests::testdata("racing/roadStraight.glb");
        let mut assets = Assets::default();
        let h = assets.load_model(&path);
        assert_eq!(assets.geometry.len(), 3);
        assert_eq!(assets.load_model(&path), h);
        assert_eq!(assets.geometry.len(), 3);
        let road = assets.model(h).nodes[0].primitives[2].mesh;
        let g = assets.mesh_geometry(road);
        assert!(!g.indices.is_empty() && g.indices.len().is_multiple_of(3));
        assert!(g.positions.iter().all(|p| (0.0..=1.0).contains(&p.x)));
    }

    #[test]
    fn loads_png_premultiplied() {
        let path = temp_path("valid.png");
        let src = [255, 255, 255, 128, 255, 0, 0, 255];
        image::save_buffer(&path, &src, 2, 1, image::ExtendedColorType::Rgba8).unwrap();
        let mut assets = Assets::default();
        let h = assets.load_texture(&path);
        std::fs::remove_file(&path).unwrap();
        let t = assets.take_pending().pop().unwrap();
        assert_eq!(
            (t.handle, t.kind, t.width, t.height),
            (h, TextureKind::Color, 2, 1)
        );
        assert_eq!(t.rgba, [188, 188, 188, 128, 255, 0, 0, 255]);
    }

    #[test]
    fn coverage_is_not_premultiplied() {
        let mut assets = Assets::default();
        let h = assets.add_coverage(2, 1, &[128, 255]);
        let t = assets.take_pending().pop().unwrap();
        assert_eq!(
            (t.handle, t.kind, t.width, t.height),
            (h, TextureKind::Coverage, 2, 1)
        );
        assert_eq!(t.rgba, [128, 128, 128, 128, 255, 255, 255, 255]);
    }

    #[test]
    fn missing_file_gives_checkerboard() {
        let mut assets = Assets::default();
        assets.load_texture(temp_path("missing.png"));
        assert_checkerboard(&assets.take_pending()[1]);
    }

    #[test]
    fn broken_png_gives_checkerboard() {
        let path = temp_path("broken.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot a png").unwrap();
        let mut assets = Assets::default();
        assets.load_texture(&path);
        std::fs::remove_file(&path).unwrap();
        assert_checkerboard(&assets.take_pending()[1]);
    }

    fn write_px(path: &Path, rgba: [u8; 4]) {
        image::save_buffer(path, &rgba, 1, 1, image::ExtendedColorType::Rgba8).unwrap();
    }

    /// Новая версия файла без паузы: mtime на Linux грубый.
    fn set_mtime(path: &Path, secs: u64) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn changed_texture_is_pending_under_same_handle() {
        let path = temp_path("reload.png");
        write_px(&path, [255, 0, 0, 255]);
        let mut assets = Assets::default();
        let h = assets.load_texture(&path);
        assets.take_pending();
        write_px(&path, [0, 255, 0, 255]);
        set_mtime(&path, 1000);
        assets.reload_changed();
        let pending = assets.take_pending();
        assets.reload_changed();
        let again = assets.take_pending();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            pending,
            [PendingTexture {
                handle: h,
                kind: TextureKind::Color,
                width: 1,
                height: 1,
                rgba: vec![0, 255, 0, 255],
            }]
        );
        assert_eq!(again, []);
    }

    #[test]
    fn unchanged_texture_is_not_reloaded() {
        let path = temp_path("unchanged.png");
        write_px(&path, [255, 0, 0, 255]);
        let mut assets = Assets::default();
        assets.load_texture(&path);
        assets.take_pending();
        assets.reload_changed();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(assets.take_pending(), []);
    }

    #[test]
    fn broken_reload_keeps_old_texture_and_remembers_version() {
        let path = temp_path("broken-reload.png");
        write_px(&path, [255, 0, 0, 255]);
        let mut assets = Assets::default();
        assets.load_texture(&path);
        assets.take_pending();
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot a png").unwrap();
        set_mtime(&path, 1000);
        assets.reload_changed();
        let stamp = Stamp::of(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(assets.take_pending(), []);
        // Запомненная версия — повторный опрос не декодирует и не пишет `warn` снова.
        assert!(stamp.is_some());
        assert_eq!(assets.paths[&path].stamp, stamp);
    }

    #[test]
    fn stub_is_replaced_when_file_appears() {
        let path = temp_path("appears.png");
        let mut assets = Assets::default();
        let h = assets.load_texture(&path);
        assets.take_pending();
        assets.reload_changed();
        assert_eq!(assets.take_pending(), []);
        write_px(&path, [0, 0, 255, 255]);
        assets.reload_changed();
        std::fs::remove_file(&path).unwrap();
        let t = assets.take_pending().pop().unwrap();
        assert_eq!((t.handle, t.rgba), (h, vec![0, 0, 255, 255]));
    }

    #[test]
    fn stamp_tracks_mtime_and_len() {
        let path = temp_path("stamp.png");
        std::fs::write(&path, b"ab").unwrap();
        set_mtime(&path, 1000);
        let a = Stamp::of(&path);
        set_mtime(&path, 2000);
        let b = Stamp::of(&path);
        std::fs::write(&path, b"abc").unwrap();
        set_mtime(&path, 2000);
        let c = Stamp::of(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(a.is_some());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(Stamp::of(&path), None);
    }

    #[test]
    fn white_is_index_zero() {
        assert_eq!(Handle::<Texture>::WHITE.index, 0);
    }

    #[test]
    fn different_paths_get_sequential_handles() {
        let mut assets = Assets::default();
        let a = assets.load_texture(temp_path("a.png"));
        let b = assets.load_texture(temp_path("b.png"));
        assert_eq!((a.index, b.index), (1, 2));
    }

    #[test]
    fn same_path_is_loaded_once() {
        let path = temp_path("cached.png");
        image::save_buffer(
            &path,
            &[0, 0, 0, 255],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        let mut assets = Assets::default();
        let a = assets.load_texture(&path);
        let b = assets.load_texture(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(a, b);
        assert_eq!(assets.take_pending().len(), 2);
    }

    #[test]
    fn missing_file_is_cached_too() {
        let path = temp_path("missing-twice.png");
        let mut assets = Assets::default();
        let a = assets.load_texture(&path);
        assert_eq!(assets.take_pending().len(), 2);
        let b = assets.load_texture(&path);
        assert_eq!(a, b);
        assert_eq!(assets.take_pending().len(), 0);
    }

    const FONT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/fonts/JetBrainsMono-Regular.ttf"
    );

    #[test]
    fn font_has_printable_ascii_in_coverage_atlas() {
        let mut assets = Assets::default();
        let h = assets.load_font(FONT, 32.0);
        let t = assets.take_pending().pop().unwrap();
        let font = assets.font(h);
        assert_eq!(
            (Some(t.handle), t.kind),
            (font.atlas, TextureKind::Coverage)
        );
        for c in ' '..='~' {
            let g = font.glyph(c).unwrap();
            assert!(g.advance > 0.0, "{c:?}");
            assert_eq!(g.width == 0.0, c == ' ', "{c:?}");
        }
        assert!(font.line_height > 32.0);
    }

    #[test]
    fn font_is_cached_by_path_and_px() {
        let mut assets = Assets::default();
        let a = assets.load_font(FONT, 32.0);
        let b = assets.load_font(FONT, 32.0);
        let small = assets.load_font(FONT, 16.0);
        assert_eq!(a, b);
        assert_ne!(a, small);
        assert_eq!(assets.take_pending().len(), 3);
        let (big, small) = (assets.font(a), assets.font(small));
        assert_ne!(big.atlas, small.atlas);
        assert!(big.glyph('M').unwrap().advance > small.glyph('M').unwrap().advance);
        assert!(big.line_height > small.line_height);
    }

    #[test]
    fn missing_font_gives_stub_without_atlas() {
        let mut assets = Assets::default();
        let h = assets.load_font(temp_path("missing.ttf"), 10.0);
        assert_eq!(assets.take_pending().len(), 1);
        let font = assets.font(h);
        assert_eq!(font.atlas, None);
        assert_eq!(font.line_height, 12.0);
        let g = font.glyph('A').unwrap();
        assert_eq!((g.width, g.height, g.advance), (5.0, 7.0, 6.0));
    }

    #[test]
    fn broken_font_gives_stub() {
        let path = temp_path("broken.ttf");
        std::fs::write(&path, b"not a font").unwrap();
        let mut assets = Assets::default();
        let h = assets.load_font(&path, 10.0);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(assets.font(h).atlas, None);
    }

    const SINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sine.ogg");

    #[test]
    fn sound_is_cached_by_path() {
        let mut assets = Assets::default();
        let a = assets.load_sound(SINE);
        let b = assets.load_sound(SINE);
        let missing = assets.load_sound(temp_path("missing.wav"));
        assert_eq!(a, b);
        assert_ne!(a, missing);
        assert!(!assets.sound(a).frames.is_empty());
        assert!(assets.sound(missing).frames.is_empty());
    }

    #[test]
    fn sound_is_resampled_to_assets_rate() {
        let mut assets = Assets::default();
        assets.set_sample_rate(44_100);
        let h = assets.load_sound(SINE);
        let native = assets.sound(h).frames.len();
        let mut assets = Assets::default();
        let h = assets.load_sound(SINE);
        assert_eq!(
            assets.sound(h).frames.len() / 2,
            (native / 2 * 480).div_ceil(441)
        );
    }

    #[test]
    fn added_sound_gets_new_handle_with_same_frames() {
        let mut assets = Assets::default();
        let loaded = assets.load_sound(SINE);
        let a = assets.add_sound(vec![0.1, -0.2, 0.3, -0.4]);
        let b = assets.add_sound(vec![0.5, 0.5]);
        assert_ne!(a, loaded);
        assert_ne!(a, b);
        assert_eq!(&*assets.sound(a).frames, &[0.1, -0.2, 0.3, -0.4]);
        assert_eq!(&*assets.sound(b).frames, &[0.5, 0.5]);
    }

    #[test]
    fn sample_rate_is_assets_rate() {
        let mut assets = Assets::default();
        assert_eq!(assets.sample_rate(), 48_000);
        assets.set_sample_rate(44_100);
        assert_eq!(assets.sample_rate(), 44_100);
    }

    #[test]
    #[should_panic(expected = "нечётная длина 3")]
    fn added_sound_of_odd_length_panics() {
        Assets::default().add_sound(vec![0.0; 3]);
    }

    #[test]
    #[should_panic(expected = "неконечный отсчёт")]
    fn added_sound_with_nan_panics() {
        Assets::default().add_sound(vec![0.0, f32::NAN]);
    }

    #[test]
    #[should_panic(expected = "set_sample_rate after load_sound")]
    fn sample_rate_after_load_panics_in_debug() {
        let mut assets = Assets::default();
        assets.load_sound(SINE);
        assets.set_sample_rate(44_100);
    }

    fn write_wav(path: &Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn changed_sound_replaces_frames_and_retires_old() {
        let path = temp_path("reload.wav");
        write_wav(&path, &[0]);
        let mut assets = Assets::default();
        let h = assets.load_sound(&path);
        let old = assets.sound(h).frames.clone();
        write_wav(&path, &[16384, 0]);
        set_mtime(&path, 1000);
        assets.reload_changed();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(&*assets.sound(h).frames, [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(&*old, [0.0, 0.0]);
        assert_eq!(Arc::strong_count(&old), 2);
    }

    #[test]
    fn broken_sound_reload_keeps_old_sound() {
        let path = temp_path("broken-reload.wav");
        write_wav(&path, &[16384]);
        let mut assets = Assets::default();
        let h = assets.load_sound(&path);
        std::fs::write(&path, b"RIFF not a wav").unwrap();
        set_mtime(&path, 1000);
        assets.reload_changed();
        let stamp = Stamp::of(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(&*assets.sound(h).frames, [0.5, 0.5]);
        assert!(assets.retired.is_empty());
        assert!(stamp.is_some());
        assert_eq!(assets.sound_paths[&path].stamp, stamp);
    }

    #[test]
    fn unchanged_sound_is_not_reloaded() {
        let path = temp_path("unchanged.wav");
        write_wav(&path, &[0]);
        let mut assets = Assets::default();
        assets.load_sound(&path);
        assets.reload_changed();
        std::fs::remove_file(&path).unwrap();
        assert!(assets.retired.is_empty());
    }

    #[test]
    fn take_pending_drains_queue() {
        let mut assets = Assets::default();
        assert_eq!(assets.take_pending().len(), 1);
        assert_eq!(assets.take_pending().len(), 0);
    }

    #[test]
    fn handle_debug_shows_index() {
        assert_eq!(format!("{:?}", Handle::<Font>::new(3)), "Handle(3)");
    }

    #[test]
    fn handle_hash_is_index_hash() {
        use std::hash::BuildHasher;
        let state = std::collections::hash_map::RandomState::new();
        assert_eq!(
            state.hash_one(Handle::<Texture>::new(7)),
            state.hash_one(7u32)
        );
        assert_ne!(
            state.hash_one(Handle::<Texture>::new(7)),
            state.hash_one(Handle::<Texture>::new(8))
        );
    }

    #[test]
    fn checkerboard_alternates_every_texel() {
        let (w, h, rgba) = checkerboard();
        assert_eq!((w, h, rgba.len()), (8, 8, 8 * 8 * 4));
        for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (i % 8, i / 8);
            let magenta = (x + y).is_multiple_of(2);
            assert_eq!(px[0] == 255, magenta, "({x}, {y})");
        }
    }
}
