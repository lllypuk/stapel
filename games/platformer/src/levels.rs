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
    fn next_wraps_to_first() {
        let levels = [PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let next = |p: &str| next(&levels, Path::new(p)).unwrap();
        assert_eq!(next("a.txt"), Path::new("b.txt"));
        assert_eq!(next("b.txt"), Path::new("a.txt"));
        assert_eq!(next("gone.txt"), Path::new("a.txt"));
        assert_eq!(super::next(&[], Path::new("a.txt")), None);
    }
}
