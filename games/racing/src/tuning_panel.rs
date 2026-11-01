//! Живая панель характеристик: слайдеры по `TUNING`, сохранение и сброс файла машины.

use stapel::egui;

use crate::car_config::{CarConfig, TUNING};

/// Отступ окна от края, точки egui.
const MARGIN: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TuneCmd {
    /// Новое значение поля `TUNING[i]`.
    Set(usize, f32),
    Save,
    Reset,
}

#[derive(Default)]
pub(crate) struct TuningPanel {
    /// Открыта ли в заезде; в гараже видна всегда.
    pub(crate) open: bool,
    pub(crate) queue: Vec<TuneCmd>,
    /// Ошибка последнего сохранения или сброса.
    pub(crate) error: Option<String>,
}

impl TuningPanel {
    /// Окно в углу `corner` со слайдерами `config`: изменённое и нажатое — команды в `queue`.
    pub(crate) fn ui(&mut self, ui: &egui::Context, config: &CarConfig, corner: egui::Align2) {
        let mut config = config.clone();
        egui::Window::new("Характеристики")
            .collapsible(false)
            .resizable(false)
            .anchor(corner, -corner.to_sign() * MARGIN)
            .show(ui, |ui| {
                ui.label(&config.model);
                for (i, t) in TUNING.iter().enumerate() {
                    let value = (t.field)(&mut config);
                    let slider = egui::Slider::new(value, t.range.clone())
                        .clamping(egui::SliderClamping::Always)
                        .text(t.label);
                    if ui.add(slider).changed() {
                        self.queue.push(TuneCmd::Set(i, *value));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Сохранить").clicked() {
                        self.queue.push(TuneCmd::Save);
                    }
                    if ui.button("Сбросить").clicked() {
                        self.queue.push(TuneCmd::Reset);
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use stapel::egui::accesskit::{Action, ActionData, ActionRequest, Role};

    use super::*;
    use crate::{CARS_DIR, load_cars};

    const SIZE: egui::Vec2 = egui::vec2(800.0, 600.0);

    fn field(label: &str) -> usize {
        (TUNING.iter().position(|t| t.label == label)).expect("поле")
    }

    fn harness(panel: TuningPanel, corner: egui::Align2) -> Harness<'static, TuningPanel> {
        let race = load_cars(CARS_DIR.as_ref()).remove(1);
        let mut h = Harness::builder().with_size(SIZE).build_ui_state(
            move |ui, p: &mut TuningPanel| p.ui(ui.ctx(), &race, corner),
            panel,
        );
        h.run();
        h
    }

    fn set_value(h: &mut Harness<'_, TuningPanel>, label: &str, value: f64) {
        let slider = h.get_by_role_and_label(Role::Slider, label);
        let (target_node, target_tree) = slider.accesskit_node().locate();
        h.event(egui::Event::AccessKitActionRequest(ActionRequest {
            action: Action::SetValue,
            target_node,
            target_tree,
            data: Some(ActionData::NumericValue(value)),
        }));
        h.run();
    }

    #[test]
    fn slider_queues_field_value_clamped_to_range() {
        let mut h = harness(TuningPanel::default(), egui::Align2::LEFT_TOP);
        assert!(
            h.query_by_label(&load_cars(CARS_DIR.as_ref())[1].model)
                .is_some()
        );
        for t in &TUNING {
            let slider = h.query_by_role_and_label(Role::Slider, t.label);
            assert!(slider.is_some(), "{}", t.label);
        }
        set_value(&mut h, "max_speed", 50.0);
        set_value(&mut h, "max_speed", 1e9);
        set_value(&mut h, "travel", -1.0);
        let (speed, travel) = (field("max_speed"), field("travel"));
        assert_eq!(
            h.state().queue,
            [
                TuneCmd::Set(speed, 50.0),
                TuneCmd::Set(speed, 80.0),
                TuneCmd::Set(travel, 0.05)
            ]
        );
    }

    #[test]
    fn buttons_queue_save_and_reset() {
        let mut h = harness(TuningPanel::default(), egui::Align2::LEFT_TOP);
        h.get_by_label("Сохранить").click();
        h.run();
        h.get_by_label("Сбросить").click();
        h.run();
        assert_eq!(h.state().queue, [TuneCmd::Save, TuneCmd::Reset]);
    }

    #[test]
    fn shows_file_error() {
        let panel = TuningPanel {
            error: Some("cars/race.toml: нет файла".into()),
            ..TuningPanel::default()
        };
        let h = harness(panel, egui::Align2::LEFT_TOP);
        assert!(h.query_by_label("cars/race.toml: нет файла").is_some());
    }

    #[test]
    fn window_keeps_margin_from_its_corner() {
        let rect = |corner| {
            harness(TuningPanel::default(), corner)
                .ctx
                .memory(|m| {
                    let mut windows = m.layer_ids().filter(|l| l.order == egui::Order::Middle);
                    m.area_rect(windows.next().expect("окно").id)
                })
                .expect("прямоугольник окна")
        };
        let left = rect(egui::Align2::LEFT_TOP);
        assert_eq!((left.min.x, left.min.y), (MARGIN, MARGIN), "{left:?}");
        let right = rect(egui::Align2::RIGHT_TOP);
        assert_eq!((right.max.x, right.min.y), (SIZE.x - MARGIN, MARGIN));
    }
}
