use std::{collections::VecDeque, iter, path::PathBuf};

use stapel::{Color, Gfx, Handle, IVec2, Input, MouseButton, Sprite, Texture, Vec2, ivec2, vec2};

use crate::{
    BLOCK, COIN, DIRT, ENEMY_LAYER, FLAG, GRASS, IDLE, ITEM_LAYER, PLANK, PLATFORM_LAYER,
    PLAYER_LAYER, SPIKES, TILE, TILE_LAYER, WALKER, cell_center, char_sprite, tile_sprite,
};

/// Символ пустой клетки: им дополняются строки и заполняются новые столбцы и строки.
const EMPTY: char = '.';

/// Снимков отмены не больше; старший вытесняется.
const HISTORY_LIMIT: usize = 100;

/// Подсветка клетки под курсором и рамка уровня — поверх уровня.
pub(crate) const OVERLAY_LAYER: i32 = 7;
const HOVER: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.35,
};
const FRAME: Color = Color {
    r: 0.1,
    g: 0.15,
    b: 0.25,
    a: 1.0,
};
/// Невидимая в игре клетка разворота платформ `!`.
const STOP: Color = Color {
    r: 1.0,
    g: 0.5,
    b: 0.0,
    a: 1.0,
};
/// Символ не из палитры.
const UNKNOWN: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

/// Карта уровня как есть, без разбора; клетки — от левой нижней, Y вверх, как у `Level`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Grid {
    width: usize,
    /// По строкам сверху вниз, как в тексте.
    cells: Vec<char>,
}

impl Grid {
    /// Не падает: строки дополняются `EMPTY` до самой длинной, пустой текст — 1×1.
    pub(crate) fn parse(text: &str) -> Self {
        let rows: Vec<Vec<char>> = text.lines().map(|l| l.chars().collect()).collect();
        let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let mut cells = vec![EMPTY; width * rows.len().max(1)];
        for (row, line) in cells.chunks_mut(width).zip(&rows) {
            row[..line.len()].copy_from_slice(line);
        }
        Self { width, cells }
    }

    pub(crate) fn to_text(&self) -> String {
        self.cells
            .chunks(self.width)
            .flat_map(|row| row.iter().copied().chain(iter::once('\n')))
            .collect()
    }

    pub(crate) fn size(&self) -> IVec2 {
        let to_i32 = |n: usize| i32::try_from(n).expect("карта меньше 2³¹ клеток");
        ivec2(to_i32(self.width), to_i32(self.cells.len() / self.width))
    }

    pub(crate) fn get(&self, cell: IVec2) -> Option<char> {
        self.index(cell).map(|i| self.cells[i])
    }

    /// Вне сетки — без изменений; `P` единственный, прежний стирается.
    pub(crate) fn set(&mut self, cell: IVec2, ch: char) {
        let Some(i) = self.index(cell) else { return };
        if ch == 'P' {
            for c in self.cells.iter_mut().filter(|c| **c == 'P') {
                *c = EMPTY;
            }
        }
        self.cells[i] = ch;
    }

    pub(crate) fn add_column(&mut self) {
        self.cells = self
            .cells
            .chunks(self.width)
            .flat_map(|row| row.iter().copied().chain(iter::once(EMPTY)))
            .collect();
        self.width += 1;
    }

    /// Последний столбец сетки не удаляется.
    pub(crate) fn remove_column(&mut self) {
        if self.width == 1 {
            return;
        }
        self.cells = self
            .cells
            .chunks(self.width)
            .flat_map(|row| row[..row.len() - 1].iter().copied())
            .collect();
        self.width -= 1;
    }

    /// Сверху: клетки снизу сохраняют координаты.
    pub(crate) fn add_row(&mut self) {
        self.cells.splice(0..0, iter::repeat_n(EMPTY, self.width));
    }

    /// Сверху; последняя строка сетки не удаляется.
    pub(crate) fn remove_row(&mut self) {
        if self.cells.len() > self.width {
            self.cells.drain(..self.width);
        }
    }

