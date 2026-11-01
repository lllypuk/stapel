use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Config, Game, RunError, Script, run_script};

/// Разобранные аргументы командной строки игры.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Args {
    pub(crate) script: Option<PathBuf>,
    pub(crate) out: Option<PathBuf>,
    pub(crate) window: bool,
}

/// Разбирает `--script <path> [--out <dir>] [--window]`; чужие аргументы пропускает.
pub(crate) fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, RunError> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--script") => parsed.script = Some(value(&mut args, "--script")?),
            Some("--out") => parsed.out = Some(value(&mut args, "--out")?),
            Some("--window") => parsed.window = true,
            _ => {}
        }
    }
    if parsed.script.is_none() && (parsed.out.is_some() || parsed.window) {
        return Err(RunError::Args("--out и --window требуют --script".into()));
    }
    Ok(parsed)
}

fn value(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<PathBuf, RunError> {
    match args.next() {
        Some(v) if !v.to_str().is_some_and(|s| s.starts_with("--")) => Ok(v.into()),
        _ => Err(RunError::Args(format!("после {flag} нужен путь"))),
    }
}

/// Читает сценарий, прогоняет его и пишет `report.json` рядом со снимками; возвращает путь отчёта.
pub(crate) fn run_script_file<G: Game>(
    config: &Config,
    script: &Path,
    out: Option<&Path>,
) -> Result<PathBuf, RunError> {
    let script_text = fs::read_to_string(script)?;
    let parsed = Script::parse(&script_text)?;
    let dir = out.map_or_else(
        || Path::new("target/stapel-run").join(script.file_stem().unwrap_or(script.as_os_str())),
        Path::to_path_buf,
    );
    let report = run_script::<G>(config, &parsed, Some(&dir))?;
    let path = dir.join("report.json");
    let json = serde_json::to_string_pretty(&report).map_err(std::io::Error::from)?;
    fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_test::gpu_full;
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
        assert_eq!(
            args,
            Args {
                script: Some("a.toml".into()),
                out: Some("dir".into()),
                window: true,
            }
        );
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

    #[test]
    fn writes_report_and_pngs_and_overwrites() {
        if gpu_full().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("stapel-cli-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("s.toml");
        let out = dir.join("out");
        let config = Config {
            title: String::new(),
            size: (16, 16),
            tick_hz: 60,
            vsync: false,
            pixel_perfect: false,
        };
        fs::write(&script, "ticks = 2\nsnapshots = [0, 1]\n").unwrap();
        let report = run_script_file::<Blank>(&config, &script, Some(&out)).unwrap();
        assert_eq!(report, out.join("report.json"));
        for name in ["tick-000000.png", "tick-000001.png", "tick-000002.png"] {
            assert!(out.join(name).is_file(), "{name}");
        }
        let first: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(first["ticks"], 2);

        fs::write(&script, "ticks = 1\n").unwrap();
        run_script_file::<Blank>(&config, &script, Some(&out)).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(second["ticks"], 1);
        assert_eq!(second["snapshots"].as_array().unwrap().len(), 1);

        assert!(matches!(
            run_script_file::<Blank>(&config, &dir.join("none.toml"), Some(&out)),
            Err(RunError::Io(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
