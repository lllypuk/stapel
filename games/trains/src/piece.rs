use stapel::{Mat4, Vec2, vec2, vec3};

/// Верх рельса над землёй.
pub(crate) const RAIL_TOP: f32 = 0.1;
/// Корень GLB путей стоит на y = −1.
const LIFT: f32 = 1.0;

/// Деталь пути Train Kit: начало в (0, 0), выход по +Z; зеркало — `scale.x = −1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    Straight,
    CornerLarge,
    CornerSmall,
    Curve,
}

const STRAIGHT: [Vec2; 2] = [vec2(0.0, 0.0), vec2(0.0, 4.0)];
const CORNER_LARGE: [Vec2; 14] = [
    vec2(0.0, 0.0),
    vec2(-0.0403, 0.4978),
    vec2(-0.1553, 0.9841),
    vec2(-0.3352, 1.4503),
    vec2(-0.5709, 1.8909),
    vec2(-0.8552, 2.3021),
    vec2(-1.1819, 2.6805),
    vec2(-1.5465, 3.0224),
    vec2(-1.9453, 3.3238),
    vec2(-2.3748, 3.5792),
    vec2(-2.8317, 3.7818),
    vec2(-3.3109, 3.9232),
    vec2(-3.8054, 3.9938),
    vec2(-4.0, 4.0),
];
const CORNER_SMALL: [Vec2; 8] = [
    vec2(0.0, 0.0),
    vec2(-0.0777, 0.4920),
    vec2(-0.2855, 0.9455),
    vec2(-0.5909, 1.3402),
    vec2(-0.9726, 1.6619),
    vec2(-1.4158, 1.8909),
    vec2(-1.9027, 1.9969),
    vec2(-2.0, 2.0),
];
const CURVE: [Vec2; 11] = [
    vec2(0.0, 0.0),
    vec2(-0.0465, 0.4982),
    vec2(-0.1990, 0.9727),
    vec2(-0.4557, 1.4002),
    vec2(-0.7819, 1.7788),
    vec2(-1.1335, 2.1342),
    vec2(-1.4698, 2.5040),
    vec2(-1.7473, 2.9186),
    vec2(-1.9268, 3.3836),
    vec2(-1.9974, 3.8771),
    vec2(-2.0, 4.0),
];

impl Piece {
    pub(crate) const ALL: [Self; 4] = [
        Self::Straight,
        Self::CornerLarge,
        Self::CornerSmall,
        Self::Curve,
    ];

    /// Подкаталог набора и имя GLB без расширения.
    pub(crate) fn model(self) -> &'static str {
        match self {
            Self::Straight => "train/railroad-straight",
            Self::CornerLarge => "train/railroad-corner-large",
            Self::CornerSmall => "train/railroad-corner-small",
            Self::Curve => "train/railroad-curve",
        }
    }

    /// Ось без зеркала — середины сечений головок рельсов меша: аналитические дуги расходятся с ним до 0,11.
    pub(crate) fn axis(self) -> &'static [Vec2] {
        match self {
            Self::Straight => &STRAIGHT,
            Self::CornerLarge => &CORNER_LARGE,
            Self::CornerSmall => &CORNER_SMALL,
            Self::Curve => &CURVE,
        }
    }

    /// Конец оси и касательная на выходе.
    pub(crate) fn end(self, mirror: bool) -> (Vec2, Vec2) {
        let axis = self.axis();
        (
            flip(axis[axis.len() - 1], mirror),
            flip(self.exit(), mirror),
        )
    }

    pub(crate) fn len(self) -> f32 {
        self.axis().windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Точка оси и касательная на расстоянии `s` в `[0, len]` от начала.
    pub(crate) fn point_at(self, mirror: bool, s: f32) -> (Vec2, Vec2) {
        let axis = self.axis();
        let last = axis.len() - 2;
        let mut rest = s;
        for i in 0..=last {
            let seg = axis[i].distance(axis[i + 1]);
            if rest <= seg || i == last {
                let t = rest / seg;
                let p = axis[i].lerp(axis[i + 1], t);
                let d = self.tangent(i).lerp(self.tangent(i + 1), t).normalize();
                return (flip(p, mirror), flip(d, mirror));
            }
            rest -= seg;
        }
        unreachable!("у оси хотя бы один отрезок")
    }

    /// Касательная в вершине оси: на концах — точная, внутри — по соседям.
    fn tangent(self, i: usize) -> Vec2 {
        let axis = self.axis();
        if i == 0 {
            Vec2::Y
        } else if i + 1 < axis.len() {
            (axis[i + 1] - axis[i - 1]).normalize()
        } else {
            self.exit()
        }
    }

    fn exit(self) -> Vec2 {
        match self {
            Self::Straight | Self::Curve => Vec2::Y,
            Self::CornerLarge | Self::CornerSmall => Vec2::NEG_X,
        }
    }
}

