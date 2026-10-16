use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Новый уровень: 20×10, пол и игрок.
pub(crate) const TEMPLATE: &str = concat!(
    "....................\n",
    "....................\n",
    "....................\n",
    "....................\n",
    "....................\n",
    "....................\n",
    "....................\n",
    "....................\n",
    ".P..................\n",
    "####################\n",
);

/// Файлы `*.txt` каталога по имени; ошибка — текстом с путём.
pub(crate) fn list(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = || -> io::Result<Vec<PathBuf>> {
        let mut levels = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "txt") {
                levels.push(path);
            }
        }
        levels.sort();
        Ok(levels)
    };
    read().map_err(|e| format!("{}: {e}", dir.display()))
}

/// `levelNN.txt` после наибольшего NN списка; без таких — `level01.txt`.
pub(crate) fn new_name(levels: &[PathBuf]) -> String {
    let nn = levels
        .iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?;
            name.strip_prefix("level")?
                .strip_suffix(".txt")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .map_or(1, |n| n + 1);
    format!("level{nn:02}.txt")
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

/// Пустой каталог теста с файлами `files` в `target/` копии дерева: копии `cargo mutants -j` его не делят.
#[cfg(test)]
pub(crate) fn temp_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/test-dirs/platformer-{name}"));
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
        assert!(list(&dir.join("missing")).unwrap_err().contains("missing"));
    }

    #[test]
    fn new_name_follows_largest_number() {
        let names = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(new_name(&[]), "level01.txt");
        assert_eq!(new_name(&names(&["a.txt", "level.txt"])), "level01.txt");
        let gap = names(&["d/level03.txt", "d/level01.txt", "d/level7.md"]);
        assert_eq!(new_name(&gap), "level04.txt");
        assert_eq!(new_name(&names(&["level99.txt"])), "level100.txt");
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
    fn next_wraps_to_first() {
        let levels = [PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let next = |p: &str| next(&levels, Path::new(p)).unwrap();
        assert_eq!(next("a.txt"), Path::new("b.txt"));
        assert_eq!(next("b.txt"), Path::new("a.txt"));
        assert_eq!(next("gone.txt"), Path::new("a.txt"));
        assert_eq!(super::next(&[], Path::new("a.txt")), None);
    }
}
