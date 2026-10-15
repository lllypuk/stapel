use std::{collections::VecDeque, iter};

use engine::{IVec2, Input, MouseButton, ivec2};

/// Символ пустой клетки: им дополняются строки и заполняются новые столбцы и строки.
const EMPTY: char = '.';

/// Снимков отмены не больше; старший вытесняется.
const HISTORY_LIMIT: usize = 100;

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
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LEVEL01;

    use engine::ElementState::{Pressed, Released};

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
            g.add_column();
            h.commit(before, &g);
        }
        for _ in 0..=HISTORY_LIMIT {
            h.undo(&mut g);
        }
        assert_eq!(g.size(), ivec2(2, 1));
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