    /// Спрайт символа клетки как в игре; пусто — `None`, символ не из палитры — маркер.
    fn sprite(
        &self,
        cell: IVec2,
        tiles: Handle<Texture>,
        chars: Handle<Texture>,
    ) -> Option<Sprite> {
        let tile = |art, layer| Some(tile_sprite(tiles, cell_center(cell), art, layer));
        let actor = |art, layer| {
            Some(char_sprite(
                chars,
                (cell * TILE).as_vec2(),
                TILE,
                art,
                1,
                layer,
            ))
        };
        let mark = |color, side: i32| {
            Some(Sprite {
                pos: cell_center(cell),
                size: Vec2::splat(side as f32),
                color,
                layer: ITEM_LAYER,
                ..Default::default()
            })
        };
        match self.get(cell)? {
            EMPTY | ' ' => None,
            '#' if self.get(cell + IVec2::Y) == Some('#') => tile(DIRT, TILE_LAYER),
            '#' => tile(GRASS, TILE_LAYER),
            'P' => actor(IDLE, PLAYER_LAYER),
            'E' => actor(WALKER, ENEMY_LAYER),
            'o' => tile(COIN, ITEM_LAYER),
            '^' => tile(SPIKES, ITEM_LAYER),
            'X' => tile(FLAG, ITEM_LAYER),
            '=' => tile(PLANK, PLATFORM_LAYER),
            'H' => tile(BLOCK, PLATFORM_LAYER),
            '!' => mark(STOP, TILE / 2),
            _ => mark(UNKNOWN, TILE),
        }
    }

    fn index(&self, cell: IVec2) -> Option<usize> {
        let size = self.size();
        if cell.cmplt(IVec2::ZERO).any() || cell.cmpge(size).any() {
            return None;
        }
        usize::try_from((size.y - 1 - cell.y) * size.x + cell.x).ok()
    }
}

/// Снимки сетки до правок и сохранённая сетка.
pub(crate) struct History {
    undo: VecDeque<Grid>,
    saved: Grid,
}

impl History {
    /// Пустая история уровня, на диске — `saved`.
    pub(crate) fn new(saved: Grid) -> Self {
        Self {
            undo: VecDeque::new(),
            saved,
        }
    }

    /// `before` — сетка до мазка или resize; правка без изменений снимка не даёт.
    pub(crate) fn commit(&mut self, before: Grid, after: &Grid) {
        if before == *after {
            return;
        }
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(before);
    }

    /// Пустая история — без изменений.
    pub(crate) fn undo(&mut self, grid: &mut Grid) {
        if let Some(before) = self.undo.pop_back() {
            *grid = before;
        }
    }

    pub(crate) fn saved(&mut self, grid: &Grid) {
        self.saved = grid.clone();
    }

    pub(crate) fn dirty(&self, grid: &Grid) -> bool {
        *grid != self.saved
    }
}

/// Изменение размера сетки: столбец справа, строка сверху.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Resize {
    AddColumn,
    RemoveColumn,
    AddRow,
    RemoveRow,
}

/// Команда панели редактора; разбирает следующий `update`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EditorCmd {
    Pick(char),
    Open(PathBuf),
    New,
    Resize(Resize),
    Save,
    Undo,
    LoadFromDisk,
}

/// Кнопка мазка и сетка до него.
struct Stroke {
    button: MouseButton,
    before: Grid,
}

/// Правка сетки уровня: кисть, история, ошибка разбора.
pub(crate) struct Editor {
    pub(crate) grid: Grid,
    history: History,
    /// Символ ЛКМ; ПКМ стирает.
    pub(crate) brush: char,
    stroke: Option<Stroke>,
    /// Почему сетку нельзя играть.
    pub(crate) error: Option<String>,
    /// Файл изменился на диске при `dirty`; держится до загрузки с диска или сохранения.
    pub(crate) changed_on_disk: bool,
    /// Последняя ошибка записи или чтения файла уровня или отказ сменить уровень.
    pub(crate) file_error: Option<String>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new("")
    }
}

