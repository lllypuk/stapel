use std::ops::RangeInclusive;

use engine::egui::{Slider, Ui};

/// Множитель ambient на ползунке.
pub(crate) const AMBIENT: RangeInclusive<f32> = 0.0..=8.0;
/// Конец тумана, м; равный началу делил бы на ноль в шейдере.
pub(crate) const FOG_END: RangeInclusive<f32> = crate::FOG.start + 0.5..=30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cmd {
    Ambient(f32),
    FogEnd(f32),
}

/// Панель отладки: сдвинутые ползунки — команды в `queue`.
pub(crate) fn show(ui: &mut Ui, fps: u32, ambient: f32, fog_end: f32, queue: &mut Vec<Cmd>) {
    ui.label(format!("FPS {fps}"));
    let mut slider = |ui: &mut Ui, value: f32, range, text, cmd: fn(f32) -> Cmd| {
        let mut v = value;
        if ui.add(Slider::new(&mut v, range).text(text)).changed() {
            queue.push(cmd(v));
        }
    };
    slider(ui, ambient, AMBIENT, "ambient ×", Cmd::Ambient);
    slider(ui, fog_end, FOG_END, "туман, м", Cmd::FogEnd);
}
