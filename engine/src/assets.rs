use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use crate::frame::{linear_to_srgb, srgb_to_linear};

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

/// Декодированная текстура, ждущая заливки на GPU; `rgba` премультиплицирован.
#[derive(Debug, PartialEq)]
pub struct PendingTexture {
    pub handle: Handle<Texture>,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Assets {
    next_index: u32,
    pending: Vec<PendingTexture>,
    paths: HashMap<PathBuf, Handle<Texture>>,
}

impl Default for Assets {
    fn default() -> Self {
        let mut assets = Self {
            next_index: 0,
            pending: Vec::new(),
            paths: HashMap::new(),
        };
        assets.push(1, 1, vec![255; 4]);
        assets
    }
}

impl Assets {
    /// Повторный путь отдаёт прежний хэндл; ошибка чтения или декодирования — шахматку и `warn`.
    pub fn load_texture(&mut self, path: impl AsRef<Path>) -> Handle<Texture> {
        let path = path.as_ref();
        if let Some(&h) = self.paths.get(path) {
            return h;
        }
        let handle = match image::open(path) {
            Ok(img) => {
                let img = img.into_rgba8();
                let (w, h) = img.dimensions();
                let mut rgba = img.into_raw();
                premultiply(&mut rgba);
                self.push(w, h, rgba)
            }
            Err(e) => {
                log::warn!("texture {}: {e}", path.display());
                let (w, h, rgba) = checkerboard();
                self.push(w, h, rgba)
            }
        };
        self.paths.insert(path.to_path_buf(), handle);
        handle
    }

    /// Забирает всё, что накопилось с прошлого вызова.
    pub fn take_pending(&mut self) -> Vec<PendingTexture> {
        std::mem::take(&mut self.pending)
    }

    fn push(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> Handle<Texture> {
        let handle = Handle::new(self.next_index);
        self.next_index += 1;
        self.pending.push(PendingTexture {
            handle,
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

fn checkerboard() -> (u32, u32, Vec<u8>) {
    const N: u32 = 8;
    let rgba = (0..N * N)
        .flat_map(|i| {
            if (i % N + i / N).is_multiple_of(2) {
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
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            }]
        );
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
        assert_eq!((t.handle, t.width, t.height), (h, 2, 1));
        assert_eq!(t.rgba, [188, 188, 188, 128, 255, 0, 0, 255]);
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
        assert!(assets.take_pending().is_empty());
    }

    #[test]
    fn take_pending_drains_queue() {
        let mut assets = Assets::default();
        assert_eq!(assets.take_pending().len(), 1);
        assert!(assets.take_pending().is_empty());
    }
}
