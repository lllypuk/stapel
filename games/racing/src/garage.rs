//! Гараж: выбор машины перед заездом, модель на вращающемся подиуме.

use std::f32::consts::TAU;

use engine::{Camera3D, IVec2, Mat4, Vec3, egui, vec3};

use crate::car_config::CarConfig;
use crate::pieces::{self, KIT_SCALE, Rot};
use crate::track::RoadEntry;

/// Поворот подиума за тик, рад: оборот за 12 с при 60 Гц.
const SPIN: f32 = TAU / 720.0;
const EYE: Vec3 = vec3(3.5, 2.2, 4.5);
const LOOK: Vec3 = vec3(0.0, 0.5, 0.0);
/// Отступ окна от края, точки egui.
const MARGIN: f32 = 12.0;
/// Деталь под машиной.
pub(crate) const PODIUM: &str = "roadStraight";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GarageCmd {
    Select(usize),
    Go,
}

pub(crate) struct Garage {
    /// Индекс в `Racing::cars`.
    pub(crate) selected: usize,
    yaw: f32,
    prev_yaw: f32,
    pub(crate) queue: Vec<GarageCmd>,
}

impl Garage {
    pub(crate) fn new(selected: usize) -> Self {
        Self {
            selected,
            yaw: 0.0,
            prev_yaw: 0.0,
            queue: Vec::new(),
        }
    }

    pub(crate) fn tick(&mut self) {
        self.prev_yaw = self.yaw;
        self.yaw += SPIN;
        if self.yaw >= TAU {
            // Сдвиг обоих углов: lerp не крутит подиум оборот назад.
            self.yaw -= TAU;
            self.prev_yaw -= TAU;
        }
    }

    /// Поворот подиума между тиками; начало модели машины — в центре его верха.
    pub(crate) fn transform(&self, alpha: f32) -> Mat4 {
        let yaw = self.prev_yaw + (self.yaw - self.prev_yaw) * alpha;
        Mat4::from_rotation_y(yaw)
    }

    /// Деталь подиума с центром клетки в начале координат.
    pub(crate) fn podium(&self, alpha: f32) -> Mat4 {
        let road = RoadEntry {
            piece: pieces::piece(PODIUM).expect("деталь подиума"),
            cell: IVec2::ZERO,
            level: 0,
            rot: Rot::default(),
        };
        self.transform(alpha)
            * Mat4::from_translation(vec3(-0.5, 0.0, -0.5) * KIT_SCALE)
            * road.transform()
    }

    pub(crate) fn camera() -> Camera3D {
        let mut camera = Camera3D::default();
        camera.look_at(EYE, LOOK);
        camera
    }

    /// Список машин и «Ехать»: нажатое — команды в `queue`.
    pub(crate) fn ui(&mut self, ui: &egui::Context, cars: &[CarConfig]) {
        egui::Window::new("Гараж")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-MARGIN, MARGIN))
            .show(ui, |ui| {
                for (i, car) in cars.iter().enumerate() {
                    if ui
                        .selectable_label(i == self.selected, &car.model)
                        .clicked()
                    {
                        self.queue.push(GarageCmd::Select(i));
                    }
                }
                ui.separator();
                if ui.button("Ехать").clicked() {
                    self.queue.push(GarageCmd::Go);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use std::f32::consts::PI;

    use engine::egui::accesskit::Toggled;
    use engine::vec2;

    use super::*;
    use crate::{CARS_DIR, load_cars};

    #[test]
    fn podium_spins_and_interpolates_across_full_turn() {
        let mut g = Garage::new(0);
        assert_eq!(g.transform(1.0), Mat4::IDENTITY);
        g.tick();
        assert!(
            g.transform(0.5)
                .abs_diff_eq(Mat4::from_rotation_y(SPIN / 2.0), 1e-6)
        );
        assert!(
            g.transform(1.0)
                .abs_diff_eq(Mat4::from_rotation_y(SPIN), 1e-6)
        );
        g.yaw = TAU - SPIN / 2.0;
        g.tick();
        assert!((g.yaw - SPIN / 2.0).abs() < 1e-5, "{}", g.yaw);
        let back = Mat4::from_rotation_y(-SPIN / 4.0);
        assert!(g.transform(0.25).abs_diff_eq(back, 1e-5));
    }

    #[test]
    fn podium_turns_once_in_twelve_seconds() {
        let mut g = Garage::new(0);
        for _ in 0..6 * 60 {
            g.tick();
        }
        assert!((g.yaw - PI).abs() < 1e-4, "{}", g.yaw);
    }

    #[test]
    fn podium_centers_cell_under_car_and_turns_with_it() {
        let mut g = Garage::new(0);
        let still = g.podium(1.0);
        for _ in 0..180 {
            g.tick();
        }
        assert!(g.podium(0.5).abs_diff_eq(g.transform(0.5) * still, 1e-5));
        // Центр клетки в пространстве модели детали.
        let center = vec3(0.5, 0.0, 0.5) - pieces::piece(PODIUM).expect("деталь").offset;
        let at = g.podium(0.5).transform_point3(center);
        assert!(at.abs_diff_eq(Vec3::ZERO, 1e-5), "{at}");
    }

    #[test]
    fn camera_looks_at_car() {
        let c = Garage::camera();
        let clip = c.view_proj(16.0 / 9.0) * LOOK.extend(1.0);
        let ndc = vec2(clip.x, clip.y) / clip.w;
        assert!(ndc.abs_diff_eq(vec2(0.0, 0.0), 1e-5), "{ndc}");
        assert_eq!(c.pos, EYE);
    }

    fn harness(selected: usize) -> Harness<'static, Garage> {
        let cars = load_cars(CARS_DIR.as_ref());
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .build_ui_state(
                move |ui, g: &mut Garage| g.ui(ui.ctx(), &cars),
                Garage::new(selected),
            );
        h.run();
        h
    }

    fn click(label: &str, selected: usize) -> Vec<GarageCmd> {
        let mut h = harness(selected);
        h.get_by_label(label).click();
        h.run();
        h.into_state().queue
    }

    #[test]
    fn list_marks_selected_car_and_window_stays_on_screen() {
        let h = harness(2);
        let toggled = |label| h.get_by_label(label).accesskit_node().toggled();
        assert_eq!(toggled("race-future"), Some(Toggled::True));
        assert_eq!(toggled("race"), Some(Toggled::False));
        let window = h.ctx.memory(|m| {
            let mut windows = m.layer_ids().filter(|l| l.order == egui::Order::Middle);
            let layer = windows.next().expect("окно");
            m.area_rect(layer.id).expect("прямоугольник окна")
        });
        assert_eq!(
            (window.max.x, window.min.y),
            (800.0 - MARGIN, MARGIN),
            "{window:?}"
        );
    }

    #[test]
    fn list_and_go_queue_commands() {
        assert_eq!(click("race-future", 1), [GarageCmd::Select(2)]);
        assert_eq!(click("race", 3), [GarageCmd::Select(1)]);
        assert_eq!(click("Ехать", 0), [GarageCmd::Go]);
    }
}
