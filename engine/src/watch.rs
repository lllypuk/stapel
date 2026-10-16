use std::{fs, path::Path};

/// Через сколько тиков `Watcher::tick` перечитывает файл.
pub const WATCH_TICKS: u32 = 30;

/// Последний загруженный или сохранённый текст файла; изменение — по содержимому, mtime грубый.
#[derive(Default)]
pub struct Watcher {
    base: String,
    /// Файл не читается; ошибка уже отдана.
    missing: bool,
    ticks: u32,
}

impl Watcher {
    /// Наблюдатель с базой `base` — текстом, уже загруженным игрой.
    pub fn new(base: String) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    /// Свой текст в файле — не изменение.
    pub fn saved(&mut self, text: String) {
        self.base = text;
    }

    /// Каждый `WATCH_TICKS`-й вызов — `poll`.
    ///
    /// # Errors
    /// Как у `poll`.
    pub fn tick(&mut self, path: &Path) -> Result<Option<String>, String> {
        self.ticks += 1;
        if self.ticks < WATCH_TICKS {
            return Ok(None);
        }
        self.ticks = 0;
        self.poll(path)
    }

    /// Текст файла, если он не совпал с базой; он же — новая база.
    ///
    /// # Errors
    /// Файл не читается — текст с путём, один раз до успешного чтения.
    pub fn poll(&mut self, path: &Path) -> Result<Option<String>, String> {
        match fs::read_to_string(path) {
            Ok(text) => {
                self.missing = false;
                if text == self.base {
                    return Ok(None);
                }
                self.base.clone_from(&text);
                Ok(Some(text))
            }
            Err(_) if self.missing => Ok(None),
            Err(e) => {
                self.missing = true;
                Err(format!("{}: {e}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Файл `a.txt` с текстом `text` в пустом каталоге теста в `target/` копии дерева.
    fn temp_file(name: &str, text: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/test-dirs"))
            .join(format!("engine-watch-{name}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn watcher_reports_each_new_text_once() {
        let path = temp_file("new", "P\n");
        let mut w = Watcher::new("P\n".into());
        assert_eq!(w.poll(&path), Ok(None));
        fs::write(&path, "P#\n").unwrap();
        assert_eq!(w.poll(&path), Ok(Some("P#\n".into())));
        assert_eq!(w.poll(&path), Ok(None));
        w.saved("P##\n".into());
        fs::write(&path, "P##\n").unwrap();
        assert_eq!(w.poll(&path), Ok(None));
    }

    #[test]
    fn watcher_reports_missing_file_once() {
        let path = temp_file("missing", "P\n");
        let mut w = Watcher::new("P\n".into());
        fs::remove_file(&path).unwrap();
        assert!(w.poll(&path).unwrap_err().contains("a.txt"));
        assert_eq!(w.poll(&path), Ok(None));
        fs::write(&path, "P\n").unwrap();
        assert_eq!(w.poll(&path), Ok(None));
        fs::remove_file(&path).unwrap();
        assert!(w.poll(&path).is_err());
    }

    #[test]
    fn watcher_reads_every_watch_ticks() {
        let path = temp_file("ticks", "P#\n");
        let mut w = Watcher::new("P\n".into());
        for _ in 1..WATCH_TICKS {
            assert_eq!(w.tick(&path), Ok(None));
        }
        assert_eq!(w.tick(&path), Ok(Some("P#\n".into())));
        fs::write(&path, "P##\n").unwrap();
        for _ in 1..WATCH_TICKS {
            assert_eq!(w.tick(&path), Ok(None));
        }
        assert_eq!(w.tick(&path), Ok(Some("P##\n".into())));
    }
}
