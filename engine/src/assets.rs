use std::path::Path;

use crate::frame::{linear_to_srgb, srgb_to_linear};

/// Индекс текстуры; без поколения, потому что текстуры не выгружаются.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureId(pub u32);

impl TextureId {
    /// Белая 1×1, резервируется в `Assets::default`.
    pub const WHITE: TextureId = TextureId(0);
}

/// Декодированная текстура, ждущая заливки на GPU; `rgba` премультиплицирован.
#[derive(Debug, PartialEq)]
pub struct PendingTexture {
    pub id: TextureId,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Assets {
    next_id: u32,
    pending: Vec<PendingTexture>,
}

impl Default for Assets {
    fn default() -> Self {
        let mut assets = Self {
            next_id: 0,
            pending: Vec::new(),
        };
        assets.push(1, 1, vec![255; 4]);
        assets
    }
}

impl Assets {
    /// Ошибка чтения или декодирования даёт шахматку и `warn` в лог, а не панику.
    pub fn load_texture(&mut self, path: impl AsRef<Path>) -> TextureId {
        let path = path.as_ref();
        match image::open(path) {
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
        }
    }

    /// Забирает всё, что накопилось с прошлого вызова.
    pub fn take_pending(&mut self) -> Vec<PendingTexture> {
        std::mem::take(&mut self.pending)
    }

    fn push(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> TextureId {
        let id = TextureId(self.next_id);
        self.next_id += 1;
        self.pending.push(PendingTexture {
            id,
            width,
            height,
            rgba,
        });
        id
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
                id: TextureId::WHITE,
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
        let id = assets.load_texture(&path);
        std::fs::remove_file(&path).unwrap();
        let t = assets.take_pending().pop().unwrap();
        assert_eq!((t.id, t.width, t.height), (id, 2, 1));
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
    fn ids_are_sequential() {
        let mut assets = Assets::default();
        let a = assets.load_texture(temp_path("a.png"));
        let b = assets.load_texture(temp_path("b.png"));
        assert_eq!((a, b), (TextureId(1), TextureId(2)));
    }

    #[test]
    fn take_pending_drains_queue() {
        let mut assets = Assets::default();
        assert_eq!(assets.take_pending().len(), 1);
        assert!(assets.take_pending().is_empty());
    }
}