impl Editor {
    /// Чистый редактор уровня с текстом `text` на диске.
    pub(crate) fn new(text: &str) -> Self {
        let grid = Grid::parse(text);
        Self {
            history: History::new(grid.clone()),
            grid,
            brush: '#',
            stroke: None,
            error: None,
            changed_on_disk: false,
            file_error: None,
        }
    }

    /// Сетка — `text` с диска; остаётся только кисть.
    pub(crate) fn replace(&mut self, text: &str) {
        *self = Self {
            brush: self.brush,
            ..Self::new(text)
        };
    }

    pub(crate) fn dirty(&self) -> bool {
        self.history.dirty(&self.grid)
    }

    /// Сетка записана в файл.
    pub(crate) fn saved(&mut self) {
        self.history.saved(&self.grid);
        self.changed_on_disk = false;
        self.file_error = None;
    }

    /// Начатый мазок отменяется целиком.
    pub(crate) fn undo(&mut self) {
        self.end_stroke();
        self.history.undo(&mut self.grid);
    }

    /// Начатый мазок кончается; размер — одним снимком в историю.
    pub(crate) fn resize(&mut self, resize: Resize) {
        self.end_stroke();
        let before = self.grid.clone();
        match resize {
            Resize::AddColumn => self.grid.add_column(),
            Resize::RemoveColumn => self.grid.remove_column(),
            Resize::AddRow => self.grid.add_row(),
            Resize::RemoveRow => self.grid.remove_row(),
        }
        self.history.commit(before, &self.grid);
    }

    /// Мазок по снимку тика: красит `cell` каждый тик, пока кнопка `down`.
    /// Конец — по `!down`, не по отпусканию: клик между тиками красит одну клетку.
    pub(crate) fn paint(&mut self, input: &Input, cell: Option<IVec2>) {
        if self.stroke.is_some() {
            self.stroke_tick(input, cell);
        }
        if self.stroke.is_none() {
            self.stroke = [MouseButton::Left, MouseButton::Right]
                .into_iter()
                .find(|&b| input.mouse_just_pressed(b))
                .map(|button| Stroke {
                    button,
                    before: self.grid.clone(),
                });
            self.stroke_tick(input, cell);
        }
    }

    fn stroke_tick(&mut self, input: &Input, cell: Option<IVec2>) {
        let Some(stroke) = &self.stroke else { return };
        let ch = if stroke.button == MouseButton::Left {
            self.brush
        } else {
            EMPTY
        };
        if let Some(cell) = cell {
            self.grid.set(cell, ch);
        }
        if !input.mouse_down(stroke.button) {
            self.end_stroke();
        }
    }

    /// Мазок — в историю одним снимком.
    pub(crate) fn end_stroke(&mut self) {
        if let Some(s) = self.stroke.take() {
            self.history.commit(s.before, &self.grid);
        }
    }

