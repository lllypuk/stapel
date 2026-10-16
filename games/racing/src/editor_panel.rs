//! Панель редактора трасс: инструмент, уровень, файлы трасс, статус цепочки.

use std::path::{Path, PathBuf};

use engine::egui::{self, Ui};

use crate::editor::{Editor, EditorCmd, Tool};
use crate::pieces::{DECOR, PIECES, Rot};
use crate::track::{Checkpoint, DraftError, Entry};

/// Отступ окна от края, точки egui.
const MARGIN: f32 = 12.0;

/// Смена трассы; при `dirty` ждёт подтверждения.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Switch {
    Open(PathBuf),
    New,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PanelCmd {
    Edit(EditorCmd),
    Switch(Switch),
    Save,
    LoadFromDisk,
    /// Выполнить `pending`, потеряв правки.
    Discard,
    Keep,
}

#[derive(Default)]
pub(crate) struct EditorPanel {
    pub(crate) queue: Vec<PanelCmd>,
    /// Файлы трасс каталога по имени.
    pub(crate) tracks: Vec<PathBuf>,
    pub(crate) pending: Option<Switch>,
    /// Файл изменён снаружи, пока черновик `dirty`.
    pub(crate) changed_on_disk: bool,
    /// Ошибка последней работы с файлом.
    pub(crate) error: Option<String>,
}

/// Номер детали — с единицы, как `[[road]]` по порядку в файле.
pub(crate) fn chain_status(chain: &Result<Vec<Checkpoint>, DraftError>) -> String {
    match chain {
        Ok(checkpoints) => format!("цепочка цела, чекпоинтов: {}", checkpoints.len()),
        Err(e) => match e.entry() {
            Some(Entry::Road(i)) => format!("деталь #{}: {e}", i + 1),
            _ => e.to_string(),
        },
    }
}

