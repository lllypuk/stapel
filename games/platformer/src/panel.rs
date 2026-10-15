use std::path::{Path, PathBuf};

use engine::egui::{Color32, Context, FontData, FontDefinitions, FontFamily, Ui};

use crate::editor::{Editor, EditorCmd, Resize};

/// Символы кисти с подписями — всё, что разбирает `Level::parse`.
const PALETTE: [(char, &str); 10] = [
    ('.', "пусто"),
    ('#', "земля"),
    ('P', "игрок"),
    ('E', "враг"),
    ('o', "монета"),
    ('^', "шипы"),
    ('X', "выход"),
    ('!', "разворот"),
    ('=', "платформа"),
    ('H', "блок"),
];
const ERROR: Color32 = Color32::from_rgb(255, 110, 110);
/// Без `default_fonts` у egui нет ни одного шрифта.
const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
));

const FONT_NAME: &str = "JetBrainsMono";

/// `JetBrainsMono` для всех семейств egui; `set_fonts` действует со следующего прохода.
pub(crate) fn set_fonts(ctx: &Context) {
    if ctx.fonts(|f| f.definitions().font_data.contains_key(FONT_NAME)) {
        return;
    }
    let mut fonts = FontDefinitions::empty();
    fonts
        .font_data
        .insert(FONT_NAME.into(), FontData::from_static(FONT).into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.insert(family, vec![FONT_NAME.into()]);
    }
    ctx.set_fonts(fonts);
}

/// Панель редактора: нажатые кнопки — команды в `queue`.
pub(crate) fn show(
    ui: &mut Ui,
    editor: &Editor,
    levels: &[PathBuf],
    current: &Path,
    queue: &mut Vec<EditorCmd>,
) {
    ui.heading("Кисть");
    for (ch, name) in PALETTE {
        if ui
            .selectable_label(editor.brush == ch, format!("{ch} {name}"))
            .clicked()
        {
            queue.push(EditorCmd::Pick(ch));
        }
    }
    ui.separator();
    ui.heading("Уровни");
    for path in levels {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if ui.selectable_label(path == current, name).clicked() {
            queue.push(EditorCmd::Open(path.clone()));
        }
    }
    let mut button = |ui: &mut Ui, text: &str, cmd: EditorCmd| {
        if ui.button(text).clicked() {
            queue.push(cmd);
        }
    };
    button(ui, "Новый", EditorCmd::New);
    ui.separator();
    ui.heading("Размер");
    let size = editor.grid.size();
    ui.label(format!("{}×{}", size.x, size.y));
    ui.horizontal(|ui| {
        button(ui, "+ столбец", EditorCmd::Resize(Resize::AddColumn));
        button(ui, "- столбец", EditorCmd::Resize(Resize::RemoveColumn));
    });
    ui.horizontal(|ui| {
        button(ui, "+ строка", EditorCmd::Resize(Resize::AddRow));
        button(ui, "- строка", EditorCmd::Resize(Resize::RemoveRow));
    });
    ui.separator();
    button(ui, "Сохранить", EditorCmd::Save);
    button(ui, "Отменить", EditorCmd::Undo);
    button(ui, "Загрузить с диска", EditorCmd::LoadFromDisk);
    if editor.changed_on_disk {
        ui.colored_label(ERROR, "файл изменён на диске");
    }
    for error in [&editor.error, &editor.file_error].into_iter().flatten() {
        ui.colored_label(ERROR, error);
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable};
    use engine::egui;

    use super::*;

    struct State {
        editor: Editor,
        queue: Vec<EditorCmd>,
    }

    fn levels() -> Vec<PathBuf> {
        vec!["/l/level01.txt".into(), "/l/level02.txt".into()]
    }

    fn harness(editor: Editor) -> Harness<'static, State> {
        let levels = levels();
        let h = Harness::builder()
            .with_size(egui::vec2(400.0, 900.0))
            .build_ui_state(
                move |ui, s: &mut State| show(ui, &s.editor, &levels, &levels[0], &mut s.queue),
                State {
                    editor,
                    queue: Vec::new(),
                },
            );
        set_fonts(&h.ctx);
        h
    }

    fn click(label: &str) -> Vec<EditorCmd> {
        let mut h = harness(Editor::new("P\n"));
        h.run();
        h.get_by_label(label).click();
        h.run();
        h.into_state().queue
    }

    #[test]
    fn buttons_queue_commands() {
        let cases = [
            ("# земля", EditorCmd::Pick('#')),
            ("^ шипы", EditorCmd::Pick('^')),
            ("level02.txt", EditorCmd::Open("/l/level02.txt".into())),
            ("Новый", EditorCmd::New),
            ("+ столбец", EditorCmd::Resize(Resize::AddColumn)),
            ("- столбец", EditorCmd::Resize(Resize::RemoveColumn)),
            ("+ строка", EditorCmd::Resize(Resize::AddRow)),
            ("- строка", EditorCmd::Resize(Resize::RemoveRow)),
            ("Сохранить", EditorCmd::Save),
            ("Отменить", EditorCmd::Undo),
            ("Загрузить с диска", EditorCmd::LoadFromDisk),
        ];
        for (label, cmd) in cases {
            assert_eq!(click(label), [cmd], "{label}");
        }
    }

    #[test]
    fn whole_palette_is_offered() {
        let mut h = harness(Editor::new("P\n"));
        h.run();
        for (ch, name) in PALETTE {
            h.get_by_label(&format!("{ch} {name}"));
        }
    }

    #[test]
    fn errors_and_conflict_shown_as_text() {
        for file_error in ["level01.txt: нет доступа", crate::UNSAVED] {
            let mut editor = Editor::new("P\n");
            editor.error = Some("нет игрока".into());
            editor.changed_on_disk = true;
            editor.file_error = Some(file_error.into());
            let mut h = harness(editor);
            h.run();
            h.get_by_label("нет игрока");
            h.get_by_label("файл изменён на диске");
            h.get_by_label(file_error);
        }
    }

    #[test]
    fn calm_editor_shows_no_errors() {
        let mut h = harness(Editor::new("P\n"));
        h.run();
        assert!(h.query_by_label("файл изменён на диске").is_none());
    }

    #[test]
    fn click_leaves_keyboard_to_game() {
        let mut h = harness(Editor::new("P\n"));
        h.run();
        h.get_by_label("Сохранить").click();
        h.run();
        assert!(!h.ctx.egui_wants_keyboard_input());
    }
}