    /// Сетка, подсветка клетки `hover`, если она в сетке, и рамка снаружи уровня.
    pub(crate) fn draw(
        &self,
        gfx: &mut Gfx,
        tiles: Handle<Texture>,
        chars: Handle<Texture>,
        hover: Option<IVec2>,
    ) {
        let size = self.grid.size();
        for y in 0..size.y {
            for x in 0..size.x {
                if let Some(s) = self.grid.sprite(ivec2(x, y), tiles, chars) {
                    gfx.draw_sprite(s);
                }
            }
        }
        let mut overlay = |pos, size, color| {
            gfx.draw_sprite(Sprite {
                pos,
                size,
                color,
                layer: OVERLAY_LAYER,
                ..Default::default()
            });
        };
        if let Some(cell) = hover.filter(|&c| self.grid.get(c).is_some()) {
            overlay(cell_center(cell), Vec2::splat(TILE as f32), HOVER);
        }
        let bounds = (size * TILE).as_vec2();
        let (across, along) = (vec2(bounds.x + 2.0, 1.0), vec2(1.0, bounds.y + 2.0));
        overlay(vec2(bounds.x / 2.0, -0.5), across, FRAME);
        overlay(vec2(bounds.x / 2.0, bounds.y + 0.5), across, FRAME);
        overlay(vec2(-0.5, bounds.y / 2.0), along, FRAME);
        overlay(vec2(bounds.x + 0.5, bounds.y / 2.0), along, FRAME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LEVEL01;

    use stapel::{
        Assets,
        ElementState::{Pressed, Released},
    };

    /// Разные хэндлы атласов тайлов и персонажей; файлов нет — обе текстуры шахматные.
    fn atlases() -> (Handle<Texture>, Handle<Texture>) {
        let mut a = Assets::default();
        (a.load_texture("tiles"), a.load_texture("chars"))
    }

    /// Спрайты `Editor::draw` сетки `text`.
    fn drawn(text: &str, hover: Option<IVec2>) -> Vec<Sprite> {
        let (tiles, chars) = atlases();
        let mut gfx = Gfx::default();
        Editor::new(text).draw(&mut gfx, tiles, chars, hover);
        gfx.sprites
    }

    /// Спрайты клеток, без подсветки и рамки.
    fn cells(text: &str) -> Vec<Sprite> {
        let mut s = drawn(text, None);
        s.retain(|s| s.layer != OVERLAY_LAYER);
        s
    }

    fn overlay(text: &str, hover: Option<IVec2>, color: Color) -> Vec<Sprite> {
        let mut s = drawn(text, hover);
        s.retain(|s| s.layer == OVERLAY_LAYER && s.color == color);
        s
    }

    #[test]
    fn palette_draws_game_art() {
        let (tiles, chars) = atlases();
        let at = Vec2::splat(TILE as f32 / 2.0);
        let tile = |art, layer| tile_sprite(tiles, at, art, layer);
        let actor = |art, layer| char_sprite(chars, Vec2::ZERO, TILE, art, 1, layer);
        let stop = Sprite {
            pos: at,
            size: Vec2::splat(9.0),
            color: STOP,
            layer: ITEM_LAYER,
            ..Default::default()
        };
        let cases = [
            ('#', tile(GRASS, TILE_LAYER)),
            ('P', actor(IDLE, PLAYER_LAYER)),
            ('E', actor(WALKER, ENEMY_LAYER)),
            ('o', tile(COIN, ITEM_LAYER)),
            ('^', tile(SPIKES, ITEM_LAYER)),
            ('X', tile(FLAG, ITEM_LAYER)),
            ('=', tile(PLANK, PLATFORM_LAYER)),
            ('H', tile(BLOCK, PLATFORM_LAYER)),
            ('!', stop),
        ];
        for (ch, sprite) in cases {
            assert_eq!(cells(&format!("{ch}\n")), [sprite], "{ch}");
        }
        for ch in ['.', ' '] {
            assert_eq!(cells(&format!("{ch}\n")), [], "{ch:?}");
        }
    }

    #[test]
    fn ground_under_ground_is_dirt() {
        let (tiles, _) = atlases();
        let top = tile_sprite(tiles, cell_center(ivec2(0, 1)), GRASS, TILE_LAYER);
        let below = tile_sprite(tiles, cell_center(ivec2(0, 0)), DIRT, TILE_LAYER);
        assert_eq!(cells("#\n#\n"), [below, top]);
    }

    #[test]
    fn actor_stands_in_its_cell() {
        let (_, chars) = atlases();
        let at = Vec2::splat(TILE as f32);
        let player = char_sprite(chars, at, TILE, IDLE, 1, PLAYER_LAYER);
        assert_eq!(cells(".P\n..\n"), [player]);
    }

    #[test]
    fn unknown_symbol_draws_marker() {
        let marker = Sprite {
            pos: Vec2::splat(9.0),
            size: Vec2::splat(18.0),
            color: UNKNOWN,
            layer: ITEM_LAYER,
            ..Default::default()
        };
        assert_eq!(cells("?\n"), [marker]);
    }

    #[test]
    fn hover_highlights_cell_inside_grid() {
        let [lit] = overlay("..\n..\n", Some(ivec2(1, 0)), HOVER)[..] else {
            panic!("одна подсветка");
        };
        assert_eq!((lit.pos, lit.size), (vec2(27.0, 9.0), Vec2::splat(18.0)));
        for hover in [None, Some(ivec2(2, 0)), Some(ivec2(0, -1))] {
            assert_eq!(overlay("..\n..\n", hover, HOVER), [], "{hover:?}");
        }
    }

    #[test]
    fn frame_surrounds_level_from_outside() {
        let mut rects: Vec<_> = overlay("...\n...\n", None, FRAME)
            .iter()
            .map(|s| {
                let (min, max) = (s.pos - s.size / 2.0, s.pos + s.size / 2.0);
                [min.x, min.y, max.x, max.y]
            })
            .collect();
        rects.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let expected = [
            [-1.0, -1.0, 0.0, 37.0],
            [-1.0, -1.0, 55.0, 0.0],
            [-1.0, 36.0, 55.0, 37.0],
            [54.0, -1.0, 55.0, 37.0],
        ];
        assert_eq!(rects, expected);
    }

    /// Кнопки, нажатые с прошлого тика; `released` уже отпущены.
    fn clicked(pressed: &[MouseButton], released: &[MouseButton]) -> Input {
        let mut i = Input::default();
        for &b in pressed {
            i.on_button(b, Pressed);
        }
        for &b in released {
            i.on_button(b, Released);
        }
        i
    }

    #[test]
    fn click_between_ticks_paints_one_cell_with_one_snapshot() {
        let mut e = Editor::new("...\n");
        e.brush = 'o';
        let click = clicked(&[MouseButton::Left], &[MouseButton::Left]);
        e.paint(&click, Some(ivec2(1, 0)));
        assert_eq!(e.grid.to_text(), ".o.\n");
        e.paint(&Input::default(), Some(ivec2(2, 0)));
        assert_eq!(e.grid.to_text(), ".o.\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "...\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "...\n");
    }

    #[test]
    fn stroke_paints_each_tick_until_button_is_up() {
        let mut e = Editor::new("....\n");
        let mut held = clicked(&[MouseButton::Left], &[]);
        e.paint(&held, Some(ivec2(0, 0)));
        held.end_tick();
        e.paint(&held, None);
        e.paint(&held, Some(ivec2(1, 0)));
        e.paint(&clicked(&[], &[MouseButton::Left]), Some(ivec2(2, 0)));
        e.paint(&Input::default(), Some(ivec2(3, 0)));
        assert_eq!(e.grid.to_text(), "###.\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "....\n");
    }

    #[test]
    fn right_button_erases_and_left_wins_when_both() {
        let mut e = Editor::new("##\n");
        e.brush = 'o';
        e.paint(
            &clicked(&[MouseButton::Right], &[MouseButton::Right]),
            Some(ivec2(0, 0)),
        );
        assert_eq!(e.grid.to_text(), ".#\n");
        let both = clicked(&[MouseButton::Left, MouseButton::Right], &[]);
        e.paint(&both, Some(ivec2(1, 0)));
        assert_eq!(e.grid.to_text(), ".o\n");
    }

    #[test]
    fn press_in_tick_that_ends_stroke_starts_next() {
        let mut e = Editor::new("...\n");
        let mut i = clicked(&[MouseButton::Left], &[]);
        e.paint(&i, Some(ivec2(0, 0)));
        i.end_tick();
        i.on_button(MouseButton::Left, Released);
        i.on_button(MouseButton::Right, Pressed);
        e.paint(&i, Some(ivec2(1, 0)));
        assert_eq!(e.grid.to_text(), "#..\n");
        i.end_tick();
        e.paint(&i, Some(ivec2(2, 0)));
        assert_eq!(e.grid.to_text(), "#..\n");
        i.on_button(MouseButton::Right, Released);
        e.paint(&i, Some(ivec2(0, 0)));
        assert_eq!(e.grid.to_text(), "...\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "##.\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "...\n");
    }

    #[test]
    fn end_stroke_commits_held_stroke() {
        let mut e = Editor::new("..\n");
        e.paint(&clicked(&[MouseButton::Left], &[]), Some(ivec2(0, 0)));
        e.end_stroke();
        let mut idle = clicked(&[MouseButton::Left], &[]);
        idle.end_tick();
        e.paint(&idle, Some(ivec2(1, 0)));
        assert_eq!(e.grid.to_text(), "#.\n");
        e.history.undo(&mut e.grid);
        assert_eq!(e.grid.to_text(), "..\n");
    }

    #[test]
    fn undo_ends_stroke_and_reverts_it() {
        let mut e = Editor::new("..\n");
        let mut held = clicked(&[MouseButton::Left], &[]);
        e.paint(&held, Some(ivec2(0, 0)));
        held.end_tick();
        e.undo();
        assert_eq!(e.grid.to_text(), "..\n");
        e.paint(&held, Some(ivec2(1, 0)));
        assert_eq!(e.grid.to_text(), "..\n");
    }

    #[test]
    fn saved_clears_dirty_and_file_flags() {
        let mut e = Editor::new("..\n");
        e.grid.set(ivec2(0, 0), '#');
        (e.changed_on_disk, e.file_error) = (true, Some("err".into()));
        assert!(e.dirty());
        e.saved();
        assert!(!e.dirty() && !e.changed_on_disk && e.file_error.is_none());
    }

    #[test]
    fn resize_ends_stroke_and_undoes_in_one_step() {
        let mut e = Editor::new("..\n");
        let mut held = clicked(&[MouseButton::Left], &[]);
        e.paint(&held, Some(ivec2(0, 0)));
        e.resize(Resize::AddColumn);
        held.end_tick();
        e.paint(&held, Some(ivec2(2, 0)));
        assert_eq!(e.grid.to_text(), "#..\n");
        e.resize(Resize::AddRow);
        e.resize(Resize::RemoveRow);
        e.resize(Resize::RemoveColumn);
        assert_eq!(e.grid.to_text(), "#.\n");
        e.resize(Resize::RemoveRow);
        for text in ["#..\n", "...\n#..\n", "#..\n", "#.\n", "..\n", "..\n"] {
            e.undo();
            assert_eq!(e.grid.to_text(), text);
        }
    }

    #[test]
    fn replace_keeps_only_brush() {
        let mut e = Editor::new("..\n");
        e.brush = 'o';
        e.paint(&clicked(&[MouseButton::Left], &[]), Some(ivec2(0, 0)));
        e.error = Some("err".into());
        (e.changed_on_disk, e.file_error) = (true, Some("err".into()));
        e.replace("#\n");
        assert_eq!(e.grid.to_text(), "#\n");
        assert_eq!(e.brush, 'o');
        assert!(!e.dirty() && !e.changed_on_disk);
        assert!(e.error.is_none() && e.file_error.is_none() && e.stroke.is_none());
        e.undo();
        assert_eq!(e.grid.to_text(), "#\n");
    }

    #[test]
    fn level01_round_trips() {
        let g = Grid::parse(LEVEL01);
        assert_eq!(g.to_text(), LEVEL01);
        assert_eq!(g.size(), ivec2(80, 12));
    }

    #[test]
    fn ragged_rows_are_padded_with_empty() {
        let g = Grid::parse("P\n#.#\n##");
        assert_eq!(g.size(), ivec2(3, 3));
        assert_eq!(g.to_text(), "P..\n#.#\n##.\n");
    }

    #[test]
    fn cells_count_from_bottom_left() {
        let g = Grid::parse("ab\ncd\nef\n");
        assert_eq!(g.get(ivec2(0, 0)), Some('e'));
        assert_eq!(g.get(ivec2(1, 0)), Some('f'));
        assert_eq!(g.get(ivec2(0, 2)), Some('a'));
        assert_eq!(g.get(ivec2(1, 1)), Some('d'));
    }

    #[test]
    fn outside_grid_is_unchanged() {
        let mut g = Grid::parse("ab\ncd\n");
        for cell in [ivec2(-1, 0), ivec2(2, 0), ivec2(0, -1), ivec2(0, 2)] {
            assert_eq!(g.get(cell), None, "{cell}");
            g.set(cell, '#');
        }
        assert_eq!(g.to_text(), "ab\ncd\n");
    }

    #[test]
    fn second_player_erases_first() {
        let mut g = Grid::parse("P.\n.#\n");
        g.set(ivec2(1, 0), 'P');
        assert_eq!(g.to_text(), "..\n.P\n");
        g.set(ivec2(0, 0), '#');
        assert_eq!(g.to_text(), "..\n#P\n");
    }

    #[test]
    fn columns_change_on_the_right() {
        let mut g = Grid::parse("ab\ncd\n");
        g.add_column();
        assert_eq!(g.to_text(), "ab.\ncd.\n");
        g.remove_column();
        g.remove_column();
        assert_eq!(g.to_text(), "a\nc\n");
        assert_eq!(g.size(), ivec2(1, 2));
    }

    #[test]
    fn rows_change_on_top_and_keep_cells_below() {
        let mut g = Grid::parse("ab\ncd\n");
        g.add_row();
        assert_eq!(g.to_text(), "..\nab\ncd\n");
        assert_eq!(g.get(ivec2(0, 0)), Some('c'));
        g.remove_row();
        g.remove_row();
        assert_eq!(g.to_text(), "cd\n");
        assert_eq!(g.get(ivec2(0, 0)), Some('c'));
    }

    #[test]
    fn grid_is_at_least_one_by_one() {
        let mut g = Grid::parse("");
        assert_eq!(g.to_text(), ".\n");
        g.remove_column();
        g.remove_row();
        assert_eq!(g.size(), ivec2(1, 1));
        g.add_row();
        g.remove_row();
        assert_eq!(g.size(), ivec2(1, 1));
    }

    #[test]
    fn undo_restores_grid_before_stroke() {
        let mut g = Grid::parse("...\n");
        let mut h = History::new(g.clone());
        let before = g.clone();
        g.set(ivec2(0, 0), '#');
        g.set(ivec2(1, 0), '#');
        h.commit(before, &g);
        h.undo(&mut g);
        assert_eq!(g.to_text(), "...\n");
        h.undo(&mut g);
        assert_eq!(g.to_text(), "...\n");
    }

    #[test]
    fn stroke_without_change_leaves_no_snapshot() {
        let mut g = Grid::parse("#.\n");
        let mut h = History::new(g.clone());
        let first = g.clone();
        g.set(ivec2(1, 0), '#');
        h.commit(first, &g);
        let before = g.clone();
        g.set(ivec2(0, 0), '#');
        h.commit(before, &g);
        h.undo(&mut g);
        assert_eq!(g.to_text(), "#.\n");
    }

    #[test]
    fn undo_reverts_resize() {
        let mut g = Grid::parse("P\n");
        let mut h = History::new(g.clone());
        let before = g.clone();
        g.add_column();
        h.commit(before, &g);
        let before = g.clone();
        g.add_row();
        h.commit(before, &g);
        h.undo(&mut g);
        assert_eq!(g.to_text(), "P.\n");
        h.undo(&mut g);
        assert_eq!(g.to_text(), "P\n");
    }

    #[test]
    fn history_keeps_last_hundred_snapshots() {
        let mut g = Grid::parse("");
        let mut h = History::new(g.clone());
        for _ in 0..=HISTORY_LIMIT {
            let before = g.clone();
            g.add_row();
            h.commit(before, &g);
        }
        for _ in 0..=HISTORY_LIMIT {
            h.undo(&mut g);
        }
        assert_eq!(g.size(), ivec2(1, 2));
    }

    #[test]
    fn dirty_until_back_to_saved() {
        let mut g = Grid::parse("..\n");
        let mut h = History::new(g.clone());
        assert!(!h.dirty(&g));
        let before = g.clone();
        g.set(ivec2(0, 0), '#');
        h.commit(before, &g);
        assert!(h.dirty(&g));
        h.undo(&mut g);
        assert!(!h.dirty(&g));

        let before = g.clone();
        g.set(ivec2(1, 0), '#');
        h.commit(before, &g);
        h.saved(&g);
        assert!(!h.dirty(&g));
        h.undo(&mut g);
        assert!(h.dirty(&g));
    }
}
