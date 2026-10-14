use glam::Vec2;

use crate::Color;
use crate::assets::{Handle, Texture};
use crate::sprite::{Rect, Sprite};

const FIRST: char = ' ';
const LAST: char = '~';
const MIN_ATLAS_WIDTH: u32 = 512;

fn to_u32(px: usize) -> u32 {
    u32::try_from(px).expect("глиф больше u32::MAX пикселей")
}

/// Глиф шрифта; метрики в пикселях растеризации, Y вверх от baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Область глифа в атласе.
    pub uv: Rect,
    /// Сдвиг левого края от пера.
    pub xmin: f32,
    /// Сдвиг нижнего края от baseline.
    pub ymin: f32,
    /// Ширина битмапа.
    pub width: f32,
    /// Высота битмапа.
    pub height: f32,
    /// Сдвиг пера к следующему глифу.
    pub advance: f32,
}

/// Шрифт одного размера; `atlas == None` — заглушка, глифы рисуются текстурой `WHITE`.
#[derive(Debug, PartialEq)]
pub struct FontData {
    /// Текстура покрытия со всеми глифами.
    pub atlas: Option<Handle<Texture>>,
    /// Шаг строки в пикселях растеризации.
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

/// Спрайты строки; `pos` — начало baseline, Y вверх. Символ вне ASCII 32..=126 рисуется как `'?'`.
pub fn layout<'a>(
    font: &'a FontData,
    text: &'a str,
    pos: Vec2,
    scale: f32,
    color: Color,
    layer: i32,
) -> impl Iterator<Item = Sprite> + 'a {
    let texture = font.atlas.unwrap_or(Handle::WHITE);
    let mut cursor = Vec2::ZERO;
    text.chars().filter_map(move |c| {
        if c == '\n' {
            cursor = Vec2::new(0.0, cursor.y - font.line_height);
            return None;
        }
        let g = font.glyph(c).or_else(|| font.glyph('?'))?;
        let min = cursor + Vec2::new(g.xmin, g.ymin);
        cursor.x += g.advance;
        let size = Vec2::new(g.width, g.height);
        if size.cmpeq(Vec2::ZERO).any() {
            return None;
        }
        Some(Sprite {
            texture,
            pos: pos + (min + size / 2.0) * scale,
            size: size * scale,
            uv: g.uv,
            color,
            layer,
            ..Default::default()
        })
    })
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
        .map(|(m, _)| (to_u32(m.width), to_u32(m.height)))
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
pub(crate) fn pack(sizes: &[(u32, u32)], width: u32) -> (Vec<(u32, u32)>, u32) {
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

    fn padded((x, y): (u32, u32), (w, h): (u32, u32)) -> (u32, u32, u32, u32) {
        (x - 1, y - 1, x + w + 1, y + h + 1)
    }

    #[test]
    fn packed_glyphs_keep_padding() {
        let sizes: Vec<_> = (0..40).map(|i| (3 + i % 7, 2 + i % 5)).collect();
        let width = 32;
        let (pos, height) = pack(&sizes, width);
        let rects: Vec<_> = pos
            .iter()
            .zip(&sizes)
            .map(|(&p, &s)| padded(p, s))
            .collect();
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

    fn lay(font: &FontData, text: &str) -> Vec<Sprite> {
        layout(font, text, Vec2::new(100.0, 50.0), 2.0, Color::WHITE, 3).collect()
    }

    #[test]
    fn stub_draws_boxes_on_baseline() {
        let s = lay(&FontData::stub(10.0), "ab");
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].pos, s[0].size),
            (Vec2::new(105.0, 57.0), Vec2::new(10.0, 14.0))
        );
        assert_eq!(s[1].pos, Vec2::new(117.0, 57.0));
        assert_eq!(
            (s[0].texture, s[0].uv),
            (Handle::WHITE, Sprite::default().uv)
        );
        assert_eq!((s[0].color, s[0].layer), (Color::WHITE, 3));
    }

    #[test]
    fn space_advances_without_sprite() {
        let s = lay(&FontData::stub(10.0), "a b");
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].pos.x, 100.0 + 2.0 * 12.0 + 5.0);
    }

    #[test]
    fn newline_moves_down_by_line_height() {
        let s = lay(&FontData::stub(10.0), "ab\nc");
        assert_eq!(s[2].pos, Vec2::new(105.0, 57.0 - 24.0));
    }

    fn font_with(c: char, g: Glyph) -> FontData {
        let mut font = FontData::stub(10.0);
        font.atlas = Some(Handle::new(7));
        font.glyphs[c as usize - FIRST as usize] = g;
        font
    }

    #[test]
    fn non_ascii_falls_back_to_question_mark() {
        let q = Glyph {
            width: 3.0,
            ..*FontData::stub(10.0).glyph('?').unwrap()
        };
        let font = font_with('?', q);
        assert_eq!(lay(&font, "я\t"), lay(&font, "??"));
        assert_eq!(lay(&font, "я")[0].size.x, 6.0);
    }

    #[test]
    fn glyph_center_from_bottom_left_with_y_up() {
        let uv = Rect {
            min: Vec2::new(0.1, 0.2),
            max: Vec2::new(0.3, 0.4),
        };
        let g = Glyph {
            uv,
            xmin: 1.0,
            ymin: -3.0,
            width: 4.0,
            height: 6.0,
            advance: 5.0,
        };
        let s = lay(&font_with('g', g), "g");
        assert_eq!(s[0].pos, Vec2::new(100.0 + 3.0 * 2.0, 50.0));
        assert_eq!(s[0].size, Vec2::new(8.0, 12.0));
        assert_eq!((s[0].texture, s[0].uv), (Handle::new(7), uv));
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
            let i = u32::try_from(i).unwrap();
            let p = Vec2::new((i % r.width) as f32, (i / r.width) as f32);
            let inside = rects
                .iter()
                .any(|(min, max)| p.cmpge(*min).all() && p.cmplt(*max).all());
            assert!(c == 0 || inside, "покрытие {c} вне глифов в {p}");
        }
        assert!(r.coverage.contains(&255));
    }
}
