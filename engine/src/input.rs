use std::collections::HashSet;
use std::hash::Hash;

use glam::Vec2;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

/// Удерживаемые кнопки одного устройства и их фронты с прошлого тика.
struct Buttons<T> {
    down: HashSet<T>,
    pressed: HashSet<T>,
    released: HashSet<T>,
}

impl<T> Default for Buttons<T> {
    fn default() -> Self {
        Self {
            down: HashSet::new(),
            pressed: HashSet::new(),
            released: HashSet::new(),
        }
    }
}

impl<T: Copy + Eq + Hash> Buttons<T> {
    fn on(&mut self, b: T, state: ElementState) {
        match state {
            ElementState::Pressed => {
                if self.down.insert(b) {
                    self.pressed.insert(b);
                }
            }
            ElementState::Released => {
                if self.down.remove(&b) {
                    self.released.insert(b);
                }
            }
        }
    }

    fn end_tick(&mut self) {
        self.pressed.clear();
        self.released.clear();
    }
}

/// Снимок клавиатуры и мыши для тика: удерживаемое и события с прошлого тика.
#[derive(Default)]
pub struct Input {
    keys: Buttons<KeyCode>,
    mouse: Buttons<MouseButton>,
    cursor: Option<Vec2>,
}

impl Input {
    /// Учитывает событие клавиши; автоповтор игнорируется.
    pub fn on_key(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        if !repeat {
            self.keys.on(code, state);
        }
    }

    /// Учитывает событие кнопки мыши.
    pub fn on_button(&mut self, button: MouseButton, state: ElementState) {
        self.mouse.on(button, state);
    }

    /// Курсор в физических пикселях окна; `None` — ушёл из окна.
    pub fn on_cursor(&mut self, pos: Option<Vec2>) {
        self.cursor = pos;
    }

    /// Окно не получит отпускания клавиш и кнопок, отпущенных вне его.
    pub fn focus_lost(&mut self) {
        self.keys.down.clear();
        self.mouse.down.clear();
    }

    /// Клавиша удерживается.
    pub fn is_down(&self, code: KeyCode) -> bool {
        self.keys.down.contains(&code)
    }

    /// Нажата с прошлого тика.
    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.keys.pressed.contains(&code)
    }

    /// Отпущена с прошлого тика.
    pub fn just_released(&self, code: KeyCode) -> bool {
        self.keys.released.contains(&code)
    }

    /// Кнопка мыши удерживается.
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse.down.contains(&button)
    }

    /// Кнопка мыши нажата с прошлого тика.
    pub fn mouse_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse.pressed.contains(&button)
    }

    /// Курсор в физических пикселях окна, начало сверху слева.
    pub fn cursor(&self) -> Option<Vec2> {
        self.cursor
    }

    /// Сбрасывает нажатия и отпускания тика.
    pub fn end_tick(&mut self) {
        self.keys.end_tick();
        self.mouse.end_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ElementState::{Pressed, Released};

    const K: KeyCode = KeyCode::Space;

    #[test]
    fn press_is_just_pressed_for_one_tick() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        assert!(i.just_pressed(K) && i.is_down(K));
        i.end_tick();
        assert!(!i.just_pressed(K) && i.is_down(K));
    }

    #[test]
    fn repeat_after_focus_return_is_not_a_press() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.end_tick();
        i.focus_lost();
        i.on_key(K, Pressed, true);
        assert!(!i.just_pressed(K) && !i.is_down(K));
    }

    #[test]
    fn tap_between_ticks_is_not_lost() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.on_key(K, Released, false);
        assert!(i.just_pressed(K) && i.just_released(K) && !i.is_down(K));
    }

    #[test]
    fn release_is_just_released_for_one_tick() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.end_tick();
        i.on_key(K, Released, false);
        assert!(i.just_released(K) && !i.is_down(K));
        i.end_tick();
        assert!(!i.just_released(K));
    }

    const B: MouseButton = MouseButton::Left;

    #[test]
    fn button_press_is_just_pressed_for_one_tick() {
        let mut i = Input::default();
        i.on_button(B, Pressed);
        assert!(i.mouse_just_pressed(B) && i.mouse_down(B));
        i.end_tick();
        assert!(!i.mouse_just_pressed(B) && i.mouse_down(B));
    }

    #[test]
    fn click_between_ticks_is_not_lost() {
        let mut i = Input::default();
        i.on_button(B, Pressed);
        i.on_button(B, Released);
        assert!(i.mouse_just_pressed(B) && !i.mouse_down(B));
    }

    #[test]
    fn release_without_press_changes_nothing() {
        let mut i = Input::default();
        i.on_button(B, Released);
        assert!(!i.mouse_just_pressed(B) && !i.mouse_down(B));
    }

    #[test]
    fn focus_lost_clears_held_buttons() {
        let mut i = Input::default();
        i.on_button(B, Pressed);
        i.focus_lost();
        assert!(!i.mouse_down(B) && i.mouse_just_pressed(B));
    }

    #[test]
    fn cursor_left_is_none() {
        let mut i = Input::default();
        i.on_cursor(Some(Vec2::new(3.0, 4.0)));
        assert_eq!(i.cursor(), Some(Vec2::new(3.0, 4.0)));
        i.on_cursor(None);
        assert_eq!(i.cursor(), None);
    }

    #[test]
    fn focus_lost_clears_held_keys() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.end_tick();
        i.focus_lost();
        assert!(!i.is_down(K));
    }
}
