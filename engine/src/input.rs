use std::collections::HashSet;

use winit::event::ElementState;
use winit::keyboard::KeyCode;

/// Снимок клавиатуры для тика: удерживаемые клавиши и события с прошлого тика.
#[derive(Default)]
pub struct Input {
    down: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    released: HashSet<KeyCode>,
}

impl Input {
    pub fn on_key(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        if repeat {
            return;
        }
        match state {
            ElementState::Pressed => {
                if self.down.insert(code) {
                    self.pressed.insert(code);
                }
            }
            ElementState::Released => {
                if self.down.remove(&code) {
                    self.released.insert(code);
                }
            }
        }
    }

    /// Окно не получит отпускания клавиш, отпущенных вне его.
    pub fn focus_lost(&mut self) {
        self.down.clear();
    }

    pub fn is_down(&self, code: KeyCode) -> bool {
        self.down.contains(&code)
    }

    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.pressed.contains(&code)
    }

    pub fn just_released(&self, code: KeyCode) -> bool {
        self.released.contains(&code)
    }

    pub fn end_tick(&mut self) {
        self.pressed.clear();
        self.released.clear();
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
    fn repeat_is_ignored() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.end_tick();
        i.on_key(K, Pressed, true);
        assert!(!i.just_pressed(K));
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

    #[test]
    fn focus_lost_clears_held_keys() {
        let mut i = Input::default();
        i.on_key(K, Pressed, false);
        i.end_tick();
        i.focus_lost();
        assert!(!i.is_down(K));
    }
}
