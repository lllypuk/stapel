use std::fmt;

use engine::{
    Aabb, Axis, Camera2D, Color, Config, Context, Game, Gfx, Handle, IVec2, Rect, Sprite, Texture,
    Vec2, ivec2, vec2,
};

const TICK_HZ: u32 = 60;
/// Сторона тайла в мировых пикселях.
const TILE: i32 = 18;
/// Видимая часть мира; чётная, поэтому края вида на целых пикселях при целом центре.
const VIEW: IVec2 = ivec2(320, 180);
/// Полуразмеры зоны вокруг центра камеры, внутри которой цель камеру не двигает.
const DEAD_ZONE: IVec2 = ivec2(24, 24);
const TILES_ATLAS: Vec2 = vec2(360.0, 162.0);
/// Ячейки атласа тайлов.
const GRASS: IVec2 = ivec2(2, 0);
const DIRT: IVec2 = ivec2(2, 6);
const TILE_LAYER: i32 = 1;
const LEVEL1: &str = include_str!("../assets/level1.txt");

/// Ошибка разбора карты; `line` — с единицы, первая строка — верх.
#[derive(Debug, PartialEq)]
enum LevelError {
    Ragged { line: usize },
    Unknown { line: usize, ch: char },
    NoPlayer,
    TwoPlayers { line: usize },
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ragged { line } => write!(f, "строка {line}: длина не как у первой"),
            Self::Unknown { line, ch } => write!(f, "строка {line}: неизвестный символ {ch:?}"),
            Self::NoPlayer => write!(f, "нет P"),
            Self::TwoPlayers { line } => write!(f, "строка {line}: второй P"),
        }
    }
}

/// Платформа из подряд идущих `=` (по X) или `H` (по Y); `cells` — в клетках.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlatformSpawn {
    cells: Aabb,
    axis: Axis,
}

/// Уровень из ASCII-карты; клетки — от левой нижней, Y вверх.
#[derive(Debug, PartialEq)]
struct Level {
    size: IVec2,
    /// По строкам снизу вверх.
    solid: Vec<bool>,
    player: IVec2,
    enemies: Vec<IVec2>,
    coins: Vec<IVec2>,
    spikes: Vec<IVec2>,
    exits: Vec<IVec2>,
    stops: Vec<IVec2>,
    platforms: Vec<PlatformSpawn>,
}

impl Level {
    fn parse(text: &str) -> Result<Self, LevelError> {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.first().map_or(0, Vec::len);
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        let size = ivec2(to_i32(width), to_i32(rows.len()));
        let mut level = Self {
            size,
            solid: vec![false; width * rows.len()],
            player: IVec2::ZERO,
            enemies: Vec::new(),
            coins: Vec::new(),
            spikes: Vec::new(),
            exits: Vec::new(),
            stops: Vec::new(),
            platforms: Vec::new(),
        };
        let mut player = None;
        for (i, row) in rows.iter().enumerate() {
            let line = i + 1;
            if row.len() != width {
                return Err(LevelError::Ragged { line });
            }
            let y = size.y - 1 - to_i32(i);
            for (j, &ch) in row.iter().enumerate() {
                let cell = ivec2(to_i32(j), y);
                match ch {
                    '#' => level.solid[to_index(cell, size.x)] = true,
                    'P' if player.is_some() => return Err(LevelError::TwoPlayers { line }),
                    'P' => player = Some(cell),
                    'E' => level.enemies.push(cell),
                    'o' => level.coins.push(cell),
                    '^' => level.spikes.push(cell),
                    'X' => level.exits.push(cell),
                    '!' => level.stops.push(cell),
                    '=' if j == 0 || row[j - 1] != '=' => {
                        let len = row[j..].iter().take_while(|&&c| c == '=').count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: cell,
                                size: ivec2(to_i32(len), 1),
                            },
                            axis: Axis::X,
                        });
                    }
                    'H' if i == 0 || rows[i - 1][j] != 'H' => {
                        let len = rows[i..]
                            .iter()
                            .take_while(|r| r.get(j) == Some(&'H'))
                            .count();
                        level.platforms.push(PlatformSpawn {
                            cells: Aabb {
                                min: ivec2(cell.x, y + 1 - to_i32(len)),
                                size: ivec2(1, to_i32(len)),
                            },
                            axis: Axis::Y,
                        });
                    }
                    ' ' | '.' | '=' | 'H' => {}
                    ch => return Err(LevelError::Unknown { line, ch }),
                }
            }
        }
        level.player = player.ok_or(LevelError::NoPlayer)?;
        Ok(level)
    }

    /// Вне карты — не твёрдое.
    fn solid_at(&self, cell: IVec2) -> bool {
        cell.cmpge(IVec2::ZERO).all()
            && cell.cmplt(self.size).all()
            && self.solid[to_index(cell, self.size.x)]
    }

    fn bounds(&self) -> IVec2 {
        self.size * TILE
    }
}

