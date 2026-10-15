use glam::Vec2;
use winit::event::{ElementState, WindowEvent};

/// Вывод egui за кадр для `Renderer::render`.
pub struct UiFrame {
    /// Новые и изменённые текстуры egui, в том числе атлас шрифта.
    pub textures: egui::TexturesDelta,
    /// Треугольники панелей.
    pub jobs: Vec<egui::ClippedPrimitive>,
    /// Масштаб egui: физических пикселей на точку.
    pub pixels_per_point: f32,
}

/// Идёт ли событие окна в `Input`; egui получает все события независимо от ответа.
/// Отпускание кнопки идёт всегда: иначе нажатая в мире кнопка, отпущенная над панелью, залипнет.
pub(crate) fn to_input(event: &WindowEvent, wants_pointer: bool) -> bool {
    let press = matches!(
        event,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        }
    );
    !(press && wants_pointer)
}

/// Занят ли указатель egui при курсоре `cursor` в физических пикселях.
/// `egui_wants_pointer_input` берёт позицию прошлого прохода: клик сразу после захода на панель рисовал бы в мире.
pub(crate) fn wants_pointer(ctx: &egui::Context, cursor: Option<Vec2>) -> bool {
    let over = cursor.is_some_and(|c| {
        let p = c / ctx.pixels_per_point();
        ctx.layer_id_at(egui::pos2(p.x, p.y))
            .is_some_and(|l| l.order != egui::Order::Background)
    });
    over || ctx.egui_is_using_pointer()
}

/// Снимает фокус egui: клавиатура идёт и в игру, сфокусированная кнопка нажималась бы её Space и Enter.
pub(crate) fn release_focus(ctx: &egui::Context) {
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;
    use winit::event::{DeviceId, MouseButton};

    use super::*;

    fn button(state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        }
    }

    #[test]
    fn press_over_ui_does_not_reach_input() {
        assert!(!to_input(&button(ElementState::Pressed), true));
        assert!(to_input(&button(ElementState::Pressed), false));
    }

    #[test]
    fn release_always_reaches_input() {
        assert!(to_input(&button(ElementState::Released), true));
        assert!(to_input(&button(ElementState::Released), false));
    }

    // `KeyEvent` не собрать вне winit; правило фильтрует только нажатие кнопки, прочее идёт всегда.
    #[test]
    fn other_events_always_reach_input() {
        let events = [
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(1.0, 2.0),
            },
            WindowEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            },
            WindowEvent::Focused(false),
        ];
        for e in &events {
            assert!(to_input(e, true), "{e:?}");
        }
    }

    #[test]
    fn wants_pointer_uses_current_cursor() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        for _ in 0..2 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("w")
                    .fixed_pos([100.0, 100.0])
                    .show(ui.ctx(), |ui| ui.label("x"));
            })
            .drop_without_applying_deltas();
        }
        assert!(wants_pointer(&ctx, Some(Vec2::new(220.0, 220.0))));
        assert!(!wants_pointer(&ctx, Some(Vec2::new(150.0, 150.0))));
        assert!(!wants_pointer(&ctx, None));
    }

    #[test]
    fn release_focus_leaves_keyboard_to_game() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("button")));
        assert!(ctx.egui_wants_keyboard_input());
        release_focus(&ctx);
        assert!(!ctx.egui_wants_keyboard_input());
        release_focus(&ctx);
    }
}
