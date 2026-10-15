use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Файлы `*.txt` каталога по имени.
pub(crate) fn list(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut levels = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "txt") {
            levels.push(path);
        }
    }
    levels.sort();
    Ok(levels)
}

/// Уровень после `current`; после последнего и вне списка — первый.
pub(crate) fn next<'a>(levels: &'a [PathBuf], current: &Path) -> Option<&'a Path> {
    let i = levels
        .iter()
        .position(|p| p == current)
        .map_or(0, |i| i + 1);
    levels.get(i).or(levels.first()).map(PathBuf::as_path)
}

/// Атомарно: временный файл рядом и `rename`; при ошибке временный удаляется.
pub(crate) fn write(path: &Path, text: &str) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|e| {
            fs::remove_file(&tmp).ok();
            format!("{}: {e}", path.display())
        })
}

/// Через сколько тиков `Watcher::tick` перечитывает файл.
pub(crate) const WATCH_TICKS: u32 = 30;

/// Последний загруженный или сохранённый текст уровня; изменение — по содержимому, mtime грубый.
#[derive(Default)]
pub(crate) struct Watcher {
    base: String,
    /// Файл не читается; ошибка уже отдана.
    missing: bool,
    ticks: u32,
}

impl Watcher {
    pub(crate) fn new(base: String) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    /// Свой текст в файле — не изменение.
    pub(crate) fn saved(&mut self, text: String) {
        self.base = text;
    }

    /// Каждый `WATCH_TICKS`-й вызов — `poll`.
    pub(crate) fn tick(&mut self, path: &Path) -> Result<Option<String>, String> {
        self.ticks += 1;
        if self.ticks < WATCH_TICKS {
            return Ok(None);
        }
        self.ticks = 0;
        self.poll(path)
    }

    /// Текст файла, если он не совпал с базой; он же — новая база. Ошибка чтения — один раз до успешного.
    pub(crate) fn poll(&mut self, path: &Path) -> Result<Option<String>, String> {
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

/// Пустой временный каталог теста с файлами `files`.
#[cfg(test)]
pub(crate) fn temp_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("platformer-{}-{name}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_takes_txt_files_by_name() {
        let dir = temp_dir(
            "list",
            &[("level02.txt", ""), ("level01.txt", ""), ("notes.md", "")],
        );
        fs::create_dir(dir.join("old.txt")).unwrap();
        assert_eq!(
            list(&dir).unwrap(),
            [dir.join("level01.txt"), dir.join("level02.txt")]
        );
        assert!(list(&dir.join("missing")).is_err());
    }

    #[test]
    fn write_replaces_file_without_leftovers() {
        let dir = temp_dir("write", &[("a.txt", "P\n")]);
        let path = dir.join("a.txt");
        write(&path, "P#\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "P#\n");
        assert_eq!(list(&dir).unwrap(), [path]);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_reports_path_and_leaves_no_temp() {
        let dir = temp_dir("write-fail", &[]);
        let gone = dir.join("gone/a.txt");
        assert!(write(&gone, "P\n").unwrap_err().contains("gone/a.txt"));
        let taken = dir.join("a.txt");
        fs::create_dir(&taken).unwrap();
        assert!(write(&taken, "P\n").unwrap_err().contains("a.txt"));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn watcher_reports_each_new_text_once() {
        let dir = temp_dir("watch", &[("a.txt", "P\n")]);
        let path = dir.join("a.txt");
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
        let dir = temp_dir("watch-missing", &[("a.txt", "P\n")]);
        let path = dir.join("a.txt");
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
        let dir = temp_dir("watch-ticks", &[("a.txt", "P#\n")]);
        let path = dir.join("a.txt");
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

    #[test]
    fn next_wraps_to_first() {
        let levels = [PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let next = |p: &str| next(&levels, Path::new(p)).unwrap();
        assert_eq!(next("a.txt"), Path::new("b.txt"));
        assert_eq!(next("b.txt"), Path::new("a.txt"));
        assert_eq!(next("gone.txt"), Path::new("a.txt"));
        assert_eq!(super::next(&[], Path::new("a.txt")), None);
    }
}