#[expect(
    clippy::cast_sign_loss,
    reason = "клетка внутри карты, координаты неотрицательны"
)]
fn to_index(cell: IVec2, width: i32) -> usize {
    (cell.y * width + cell.x) as usize
}

/// Центр клетки в мировых пикселях.
fn cell_center(cell: IVec2) -> Vec2 {
    (cell * TILE + TILE / 2).as_vec2()
}

/// UV ячейки `cell` атласа `atlas` с ячейками `size × size` без зазоров.
fn atlas_uv(cell: IVec2, size: i32, atlas: Vec2) -> Rect {
    let min = (cell * size).as_vec2() / atlas;
    Rect {
        min,
        max: min + Vec2::splat(size as f32) / atlas,
    }
}

/// Положение между тиками, округлённое до целого пикселя.
fn lerp_round(prev: IVec2, cur: IVec2, alpha: f32) -> Vec2 {
    prev.as_vec2().lerp(cur.as_vec2(), alpha).round()
}

/// Центр камеры в мировых пикселях на конец и начало тика.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Follow {
    pos: IVec2,
    prev: IVec2,
}

impl Follow {
    fn at(pos: IVec2) -> Self {
        Self { pos, prev: pos }
    }

    /// Держит `target` в мёртвой зоне, а вид — внутри `bounds`; уровень уже вида прижат к началу.
    fn follow(&mut self, target: IVec2, bounds: IVec2) {
        let pos = self.pos.clamp(target - DEAD_ZONE, target + DEAD_ZONE);
        self.pos = pos.min(bounds - VIEW / 2).max(VIEW / 2);
    }

    fn lerp(&self, alpha: f32) -> Vec2 {
        lerp_round(self.prev, self.pos, alpha)
    }
}

struct Platformer {
    level: Level,
    camera: Follow,
    tiles: Handle<Texture>,
}

impl Platformer {
    fn new(level: Level) -> Self {
        let mut camera = Follow::at(level.bounds() / 2);
        camera.follow(cell_center(level.player).as_ivec2(), level.bounds());
        camera.prev = camera.pos;
        Self {
            level,
            camera,
            tiles: Handle::WHITE,
        }
    }

    fn tick(&mut self) {
        self.camera.prev = self.camera.pos;
        let target = cell_center(self.level.player).as_ivec2();
        self.camera.follow(target, self.level.bounds());
    }

    /// Твёрдые клетки, задевающие вид с целым центром `center`; верхняя — трава.
    fn draw_tiles(&self, gfx: &mut Gfx, center: IVec2) {
        let lo = (center - VIEW / 2).div_euclid(IVec2::splat(TILE));
        let hi = (center + VIEW / 2 - 1).div_euclid(IVec2::splat(TILE));
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let cell = ivec2(x, y);
                if !self.level.solid_at(cell) {
                    continue;
                }
                let art = if self.level.solid_at(cell + IVec2::Y) {
                    DIRT
                } else {
                    GRASS
                };
                gfx.draw_sprite(Sprite {
                    texture: self.tiles,
                    pos: cell_center(cell),
                    size: Vec2::splat(TILE as f32),
                    uv: atlas_uv(art, TILE, TILES_ATLAS),
                    layer: TILE_LAYER,
                    ..Default::default()
                });
            }
        }
    }
}