/// Модель детали в её осях: рельс на `RAIL_TOP`, ось по `axis`.
pub(crate) fn transform(mirror: bool) -> Mat4 {
    let x = if mirror { -1.0 } else { 1.0 };
    Mat4::from_translation(vec3(0.0, LIFT, 0.0)) * Mat4::from_scale(vec3(x, 1.0, 1.0))
}

fn flip(v: Vec2, mirror: bool) -> Vec2 {
    if mirror { vec2(-v.x, v.y) } else { v }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_1_SQRT_2, PI};

    use stapel::{Assets, Model};

    use super::*;

    const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");

    fn load(assets: &mut Assets, piece: Piece) -> &Model {
        let h = assets.load_model(format!("{ASSETS}{}.glb", piece.model()));
        assets.model(h)
    }

    fn near(a: Vec2, b: Vec2, eps: f32) -> bool {
        a.distance(b) < eps
    }

    #[test]
    fn rail_heads_run_along_axis() {
        let mut assets = Assets::default();
        let cases = Piece::ALL
            .map(|p| (p, false))
            .into_iter()
            .chain([(Piece::Curve, true)]);
        for (piece, mirror) in cases {
            let model = load(&mut assets, piece);
            let meshes: Vec<_> = model
                .nodes
                .iter()
                .flat_map(|n| n.primitives.iter().map(|p| p.mesh))
                .collect();
            let points: Vec<_> = meshes
                .iter()
                .flat_map(|&m| assets.mesh_geometry(m).positions.iter().copied())
                .collect();
            let top = points.iter().map(|p| p.y).fold(f32::MIN, f32::max);
            let heads: Vec<_> = points
                .iter()
                .filter(|p| p.y > top - 1e-4)
                .map(|p| flip(vec2(p.x, p.z), mirror))
                .collect();
            assert!(heads.len() >= 16, "{piece:?}: {}", heads.len());
            let samples: Vec<_> = (0..=1000)
                .map(|i| piece.point_at(mirror, piece.len() * i as f32 / 1000.0).0)
                .collect();
            for h in heads {
                let d = samples
                    .iter()
                    .map(|s| s.distance(h))
                    .fold(f32::MAX, f32::min);
                assert!((d - 0.30).abs() <= 0.06, "{piece:?} {mirror}: {h} в {d}");
            }
        }
    }

    #[test]
    fn ends_match_kit_table() {
        let table = [
            (Piece::Straight, vec2(0.0, 4.0), Vec2::Y),
            (Piece::CornerLarge, vec2(-4.0, 4.0), Vec2::NEG_X),
            (Piece::CornerSmall, vec2(-2.0, 2.0), Vec2::NEG_X),
            (Piece::Curve, vec2(-2.0, 4.0), Vec2::Y),
        ];
        for (piece, end, tangent) in table {
            assert_eq!(piece.end(false), (end, tangent), "{piece:?}");
            assert_eq!(
                piece.end(true),
                (vec2(-end.x, end.y), vec2(-tangent.x, tangent.y)),
                "{piece:?}"
            );
            assert_eq!(piece.axis()[0], Vec2::ZERO, "{piece:?}");
        }
    }

    #[test]
    fn len_sums_segments_close_to_arcs() {
        let arcs = [
            (Piece::Straight, 4.0),
            (Piece::CornerLarge, 2.0 * PI),
            (Piece::CornerSmall, PI),
            (Piece::Curve, 5.0 * 2.0 * 0.5f32.atan()),
        ];
        for (piece, arc) in arcs {
            let axis = piece.axis();
            let sum: f32 = (1..axis.len())
                .map(|i| (axis[i] - axis[i - 1]).length())
                .sum();
            assert!((piece.len() - sum).abs() < 1e-5, "{piece:?}");
            assert!(
                (piece.len() - arc).abs() < 0.1,
                "{piece:?}: {}",
                piece.len()
            );
        }
    }

    #[test]
    fn point_at_ends_and_middle() {
        let middles = [
            (Piece::Straight, vec2(0.0, 2.0), Vec2::Y),
            (
                Piece::CornerLarge,
                vec2(-4.0 + 4.0 * FRAC_1_SQRT_2, 4.0 * FRAC_1_SQRT_2),
                vec2(-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            ),
            (
                Piece::CornerSmall,
                vec2(-2.0 + 2.0 * FRAC_1_SQRT_2, 2.0 * FRAC_1_SQRT_2),
                vec2(-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            ),
            (
                Piece::Curve,
                vec2(-1.0, 2.0),
                vec2(-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
            ),
        ];
        for (piece, mid, tangent) in middles {
            let len = piece.len();
            assert_eq!(
                piece.point_at(false, 0.0),
                (Vec2::ZERO, Vec2::Y),
                "{piece:?}"
            );
            let (p, d) = piece.point_at(false, len);
            let (end, end_d) = piece.end(false);
            assert!(
                near(p, end, 1e-4) && near(d, end_d, 1e-4),
                "{piece:?}: {p} {d}"
            );
            let (p, d) = piece.point_at(false, len / 2.0);
            assert!(near(p, mid, 0.15), "{piece:?}: {p}");
            assert!(near(d, tangent, 0.1), "{piece:?}: {d}");
            assert!((d.length() - 1.0).abs() < 1e-5, "{piece:?}: {d}");
            for s in [0.3, len / 3.0, len * 0.9] {
                let (p, d) = piece.point_at(false, s);
                assert_eq!(piece.point_at(true, s), (flip(p, true), flip(d, true)));
                assert!(p.x != 0.0 || piece == Piece::Straight, "{piece:?}");
            }
        }
    }

    #[test]
    fn point_at_follows_axis_vertices() {
        for piece in Piece::ALL {
            let axis = piece.axis();
            let mut s = 0.0;
            for i in 1..axis.len() {
                s += axis[i].distance(axis[i - 1]);
                let (p, _) = piece.point_at(false, s);
                assert!(near(p, axis[i], 1e-4), "{piece:?} {i}: {p}");
                let (p, _) = piece.point_at(false, s - 0.05);
                let back = axis[i] - (axis[i] - axis[i - 1]).normalize() * 0.05;
                assert!(near(p, back, 1e-4), "{piece:?} {i}: {p}");
            }
        }
    }

    #[test]
    fn rail_top_after_transform() {
        let mut assets = Assets::default();
        for piece in Piece::ALL {
            for mirror in [false, true] {
                let b = load(&mut assets, piece)
                    .bounds()
                    .unwrap()
                    .transformed(transform(mirror));
                assert!((b.max.y - RAIL_TOP).abs() < 1e-4, "{piece:?}: {b:?}");
                assert!(b.min.y.abs() < 1e-4, "{piece:?}: {b:?}");
                if piece != Piece::Straight {
                    assert_eq!(b.max.x > 1.0, mirror, "{piece:?}: {b:?}");
                    assert_eq!(b.min.x < -1.0, !mirror, "{piece:?}: {b:?}");
                }
            }
        }
    }
}
