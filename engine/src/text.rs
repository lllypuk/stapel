use glam::Vec2;

use crate::assets::{Handle, Texture};
use crate::sprite::Rect;

const FIRST: char = ' ';
const LAST: char = '~';
const MIN_ATLAS_WIDTH: u32 = 512;

/// Глиф шрифта; метрики в пикселях растеризации, Y вверх от baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub uv: Rect,
    pub xmin: f32,
    pub ymin: f32,
    pub width: f32,
    pub height: f32,
    pub advance: f32,
}

/// Шрифт одного размера; `atlas == None` — заглушка, глифы рисуются текстурой `WHITE`.
#[derive(Debug, PartialEq)]
pub struct FontData {
    pub atlas: Option<Handle<Texture>>,
    pub line_height: f32,
    pub(crate) glyphs: Vec<Glyph>,
}

impl FontData {
    /// `None` вне ASCII 32..=126.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        let i = (c as usize).checked_sub(FIRST as usize)?;
        self.glyphs.get(i)
    }

    pub(crate) fn stub(px: f32) -> Self {
        let full = Rect {
            min: Vec2::ZERO,
            max: Vec2::ONE,
        };
        let glyphs = (FIRST..=LAST)
            .map(|c| Glyph {
                uv: full,
                xmin: 0.0,
                ymin: 0.0,
                width: if c == ' ' { 0.0 } else { 0.5 * px },
                height: if c == ' ' { 0.0 } else { 0.7 * px },
                advance: 0.6 * px,
            })
            .collect();
        Self {
            atlas: None,
            line_height: 1.2 * px,
            glyphs,
        }
    }
}

/// Растеризованный шрифт до выделения текстуры атласа.
pub(crate) struct Rasterized {
    pub glyphs: Vec<Glyph>,
    pub line_height: f32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
}

pub(crate) fn rasterize(ttf: &[u8], px: f32) -> Result<Rasterized, &'static str> {
    let settings = fontdue::FontSettings {
        scale: px,
        ..Default::default()
    };
    let font = fontdue::Font::from_bytes(ttf, settings)?;
    let bitmaps: Vec<_> = (FIRST..=LAST).map(|c| font.rasterize(c, px)).collect();
    let sizes: Vec<_> = bitmaps
        .iter()
        .map(|(m, _)| (m.width as u32, m.height as u32))
        .collect();
    let width = sizes
        .iter()
        .map(|&(w, _)| w + 2)
        .max()
        .unwrap_or(0)
        .max(MIN_ATLAS_WIDTH);
    let (positions, height) = pack(&sizes, width);
    let mut coverage = vec![0; (width * height) as usize];
    let atlas = Vec2::new(width as f32, height as f32);
    let glyphs = bitmaps
        .iter()
        .zip(&positions)
        .map(|((m, bitmap), &(x, y))| {
            for (row, src) in bitmap.chunks_exact(m.width.max(1)).enumerate() {
                let start = (y as usize + row) * width as usize + x as usize;
                coverage[start..start + m.width].copy_from_slice(src);
            }
            let min = Vec2::new(x as f32, y as f32);
            let size = Vec2::new(m.width as f32, m.height as f32);
            Glyph {
                uv: Rect {
                    min: min / atlas,
                    max: (min + size) / atlas,
                },
                xmin: m.xmin as f32,
                ymin: m.ymin as f32,
                width: size.x,
                height: size.y,
                advance: m.advance_width,
            }
        })
        .collect();
    let line_height = font
        .horizontal_line_metrics(px)
        .map_or(1.2 * px, |l| l.new_line_size);
    Ok(Rasterized {
        glyphs,
        line_height,
        width,
        height,
        coverage,
    })
}

/// Полочная упаковка: левые верхние углы глифов и высота атласа.
/// Каждый глиф окружён пустой рамкой в 1 texel, иначе `Linear` подмешает соседа или край.
fn pack(sizes: &[(u32, u32)], width: u32) -> (Vec<(u32, u32)>, u32) {
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    let positions = sizes
        .iter()
        .map(|&(w, h)| {
            let (cw, ch) = (w + 2, h + 2);
            assert!(cw <= width, "глиф {w}px шире атласа {width}px");
            if x + cw > width {
                y += shelf;
                x = 0;
                shelf = 0;
            }
            let pos = (x + 1, y + 1);
            x += cw;
            shelf = shelf.max(ch);
            pos
        })
        .collect();
    (positions, y + shelf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn padded(&(x, y): &(u32, u32), &(w, h): &(u32, u32)) -> (u32, u32, u32, u32) {
        (x - 1, y - 1, x + w + 1, y + h + 1)
    }

    #[test]
    fn packed_glyphs_keep_padding() {
        let sizes: Vec<_> = (0..40).map(|i| (3 + i % 7, 2 + i % 5)).collect();
        let width = 32;
        let (pos, height) = pack(&sizes, width);
        let rects: Vec<_> = pos.iter().zip(&sizes).map(|(p, s)| padded(p, s)).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.0 < a.2 && a.2 <= width && a.3 <= height, "{i}: {a:?}");
            for b in &rects[i + 1..] {
                let overlap = a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3;
                assert!(!overlap, "{a:?} {b:?}");
            }
        }
        assert!(height > sizes[0].1 + 2, "ожидалось несколько полок");
    }

    #[test]
    fn glyph_lookup_is_ascii_only() {
        let font = FontData::stub(10.0);
        assert!(font.glyph(' ').is_some() && font.glyph('~').is_some());
        assert!(font.glyph('\u{7f}').is_none() && font.glyph('\n').is_none());
        assert!(font.glyph('я').is_none());
    }

    #[test]
    fn atlas_coverage_stays_inside_glyphs() {
        let ttf = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../assets/fonts/JetBrainsMono-Regular.ttf"
        ))
        .unwrap();
        let r = rasterize(&ttf, 32.0).unwrap();
        let atlas = Vec2::new(r.width as f32, r.height as f32);
        let rects: Vec<_> = r
            .glyphs
            .iter()
            .map(|g| ((g.uv.min * atlas).round(), (g.uv.max * atlas).round()))
            .collect();
        for (i, &c) in r.coverage.iter().enumerate() {
            let p = Vec2::new((i as u32 % r.width) as f32, (i as u32 / r.width) as f32);
            let inside = rects
                .iter()
                .any(|(min, max)| p.cmpge(*min).all() && p.cmplt(*max).all());
            assert!(c == 0 || inside, "покрытие {c} вне глифов в {p}");
        }
        assert!(r.coverage.contains(&255));
    }
}
