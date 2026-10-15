use engine::{Color, Gfx, Handle, IVec2, Sprite, Texture, Vec2, vec2};

use crate::map::Map;

/// Клетка открывается, когда её центр ближе к игроку, м.
pub(crate) const REVEAL: f32 = 2.5;
/// Сторона клетки на миникарте, единиц HUD.
pub(crate) const CELL: f32 = 10.0;
pub(crate) const ARROW: f32 = 14.0;
pub(crate) const CELL_LAYER: i32 = 1;
pub(crate) const ARROW_LAYER: i32 = 2;
pub(crate) const WALL: Color = Color {
    r: 0.45,
    g: 0.4,
    b: 0.35,
    a: 0.9,
};
pub(crate) const FLOOR: Color = Color {
    r: 0.03,
    g: 0.03,
    b: 0.04,
    a: 0.75,
};
pub(crate) const PLAYER: Color = Color {
    r: 1.0,
    g: 0.8,
    b: 0.1,
    a: 1.0,
};

/// Открытые клетки карты; однажды открытая не закрывается.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Minimap {
    size: IVec2,
    seen: Vec<bool>,
}

impl Minimap {
    pub(crate) fn new(map: &Map) -> Self {
        Self {
            size: map.size,
            seen: vec![false; (map.size.x * map.size.y).unsigned_abs() as usize],
        }
    }

    /// Открывает клетки карты в радиусе `REVEAL` вокруг `pos`.
    pub(crate) fn reveal(&mut self, pos: Vec2) {
        let min = (pos - REVEAL).floor().as_ivec2().max(IVec2::ZERO);
        let max = (pos + REVEAL).floor().as_ivec2().min(self.size - 1);
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let cell = IVec2::new(x, y);
                if pos.distance(cell.as_vec2() + 0.5) < REVEAL {
                    let i = self.index(cell);
                    self.seen[i] = true;
                }
            }
        }
    }

    pub(crate) fn seen(&self, cell: IVec2) -> bool {
        self.seen[self.index(cell)]
    }

    #[expect(
        clippy::cast_sign_loss,
        reason = "клетка внутри карты, координаты неотрицательны"
    )]
    fn index(&self, cell: IVec2) -> usize {
        (cell.y * self.size.x + cell.x) as usize
    }

    /// Открытые клетки с левым верхним углом карты в `origin` и игрок `pos` стрелкой по `yaw`.
    pub(crate) fn draw(
        &self,
        gfx: &mut Gfx,
        map: &Map,
        origin: Vec2,
        arrow: Handle<Texture>,
        pos: Vec2,
        yaw: f32,
    ) {
        let to_hud = |p: Vec2| origin + vec2(p.x, -p.y) * CELL;
        for cell in map.cells().filter(|&c| self.seen(c)) {
            gfx.draw_sprite(Sprite {
                pos: to_hud(cell.as_vec2() + 0.5),
                size: Vec2::splat(CELL),
                color: if map.wall(cell) { WALL } else { FLOOR },
                layer: CELL_LAYER,
                ..Default::default()
            });
        }
        // Yaw против часовой сверху, как поворот спрайта: стрелка текстуры смотрит вверх, на −Z.
        gfx.draw_sprite(Sprite {
            texture: arrow,
            pos: to_hud(pos),
            size: Vec2::splat(ARROW),
            rotation: yaw,
            color: PLAYER,
            layer: ARROW_LAYER,
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use engine::{Assets, ivec2};

    use super::*;

    fn map() -> Map {
        Map::parse("#######\n#P....#\n#.###.#\n#.....#\n#######\n").unwrap()
    }

    fn opened(m: &Minimap, map: &Map) -> Vec<IVec2> {
        map.cells().filter(|&c| m.seen(c)).collect()
    }

    #[test]
    fn reveal_opens_cells_within_radius() {
        let map = map();
        let mut m = Minimap::new(&map);
        assert_eq!(opened(&m, &map), []);
        let pos = vec2(1.5, 1.0);
        m.reveal(pos);
        let want: Vec<_> = (map.cells())
            .filter(|c| pos.distance(c.as_vec2() + 0.5) < REVEAL)
            .collect();
        assert_eq!(opened(&m, &map), want);
        assert!(m.seen(ivec2(3, 1)) && m.seen(ivec2(0, 0)));
        assert!(!m.seen(ivec2(3, 2)), "ровно 2,5 м — не открыта");
        assert!(!m.seen(ivec2(4, 1)));
    }

    #[test]
    fn reveal_matches_radius_near_map_corner() {
        let map = map();
        for pos in [
            vec2(0.5, 0.5),
            vec2(2.5, 1.5),
            vec2(5.5, 3.5),
            vec2(3.0, 2.0),
        ] {
            let mut m = Minimap::new(&map);
            m.reveal(pos);
            let want: Vec<_> = (map.cells())
                .filter(|c| pos.distance(c.as_vec2() + 0.5) < REVEAL)
                .collect();
            assert_eq!(opened(&m, &map), want, "{pos}");
        }
    }

    #[test]
    fn reveal_reaches_map_edges_and_keeps_opened() {
        let map = map();
        let mut m = Minimap::new(&map);
        m.reveal(vec2(5.5, 3.5));
        assert!(m.seen(ivec2(6, 4)), "угол карты");
        let before = opened(&m, &map);
        m.reveal(vec2(1.5, 1.5));
        let after = opened(&m, &map);
        assert!(before.iter().all(|c| after.contains(c)));
        assert!(after.len() > before.len());
    }

    #[test]
    fn reveal_far_outside_map_opens_nothing() {
        let map = map();
        let mut m = Minimap::new(&map);
        m.reveal(vec2(-10.0, -10.0));
        m.reveal(vec2(20.0, 20.0));
        assert_eq!(opened(&m, &map), []);
    }

    #[test]
    fn draw_shows_opened_walls_and_floor_and_player_arrow() {
        let map = map();
        let mut m = Minimap::new(&map);
        m.reveal(vec2(1.5, 1.5));
        let arrow = Assets::default().load_texture("arrow.png");
        let mut gfx = Gfx::default();
        let origin = vec2(100.0, 50.0);
        m.draw(&mut gfx, &map, origin, arrow, vec2(2.0, 1.25), 0.5);
        let (cells, rest): (Vec<&Sprite>, Vec<_>) =
            gfx.sprites.iter().partition(|s| s.layer == CELL_LAYER);
        assert_eq!(cells.len(), opened(&m, &map).len());
        for (s, cell) in cells.iter().zip(opened(&m, &map)) {
            let center = origin + vec2(cell.x as f32 + 0.5, -(cell.y as f32 + 0.5)) * CELL;
            assert_eq!(s.pos, center, "{cell}");
            assert_eq!(s.size, Vec2::splat(CELL));
            assert_eq!(s.texture, Handle::WHITE);
            assert_eq!(s.color, if map.wall(cell) { WALL } else { FLOOR }, "{cell}");
        }
        assert!(cells.iter().any(|s| s.color == WALL));
        assert!(cells.iter().any(|s| s.color == FLOOR));
        let [player] = rest[..] else {
            panic!("{rest:?}")
        };
        assert_eq!(player.texture, arrow);
        assert_eq!(player.pos, origin + vec2(20.0, -12.5));
        assert_eq!(player.rotation, 0.5);
        assert_eq!(player.layer, ARROW_LAYER);
        assert_eq!(player.color, PLAYER);
        assert_eq!(player.size, Vec2::splat(ARROW));
    }
}
