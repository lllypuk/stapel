use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Config, Game, RunError, RunReport, Script, run_script};

/// Разобранные аргументы командной строки игры.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Args {
    pub(crate) script: Option<PathBuf>,
    pub(crate) out: Option<PathBuf>,
    pub(crate) window: bool,
    pub(crate) no_png: bool,
}

/// Разбирает `--script <path> [--out <dir>] [--window] [--no-png]`; чужие аргументы пропускает.
pub(crate) fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, RunError> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--script") => parsed.script = Some(value(&mut args, "--script")?),
            Some("--out") => parsed.out = Some(value(&mut args, "--out")?),
            Some("--window") => parsed.window = true,
            Some("--no-png") => parsed.no_png = true,
            _ => {}
        }
    }
    if parsed.script.is_none() && (parsed.out.is_some() || parsed.window || parsed.no_png) {
        return Err(RunError::Args(
            "--out, --window и --no-png требуют --script".into(),
        ));
    }
    if parsed.window && parsed.no_png {
        return Err(RunError::Args("--no-png несовместим с --window".into()));
    }
    Ok(parsed)
}

fn value(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<PathBuf, RunError> {
    match args.next() {
        Some(v) if !v.to_str().is_some_and(|s| s.starts_with("--")) => Ok(v.into()),
        _ => Err(RunError::Args(format!("после {flag} нужен путь"))),
    }
}

/// Читает сценарий, прогоняет его (с `args.window` — в окне) и пишет `report.json` рядом со снимками, а с `args.no_png` — без PNG; возвращает путь отчёта.
pub(crate) fn run_script_file<G: Game>(
    config: &Config,
    script: &Path,
    args: &Args,
) -> Result<PathBuf, RunError> {
    let script_text = fs::read_to_string(script)?;
    let parsed = Script::parse(&script_text)?;
    let dir = out_dir(script, args.out.as_deref());
    remove_stale(&dir)?;
    let report = if args.window {
        crate::app::run_script_window::<G>(config, &parsed, &dir)?
    } else {
        run_script::<G>(config, &parsed, (!args.no_png).then_some(dir.as_path()))?
    };
    fs::create_dir_all(&dir)?;
    write_report(&dir, &report)
}

fn out_dir(script: &Path, out: Option<&Path>) -> PathBuf {
    out.map_or_else(
        || Path::new("target/stapel-run").join(script.file_stem().unwrap_or(script.as_os_str())),
        Path::to_path_buf,
    )
}

/// Удаляет снимки и отчёт прошлого прогона, чтобы агент не принял их за свежие; чужие файлы каталога не трогает.
fn remove_stale(dir: &Path) -> Result<(), RunError> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = name == "report.json" || (name.starts_with("tick-") && name.ends_with(".png"));
        if ours && entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Пишет `report.json` в `dir`; возвращает его путь.
pub(crate) fn write_report(dir: &Path, report: &RunReport) -> Result<PathBuf, RunError> {
    let path = dir.join("report.json");
    let json = serde_json::to_string_pretty(report).map_err(std::io::Error::from)?;
    fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::{TempDir, gpu_full};
    use crate::{Color, Context};

    fn parse(args: &[&str]) -> Result<Args, RunError> {
        parse_args(args.iter().map(OsString::from))
    }

    #[test]
    fn no_script_is_plain_run() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
    }

    #[test]
    fn full_set() {
        let args = parse(&["--script", "a.toml", "--out", "dir", "--window"]).unwrap();
        assert!(!args.no_png);
        assert_eq!(
            args,
            Args {
                script: Some("a.toml".into()),
                out: Some("dir".into()),
                window: true,
                no_png: false,
            }
        );
        let args = parse(&["--script", "a.toml", "--no-png"]).unwrap();
        assert!(args.no_png && !args.window);
    }

    #[test]
    fn script_without_path_is_error() {
        assert!(matches!(parse(&["--script"]), Err(RunError::Args(_))));
        assert!(matches!(
            parse(&["--script", "--window"]),
            Err(RunError::Args(_))
        ));
        assert!(matches!(
            parse(&["--script", "a.toml", "--out"]),
            Err(RunError::Args(_))
        ));
    }

    #[test]
    fn out_and_window_need_script() {
        assert!(matches!(parse(&["--out", "d"]), Err(RunError::Args(_))));
        assert!(matches!(parse(&["--window"]), Err(RunError::Args(_))));
        assert!(matches!(parse(&["--no-png"]), Err(RunError::Args(_))));
    }

    #[test]
    fn no_png_conflicts_with_window() {
        assert!(matches!(
            parse(&["--script", "a.toml", "--window", "--no-png"]),
            Err(RunError::Args(_))
        ));
    }

    #[test]
    fn foreign_arguments_ignored() {
        let args = parse(&["--nocapture", "x", "--script", "a.toml"]).unwrap();
        assert_eq!(args.script.as_deref(), Some(Path::new("a.toml")));
        assert!(args.out.is_none() && !args.window);
    }

    struct Blank;

    impl Game for Blank {
        fn init(_: &mut Context) -> Self {
            Self
        }
        fn update(&mut self, _: &mut Context) {}
        fn draw(&mut self, ctx: &mut Context, _: f32) {
            ctx.gfx.clear_color = Color::WHITE;
        }
    }

    fn args(out: &Path, window: bool, no_png: bool) -> Args {
        Args {
            script: None,
            out: Some(out.to_path_buf()),
            window,
            no_png,
        }
    }

    fn config16() -> Config {
        Config {
            title: String::new(),
            size: (16, 16),
            tick_hz: 60,
            vsync: false,
            pixel_perfect: false,
        }
    }

    #[test]
    fn last_duplicate_wins_and_foreign_bytes_ignored() {
        let args = parse(&["--script", "a", "--script", "b", "--out", "x", "--out", "y"]).unwrap();
        assert_eq!(args.script.as_deref(), Some(Path::new("b")));
        assert_eq!(args.out.as_deref(), Some(Path::new("y")));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let raw = [
                OsString::from_vec(vec![0xff, 0xfe]),
                OsString::from("--script"),
                OsString::from("a"),
            ];
            let args = parse_args(raw).unwrap();
            assert_eq!(args.script.as_deref(), Some(Path::new("a")));
        }
    }

    #[test]
    fn out_value_starting_with_dashes_is_error() {
        assert!(matches!(
            parse(&["--script", "a", "--out", "--no-png"]),
            Err(RunError::Args(_))
        ));
    }

    #[test]
    fn default_out_dir_uses_stem() {
        assert_eq!(
            out_dir(Path::new("dir/level.toml"), None),
            Path::new("target/stapel-run/level")
        );
        assert_eq!(
            out_dir(Path::new(".."), None),
            Path::new("target/stapel-run/..")
        );
        assert_eq!(
            out_dir(Path::new("a.toml"), Some(Path::new("o"))),
            Path::new("o")
        );
    }

    #[test]
    fn stale_files_removed_and_foreign_kept() {
        let dir = TempDir::new("stale");
        for name in [
            "tick-000009.png",
            "report.json",
            "notes.txt",
            "tick-x.txt",
            "my.png",
        ] {
            fs::write(dir.0.join(name), "x").unwrap();
        }
        fs::create_dir(dir.0.join("tick-000001.png")).unwrap();
        remove_stale(&dir.0).unwrap();
        let mut left: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["my.png", "notes.txt", "tick-000001.png", "tick-x.txt"]
        );
        remove_stale(&dir.0.join("absent")).unwrap();
    }

    #[test]
    fn rerun_without_png_leaves_no_old_pictures() {
        if gpu_full().is_none() {
            return;
        }
        let dir = TempDir::new("rerun");
        let script = dir.0.join("s.toml");
        let out = dir.0.join("out");
        fs::write(&script, "ticks = 1\nsnapshots = [0]\n").unwrap();
        run_script_file::<Blank>(&config16(), &script, &args(&out, false, false)).unwrap();
        fs::write(out.join("keep.txt"), "x").unwrap();
        run_script_file::<Blank>(&config16(), &script, &args(&out, false, true)).unwrap();
        let mut names: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["keep.txt", "report.json"]);
    }

    #[test]
    fn writes_report_and_pngs_and_overwrites() {
        if gpu_full().is_none() {
            return;
        }
        let dir = TempDir::new("cli");
        let script = dir.0.join("s.toml");
        let out = dir.0.join("out");
        let config = Config {
            title: String::new(),
            size: (16, 16),
            tick_hz: 60,
            vsync: false,
            pixel_perfect: false,
        };
        fs::write(&script, "ticks = 2\nsnapshots = [0, 1]\n").unwrap();
        let report = run_script_file::<Blank>(&config, &script, &args(&out, false, false)).unwrap();
        assert_eq!(report, out.join("report.json"));
        for name in ["tick-000000.png", "tick-000001.png", "tick-000002.png"] {
            assert!(out.join(name).is_file(), "{name}");
        }
        let first: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(first["ticks"], 2);

        fs::write(&script, "ticks = 1\n").unwrap();
        run_script_file::<Blank>(&config, &script, &args(&out, false, false)).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(second["ticks"], 1);
        assert_eq!(second["snapshots"].as_array().unwrap().len(), 1);

        assert!(matches!(
            run_script_file::<Blank>(&config, &dir.0.join("none.toml"), &args(&out, false, false)),
            Err(RunError::Io(_))
        ));
    }

    #[test]
    fn no_png_writes_only_report() {
        if gpu_full().is_none() {
            return;
        }
        let dir = TempDir::new("nopng");
        let script = dir.0.join("s.toml");
        let out = dir.0.join("out");
        let config = Config {
            title: String::new(),
            size: (16, 16),
            tick_hz: 60,
            vsync: false,
            pixel_perfect: false,
        };
        fs::write(&script, "ticks = 2\nsnapshots = [0, 1]\n").unwrap();
        let report = run_script_file::<Blank>(&config, &script, &args(&out, false, true)).unwrap();
        let names: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["report.json"]);
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
        let snapshots = json["snapshots"].as_array().unwrap();
        assert_eq!(snapshots.len(), 3);
        assert!(snapshots.iter().all(|s| s["png"].is_null()));
    }
}