impl EditorPanel {
    /// Окно слева сверху: нажатое — команды в `queue`.
    pub(crate) fn ui(&mut self, ui: &egui::Context, editor: &Editor, current: &Path) {
        egui::Window::new("Редактор")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(MARGIN, MARGIN))
            .show(ui, |ui| self.show(ui, editor, current));
    }

    fn show(&mut self, ui: &mut Ui, editor: &Editor, current: &Path) {
        let queue = &mut self.queue;
        let mut on_click = |r: egui::Response, cmd: PanelCmd| {
            if r.clicked() {
                queue.push(cmd);
            }
        };
        let tool = |t| PanelCmd::Edit(EditorCmd::Tool(t));
        let (rot, yaw) = match editor.tool {
            Tool::Road { rot, .. } => (rot, 0.0),
            Tool::Decor { yaw, .. } => (Rot::default(), yaw),
            Tool::Erase => (Rot::default(), 0.0),
        };
        ui.label("Дороги");
        ui.horizontal_wrapped(|ui| {
            for piece in &PIECES {
                let on = matches!(editor.tool, Tool::Road { piece: p, .. } if p == piece);
                on_click(
                    ui.selectable_label(on, piece.name),
                    tool(Tool::Road { piece, rot }),
                );
            }
        });
        ui.label("Декор");
        ui.horizontal_wrapped(|ui| {
            for piece in &DECOR {
                let on = matches!(editor.tool, Tool::Decor { piece: p, .. } if p == piece);
                on_click(
                    ui.selectable_label(on, piece.name),
                    tool(Tool::Decor { piece, yaw }),
                );
            }
            let on = editor.tool == Tool::Erase;
            on_click(ui.selectable_label(on, "стереть"), tool(Tool::Erase));
        });
        let degrees = match editor.tool {
            Tool::Road { rot, .. } => Some(rot.degrees().to_string()),
            Tool::Decor { yaw, .. } => Some(yaw.to_string()),
            Tool::Erase => None,
        };
        if let Some(degrees) = degrees {
            ui.horizontal(|ui| {
                ui.label(format!("поворот {degrees}°"));
                on_click(ui.button("−"), tool(editor.tool.turned(-1)));
                on_click(ui.button("+"), tool(editor.tool.turned(1)));
            });
        }
        ui.horizontal(|ui| {
            let level = |l| PanelCmd::Edit(EditorCmd::Level(l));
            ui.label(format!("уровень {}", editor.level));
            on_click(ui.button("ниже"), level(editor.level.saturating_sub(1)));
            on_click(ui.button("выше"), level(editor.level + 1));
        });
        ui.separator();
        let status = chain_status(&editor.chain);
        if editor.chain.is_ok() {
            ui.label(status);
        } else {
            ui.colored_label(ui.visuals().error_fg_color, status);
        }
        ui.separator();
        ui.label("Трассы");
        for path in &self.tracks {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let open = PanelCmd::Switch(Switch::Open(path.clone()));
            on_click(ui.selectable_label(path == current, name), open);
        }
        ui.horizontal(|ui| {
            on_click(ui.button("Новый"), PanelCmd::Switch(Switch::New));
            on_click(ui.button("Сохранить"), PanelCmd::Save);
            on_click(ui.button("Отменить"), PanelCmd::Edit(EditorCmd::Undo));
        });
        on_click(ui.button("Загрузить с диска"), PanelCmd::LoadFromDisk);
        let error = ui.visuals().error_fg_color;
        if self.changed_on_disk {
            ui.colored_label(error, "файл изменён на диске");
        }
        if self.pending.is_some() {
            ui.colored_label(error, "есть несохранённые правки");
            ui.horizontal(|ui| {
                on_click(ui.button("Отбросить"), PanelCmd::Discard);
                on_click(ui.button("Оставить"), PanelCmd::Keep);
            });
        }
        if let Some(e) = &self.error {
            ui.colored_label(error, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use engine::egui::accesskit::Toggled;
    use engine::{Assets, IVec2};

    use super::*;
    use crate::layout::Models;
    use crate::pieces::{decor_piece, piece};
    use crate::track::{RoadEntry, TrackDraft};

    struct State {
        panel: EditorPanel,
        editor: Editor,
    }

    fn tracks() -> Vec<PathBuf> {
        vec!["/t/track01.toml".into(), "/t/track02.toml".into()]
    }

    fn editor(roads: Vec<RoadEntry>) -> Editor {
        let mut assets = Assets::default();
        let models = Models::load(&mut assets);
        let draft = TrackDraft {
            car: "race".into(),
            roads,
            decor: Vec::new(),
        };
        Editor::new(draft, &models, &assets)
    }

    fn harness(panel: EditorPanel, editor: Editor) -> Harness<'static, State> {
        let current = tracks().remove(0);
        let panel = EditorPanel {
            tracks: tracks(),
            ..panel
        };
        let mut h = Harness::builder()
            .with_size(egui::vec2(600.0, 1000.0))
            .build_ui_state(
                move |ui, s: &mut State| s.panel.ui(ui.ctx(), &s.editor, &current),
                State { panel, editor },
            );
        engine::ui::set_font(&h.ctx, crate::FONT_PATH);
        h.run();
        h
    }

    fn click(tool: Tool, label: &str) -> Vec<PanelCmd> {
        let mut e = editor(Vec::new());
        e.tool = tool;
        e.level = 2;
        let mut h = harness(EditorPanel::default(), e);
        h.get_by_label(label).click();
        h.run();
        h.into_state().panel.queue
    }

    fn road(name: &str, deg: i64) -> Tool {
        Tool::Road {
            piece: piece(name).expect("деталь"),
            rot: Rot::from_degrees(deg).expect("rot"),
        }
    }

    fn decor(name: &str, yaw: f32) -> Tool {
        Tool::Decor {
            piece: decor_piece(name).expect("декор"),
            yaw,
        }
    }

    #[test]
    fn buttons_queue_commands() {
        let edit = PanelCmd::Edit;
        let tool = |t| edit(EditorCmd::Tool(t));
        let curve = road("roadCornerLarge", 90);
        let cases = [
            ("roadStart", tool(road("roadStart", 90))),
            ("cone", tool(decor("cone", 0.0))),
            ("стереть", tool(Tool::Erase)),
            ("+", tool(road("roadCornerLarge", 180))),
            ("−", tool(road("roadCornerLarge", 0))),
            ("выше", edit(EditorCmd::Level(3))),
            ("ниже", edit(EditorCmd::Level(1))),
            (
                "track02.toml",
                PanelCmd::Switch(Switch::Open(tracks().remove(1))),
            ),
            ("Новый", PanelCmd::Switch(Switch::New)),
            ("Сохранить", PanelCmd::Save),
            ("Отменить", edit(EditorCmd::Undo)),
            ("Загрузить с диска", PanelCmd::LoadFromDisk),
        ];
        for (label, cmd) in cases {
            assert_eq!(click(curve, label), [cmd], "{label}");
        }
        let tree = decor("treeLarge", 30.0);
        assert_eq!(click(tree, "+"), [tool(decor("treeLarge", 45.0))]);
        assert_eq!(click(tree, "barrierRed"), [tool(decor("barrierRed", 30.0))]);
        assert_eq!(click(tree, "roadStart"), [tool(road("roadStart", 0))]);
    }

    #[test]
    fn whole_palette_offered_with_tool_and_track_selected() {
        let mut e = editor(Vec::new());
        e.tool = decor("cone", 15.0);
        let h = harness(EditorPanel::default(), e);
        let toggled = |label| h.get_by_label(label).accesskit_node().toggled();
        for p in &PIECES {
            assert_eq!(toggled(p.name), Some(Toggled::False), "{}", p.name);
        }
        for d in &DECOR {
            let on = if d.name == "cone" {
                Toggled::True
            } else {
                Toggled::False
            };
            assert_eq!(toggled(d.name), Some(on), "{}", d.name);
        }
        assert_eq!(toggled("стереть"), Some(Toggled::False));
        assert_eq!(toggled("track01.toml"), Some(Toggled::True));
        assert_eq!(toggled("track02.toml"), Some(Toggled::False));
        h.get_by_label("поворот 15°");
        h.get_by_label("уровень 0");
    }

    #[test]
    fn eraser_has_no_turn() {
        let mut e = editor(Vec::new());
        e.tool = Tool::Erase;
        let h = harness(EditorPanel::default(), e);
        assert!(h.query_by_label("+").is_none());
        assert_eq!(
            h.get_by_label("стереть").accesskit_node().toggled(),
            Some(Toggled::True)
        );
    }

    #[test]
    fn chain_status_names_piece_from_one() {
        let start = RoadEntry {
            piece: piece("roadStart").expect("деталь"),
            cell: IVec2::ZERO,
            level: 0,
            rot: Rot::default(),
        };
        let e = editor(vec![start, start]);
        assert!(matches!(e.chain, Err(DraftError::Start { count: 2 })));
        assert_eq!(chain_status(&e.chain), "нужен один roadStart, а их 2");
        let e = editor(vec![start]);
        let Err(error) = &e.chain else {
            panic!("одна деталь — не круг");
        };
        let status = format!("деталь #1: {error}");
        assert_eq!(chain_status(&e.chain), status);
        let h = harness(EditorPanel::default(), e);
        h.get_by_label(&status);
        let oval = TrackDraft::parse(include_str!("testdata/oval.toml")).expect("овал");
        let checkpoints = oval.chain().expect("цепочка");
        let want = format!("цепочка цела, чекпоинтов: {}", checkpoints.len());
        assert_eq!(chain_status(&Ok(checkpoints)), want);
    }

    #[test]
    fn conflict_pending_and_error_shown() {
        let calm = harness(EditorPanel::default(), editor(Vec::new()));
        for label in [
            "файл изменён на диске",
            "есть несохранённые правки",
            "Отбросить",
        ] {
            assert!(calm.query_by_label(label).is_none(), "{label}");
        }
        let panel = || EditorPanel {
            pending: Some(Switch::New),
            changed_on_disk: true,
            error: Some("track01.toml: нет доступа".into()),
            ..EditorPanel::default()
        };
        let mut h = harness(panel(), editor(Vec::new()));
        h.get_by_label("файл изменён на диске");
        h.get_by_label("есть несохранённые правки");
        h.get_by_label("track01.toml: нет доступа");
        h.get_by_label("Отбросить").click();
        h.run();
        assert_eq!(h.state().panel.queue, [PanelCmd::Discard]);
        let mut h = harness(panel(), editor(Vec::new()));
        h.get_by_label("Оставить").click();
        h.run();
        assert_eq!(h.state().panel.queue, [PanelCmd::Keep]);
    }
}
