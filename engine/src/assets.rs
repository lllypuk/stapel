use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use crate::audio::{Sound, decode};
use crate::frame::{linear_to_srgb, srgb_to_linear};
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

/// Реестр текстур, шрифтов и звуков; новые текстуры ждут заливки на GPU до `take_pending`.
pub struct Assets {
    next_index: u32,
    pending: Vec<PendingTexture>,
    paths: HashMap<PathBuf, Handle<Texture>>,
    fonts: Vec<FontData>,
    font_keys: HashMap<(PathBuf, u32), Handle<Font>>,
    sounds: Vec<Sound>,
    sound_paths: HashMap<PathBuf, Handle<Sound>>,
    sample_rate: u32,
}

impl Default for Assets {
    fn default() -> Self {
        let mut assets = Self {
            next_index: 0,
            pending: Vec::new(),
            paths: HashMap::new(),
            fonts: Vec::new(),
            font_keys: HashMap::new(),
            sounds: Vec::new(),
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
        if let Some(&h) = self.paths.get(path) {
            return h;
        }
        let handle = match image::open(path) {
            Ok(img) => {
                let img = img.into_rgba8();
                let (w, h) = img.dimensions();
                let mut rgba = img.into_raw();
                premultiply(&mut rgba);
                self.push(TextureKind::Color, w, h, rgba)
            }
            Err(e) => {
                log::warn!("texture {}: {e}", path.display());
                let (w, h, rgba) = checkerboard();
                self.push(TextureKind::Color, w, h, rgba)
            }
        };
        self.paths.insert(path.to_path_buf(), handle);
        handle
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
        if let Some(&h) = self.sound_paths.get(path) {
            return h;
        }
        let frames = decode::load(path, self.sample_rate).unwrap_or_else(|e| {
            log::warn!("sound {}: {e}", path.display());
            Vec::new()
        });
        #[expect(
            clippy::cast_possible_truncation,
            reason = "u32::MAX звуков не поместится в память"
        )]
        let handle = Handle::new(self.sounds.len() as u32);
        self.sounds.push(Sound {
            frames: frames.into(),
        });
        self.sound_paths.insert(path.to_path_buf(), handle);
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

    /// Забирает всё, что накопилось с прошлого вызова.
    pub fn take_pending(&mut self) -> Vec<PendingTexture> {
        std::mem::take(&mut self.pending)
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
    #[should_panic(expected = "set_sample_rate after load_sound")]
    fn sample_rate_after_load_panics_in_debug() {
        let mut assets = Assets::default();
        assets.load_sound(SINE);
        assets.set_sample_rate(44_100);
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
