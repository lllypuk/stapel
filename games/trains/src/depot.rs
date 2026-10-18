//! Депо: сборка состава на пути депо.

use engine::egui;

use crate::consist::MAX_CARS;
use crate::track::Track;
use crate::train::Train;

/// Отступ окна от края, точки egui.
const MARGIN: f32 = 12.0;
const WAGON: &str = "train/train-carriage-";
/// Подкаталог и префикс имён моделей машин; в панели не показывается.
const PREFIX: &str = "train/train-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DepotCmd {
    Locomotive(&'static str),
    Add(&'static str),
    /// Номер среди вагонов, без локомотива.
    Remove(usize),
    Go,
}

pub(crate) struct Depot {
    pub(crate) open: bool,
    /// Имена моделей: локомотив, за ним вагоны.
    pub(crate) consist: Vec<&'static str>,
    pub(crate) queue: Vec<DepotCmd>,
}

pub(crate) fn is_wagon(name: &str) -> bool {
    name.starts_with(WAGON)
}

/// Стоящий состав целиком на рёбрах пути депо.
pub(crate) fn can_open(track: &Track, train: &Train) -> bool {
    train.speed == 0.0
        && (train.route.iter()).all(|&(e, _)| track.depot.iter().any(|&(d, _)| d == e))
}

fn label(name: &str) -> &str {
    name.strip_prefix(PREFIX).unwrap_or(name)
}

impl Depot {
    pub(crate) fn new(consist: Vec<&'static str>) -> Self {
        Self {
            open: true,
            consist,
            queue: Vec::new(),
        }
    }

    /// Разбор очереди; `true` — состав изменился.
    pub(crate) fn apply(&mut self) -> bool {
        let mut changed = false;
        for cmd in std::mem::take(&mut self.queue) {
            match cmd {
                DepotCmd::Locomotive(name) if self.consist[0] != name => {
                    self.consist[0] = name;
                    changed = true;
                }
                DepotCmd::Add(name) if self.consist.len() <= MAX_CARS => {
                    self.consist.push(name);
                    changed = true;
                }
                DepotCmd::Remove(i) if i + 1 < self.consist.len() => {
                    self.consist.remove(i + 1);
                    changed = true;
                }
                DepotCmd::Go => self.open = false,
                _ => {}
            }
        }
        changed
    }

    /// Локомотивы, вагоны состава и каталога, «Ехать»: нажатое — команды в `queue`.
    pub(crate) fn ui(&mut self, ui: &egui::Context, catalog: &[&'static str]) {
        egui::Window::new("Депо")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-MARGIN, MARGIN))
            .show(ui, |ui| {
                ui.label("Локомотив");
                for &name in catalog.iter().filter(|n| !is_wagon(n)) {
                    if (ui.selectable_label(self.consist[0] == name, label(name))).clicked() {
                        self.queue.push(DepotCmd::Locomotive(name));
                    }
                }
                ui.separator();
                ui.label(format!("Вагоны {}/{MAX_CARS}", self.consist.len() - 1));
                for (i, name) in self.consist[1..].iter().enumerate() {
                    if ui.button(format!("- {}", label(name))).clicked() {
                        self.queue.push(DepotCmd::Remove(i));
                    }
                }
                ui.separator();
                let room = self.consist.len() <= MAX_CARS;
                for &name in catalog.iter().filter(|n| is_wagon(n)) {
                    let add = egui::Button::new(format!("+ {}", label(name)));
                    if ui.add_enabled(room, add).clicked() {
                        self.queue.push(DepotCmd::Add(name));
                    }
                }
                ui.separator();
                if ui.button("Ехать").clicked() {
                    self.queue.push(DepotCmd::Go);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use engine::egui::accesskit::Toggled;

    use super::*;

    const LOCO: &str = "train/train-locomotive-a";
    const DIESEL: &str = "train/train-diesel-a";
    const BOX: &str = "train/train-carriage-box";
    const TANK: &str = "train/train-carriage-tank";
    const CATALOG: [&str; 4] = [BOX, TANK, DIESEL, LOCO];

    fn depot(consist: &[&'static str], queue: &[DepotCmd]) -> Depot {
        let mut d = Depot::new(consist.to_vec());
        d.queue = queue.to_vec();
        d
    }

    #[test]
    fn commands_change_consist() {
        let mut d = depot(
            &[LOCO, BOX],
            &[
                DepotCmd::Add(TANK),
                DepotCmd::Locomotive(DIESEL),
                DepotCmd::Add(BOX),
                DepotCmd::Remove(0),
            ],
        );
        assert!(d.apply());
        assert_eq!(d.consist, [DIESEL, TANK, BOX]);
        assert_eq!(d.queue, []);
        assert!(d.open);
        assert!(!d.apply(), "пустая очередь");
    }

    #[test]
    fn noop_commands_leave_consist_unchanged() {
        let mut d = depot(
            &[LOCO, BOX],
            &[
                DepotCmd::Locomotive(LOCO),
                DepotCmd::Remove(1),
                DepotCmd::Go,
            ],
        );
        assert!(!d.apply());
        assert_eq!(d.consist, [LOCO, BOX]);
        assert!(!d.open);
        let mut d = depot(&[LOCO, BOX], &[DepotCmd::Remove(0)]);
        assert!(d.apply());
        assert_eq!(d.consist, [LOCO]);
    }

    #[test]
    fn no_more_than_max_cars_wagons() {
        let mut d = depot(&[LOCO], &[DepotCmd::Add(BOX); MAX_CARS + 1]);
        assert!(d.apply());
        assert_eq!(d.consist.len(), MAX_CARS + 1);
        d.queue.push(DepotCmd::Add(TANK));
        assert!(!d.apply());
        assert_eq!(d.consist.len(), MAX_CARS + 1);
        let mut h = harness(d);
        h.get_by_label("+ carriage-tank").click();
        h.run();
        assert_eq!(h.state().queue, [], "кнопка добавления выключена");
    }

    fn harness(d: Depot) -> Harness<'static, Depot> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui_state(|ui, d: &mut Depot| d.ui(ui.ctx(), &CATALOG), d);
        h.run();
        h
    }

    fn click(consist: &[&'static str], label: &str) -> Vec<DepotCmd> {
        let mut h = harness(depot(consist, &[]));
        h.get_all_by_label(label).last().expect(label).click();
        h.run();
        h.into_state().queue
    }

    #[test]
    fn panel_lists_locomotives_and_wagons_and_queues_commands() {
        let h = harness(depot(&[DIESEL, BOX], &[]));
        let toggled = |label| h.get_by_label(label).accesskit_node().toggled();
        assert_eq!(toggled("diesel-a"), Some(Toggled::True));
        assert_eq!(toggled("locomotive-a"), Some(Toggled::False));
        assert!(
            h.query_by_label("carriage-box").is_none(),
            "вагон не локомотив"
        );
        assert!(h.query_by_label("Вагоны 1/5").is_some());
        assert_eq!(
            click(&[DIESEL], "locomotive-a"),
            [DepotCmd::Locomotive(LOCO)]
        );
        assert_eq!(click(&[LOCO], "+ carriage-tank"), [DepotCmd::Add(TANK)]);
        assert_eq!(
            click(&[LOCO, BOX, BOX], "- carriage-box"),
            [DepotCmd::Remove(1)]
        );
        assert_eq!(click(&[LOCO], "Ехать"), [DepotCmd::Go]);
    }

    #[test]
    fn window_stays_in_top_right_corner() {
        let h = harness(depot(&[LOCO], &[]));
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
}