impl Game for Platformer {
    fn init(ctx: &mut Context) -> Self {
        ctx.gfx.clear_color = Color::from_srgb8(185, 225, 245);
        let level = Level::parse(LEVEL1).unwrap_or_else(|e| panic!("level1.txt, {e}"));
        let mut game = Self::new(level);
        game.tiles = ctx.assets.load_texture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/tilemap_packed.png"
        ));
        game
    }

    fn update(&mut self, _: &mut Context) {
        self.tick();
    }

    fn draw(&mut self, ctx: &mut Context, alpha: f32) {
        let center = self.camera.lerp(alpha);
        ctx.gfx.camera = Camera2D {
            center,
            size: VIEW.as_vec2(),
        };
        self.draw_tiles(&mut ctx.gfx, center.as_ivec2());
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Platformer>(Config {
        title: "platformer".into(),
        size: (1280, 720),
        tick_hz: TICK_HZ,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Level {
        Level::parse(text).unwrap()
    }

    #[test]
    fn first_line_is_top_and_hash_is_solid() {
        let l = parse("#..\n.P#\n");
        assert_eq!((l.size, l.player), (ivec2(3, 2), ivec2(1, 0)));
        assert_eq!(l.solid, [false, false, true, true, false, false]);
        assert!(l.solid_at(ivec2(0, 1)) && l.solid_at(ivec2(2, 0)));
        assert!(!l.solid_at(ivec2(1, 0)));
    }

    #[test]
    fn solid_at_outside_map_is_empty() {
        let l = parse("###\n#P#\n");
        for cell in [ivec2(-1, 0), ivec2(3, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert!(!l.solid_at(cell), "{cell}");
        }
    }

    #[test]
    fn spawns_are_cells_from_bottom() {
        let l = parse("E o.^\nX!P  \n");
        assert_eq!(l.enemies, [ivec2(0, 1)]);
        assert_eq!(l.coins, [ivec2(2, 1)]);
        assert_eq!(l.spikes, [ivec2(4, 1)]);
        assert_eq!(l.exits, [ivec2(0, 0)]);
        assert_eq!(l.stops, [ivec2(1, 0)]);
        assert_eq!(l.player, ivec2(2, 0));
        assert!(l.solid.iter().all(|&s| !s));
    }

    #[test]
    fn runs_of_eq_and_h_are_one_platform_each() {
        let l = parse("==.=\n=..H\nP.HH\n");
        let p = |min, size, axis| PlatformSpawn {
            cells: Aabb { min, size },
            axis,
        };
        assert_eq!(
            l.platforms,
            [
                p(ivec2(0, 2), ivec2(2, 1), Axis::X),
                p(ivec2(3, 2), ivec2(1, 1), Axis::X),
                p(ivec2(0, 1), ivec2(1, 1), Axis::X),
                p(ivec2(3, 0), ivec2(1, 2), Axis::Y),
                p(ivec2(2, 0), ivec2(1, 1), Axis::Y),
            ]
        );
    }

    #[test]
    fn h_run_starts_at_top_row() {
        let l = parse("H\nH\nP\n");
        assert_eq!(l.platforms[0].cells.min, ivec2(0, 1));
        assert_eq!(l.platforms[0].cells.size, ivec2(1, 2));
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert_eq!(
            Level::parse("P..\n..\n"),
            Err(LevelError::Ragged { line: 2 })
        );
        assert_eq!(
            Level::parse("P..\n.?.\n"),
            Err(LevelError::Unknown { line: 2, ch: '?' })
        );
        assert_eq!(Level::parse("...\n###\n"), Err(LevelError::NoPlayer));
        assert_eq!(Level::parse(""), Err(LevelError::NoPlayer));
        assert_eq!(
            Level::parse("P..\n..P\n"),
            Err(LevelError::TwoPlayers { line: 2 })
        );
    }

    #[test]
    fn errors_print_line_and_cause() {
        let msg = |text| Level::parse(text).unwrap_err().to_string();
        assert_eq!(msg("P..\n..\n"), "строка 2: длина не как у первой");
        assert_eq!(msg("P..\n.?.\n"), "строка 2: неизвестный символ '?'");
        assert_eq!(msg(""), "нет P");
        assert_eq!(msg("P\nP\n"), "строка 2: второй P");
    }

    #[test]
    fn level1_is_at_least_four_screens_wide() {
        let l = parse(LEVEL1);
        assert!(l.bounds().x >= 4 * VIEW.x, "{}", l.bounds());
        assert!(l.bounds().y >= VIEW.y, "{}", l.bounds());
        assert!(!l.exits.is_empty() && !l.platforms.is_empty());
    }

    const BOUNDS: IVec2 = ivec2(1000, 400);

    #[test]
    fn camera_ignores_target_inside_dead_zone() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(500, 200) + DEAD_ZONE, BOUNDS);
        c.follow(ivec2(500, 200) - DEAD_ZONE, BOUNDS);
        assert_eq!(c.pos, ivec2(500, 200));
    }

    #[test]
    fn camera_drags_target_on_dead_zone_edge() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(530, 170), BOUNDS);
        assert_eq!(c.pos, ivec2(530, 170) - ivec2(DEAD_ZONE.x, -DEAD_ZONE.y));
        c.follow(ivec2(400, 250), BOUNDS);
        assert_eq!(c.pos, ivec2(400, 250) + ivec2(DEAD_ZONE.x, -DEAD_ZONE.y));
    }

    #[test]
    fn camera_view_stays_inside_level() {
        let mut c = Follow::at(ivec2(500, 200));
        c.follow(ivec2(0, 0), BOUNDS);
        assert_eq!(c.pos, VIEW / 2);
        c.follow(BOUNDS, BOUNDS);
        assert_eq!(c.pos, BOUNDS - VIEW / 2);
        let mut small = Follow::at(ivec2(500, 200));
        small.follow(ivec2(500, 200), ivec2(100, 100));
        assert_eq!(small.pos, VIEW / 2);
    }

    #[test]
    fn camera_lerp_rounds_to_whole_pixels() {
        let c = Follow {
            pos: ivec2(3, -3),
            prev: IVec2::ZERO,
        };
        assert_eq!(c.lerp(0.0), Vec2::ZERO);
        assert_eq!(c.lerp(0.3), vec2(1.0, -1.0));
        assert_eq!(c.lerp(0.5), vec2(2.0, -2.0));
        assert_eq!(c.lerp(1.0), vec2(3.0, -3.0));
    }

    fn started() -> (Platformer, Context) {
        let mut ctx = Context::default();
        let g = Platformer::init(&mut ctx);
        (g, ctx)
    }

    #[test]
    fn init_puts_spawn_in_view_and_loads_tiles() {
        let (g, mut ctx) = started();
        let pending = ctx.assets.take_pending();
        let tiles = pending.iter().find(|p| p.handle == g.tiles).unwrap();
        assert_eq!(vec2(tiles.width as f32, tiles.height as f32), TILES_ATLAS);
        let spawn = cell_center(g.level.player).as_ivec2();
        assert!((spawn - g.camera.pos).abs().cmplt(VIEW / 2).all());
        assert_eq!(g.camera.pos, VIEW / 2);
        assert_eq!(g.camera.prev, g.camera.pos);
        assert_ne!(g.tiles, Handle::WHITE);
        assert_ne!(ctx.gfx.clear_color, Color::default());
    }

    #[test]
    fn new_camera_starts_still_on_spawn() {
        let g = Platformer::new(parse(&format!("{}P{}\n", ".".repeat(50), ".".repeat(149))));
        assert_eq!(g.camera.pos, ivec2(909 + DEAD_ZONE.x, VIEW.y / 2));
        assert_eq!(g.camera.prev, g.camera.pos);
    }

    #[test]
    fn update_saves_prev_then_follows() {
        let (mut g, mut ctx) = started();
        g.camera.pos = ivec2(900, 300);
        g.update(&mut ctx);
        assert_eq!(g.camera.prev, ivec2(900, 300));
        assert_eq!(g.camera.pos, Platformer::new(parse(LEVEL1)).camera.pos);
    }

    fn tile_sprites(gfx: &Gfx) -> Vec<Sprite> {
        gfx.sprites
            .iter()
            .filter(|s| s.layer == TILE_LAYER)
            .copied()
            .collect()
    }

    /// 40×12 клеток: пол в две строки и колонна из трёх клеток у `x = 20`.
    fn strip() -> Platformer {
        let mut rows = vec![".".repeat(40); 12];
        rows[9].replace_range(20..21, "#");
        rows[8].replace_range(20..21, "#");
        rows[7].replace_range(20..21, "#");
        rows[10] = "#".repeat(40);
        rows[11] = "#".repeat(40);
        rows[0].replace_range(0..1, "P");
        let mut g = Platformer::new(parse(&rows.join("\n")));
        g.tiles = Handle::WHITE;
        g
    }

    #[test]
    fn draws_only_tiles_touching_view() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(169, 90));
        g.draw_tiles(&mut gfx, g.camera.pos);
        let xs: Vec<f32> = tile_sprites(&gfx).iter().map(|s| s.pos.x).collect();
        // Вид [9, 329): клетки 0..=18 по X, пол — две строки.
        assert_eq!(xs.len(), 2 * 19);
        assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), 9.0);
        assert_eq!(
            xs.iter().copied().fold(f32::MIN, f32::max),
            18.0 * 18.0 + 9.0
        );
    }

    #[test]
    fn view_edge_on_tile_border_skips_next_tile() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(200, 125));
        g.draw_tiles(&mut gfx, g.camera.pos);
        // Вид [40, 360) × [35, 215): клетки 2..=19 верхней строки пола, колонна у x = 20 — нет.
        let ys: Vec<f32> = tile_sprites(&gfx).iter().map(|s| s.pos.y).collect();
        assert_eq!(ys, [27.0; 18]);
    }

    #[test]
    fn top_tile_is_grass_and_below_is_dirt() {
        let mut g = strip();
        let mut gfx = Gfx::default();
        g.camera = Follow::at(ivec2(360, 90));
        g.draw_tiles(&mut gfx, g.camera.pos);
        let uv_at = |pos: Vec2| {
            tile_sprites(&gfx)
                .into_iter()
                .find(|s| s.pos == pos)
                .unwrap()
                .uv
        };
        let grass = atlas_uv(GRASS, TILE, TILES_ATLAS);
        let dirt = atlas_uv(DIRT, TILE, TILES_ATLAS);
        assert_eq!(uv_at(cell_center(ivec2(20, 4))), grass);
        assert_eq!(uv_at(cell_center(ivec2(20, 3))), dirt);
        assert_eq!(uv_at(cell_center(ivec2(20, 1))), dirt);
        assert_eq!(uv_at(cell_center(ivec2(19, 1))), grass);
        assert_eq!(uv_at(cell_center(ivec2(19, 0))), dirt);
    }

    #[test]
    fn atlas_uv_covers_one_cell() {
        let uv = atlas_uv(ivec2(2, 6), TILE, TILES_ATLAS);
        assert_eq!(uv.min, vec2(36.0 / 360.0, 108.0 / 162.0));
        assert_eq!(uv.max, vec2(54.0 / 360.0, 126.0 / 162.0));
        let chars = atlas_uv(ivec2(1, 2), 24, vec2(216.0, 72.0));
        assert_eq!(chars.min, vec2(24.0 / 216.0, 48.0 / 72.0));
        assert_eq!(chars.max, vec2(48.0 / 216.0, 1.0));
    }

    #[test]
    fn draw_sets_camera_and_tile_edges_on_whole_pixels() {
        let (mut g, mut ctx) = started();
        g.camera = Follow {
            pos: ivec2(400, 100),
            prev: ivec2(397, 99),
        };
        g.draw(&mut ctx, 0.4);
        assert_eq!(ctx.gfx.camera.center, vec2(398.0, 99.0));
        assert_eq!(ctx.gfx.camera.size, VIEW.as_vec2());
        let tiles = tile_sprites(&ctx.gfx);
        assert_ne!(tiles.len(), 0);
        for s in tiles {
            let edge = s.pos - s.size / 2.0;
            assert_eq!(edge, edge.round(), "{s:?}");
            assert_eq!(s.size, Vec2::splat(18.0));
            assert_eq!(s.texture, g.tiles);
        }
    }
}
