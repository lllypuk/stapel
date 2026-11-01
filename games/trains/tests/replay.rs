//! Один сценарий через бинарник: два процесса офскрином и окно дают равные отчёт и пиксели.

#![expect(
    clippy::print_stderr,
    reason = "SKIPPED в stderr, как у офскрин-тестов движка"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use stapel::Gpu;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/scripts/switch.toml");
const TIMEOUT: Duration = Duration::from_secs(300);

fn env_set(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

/// `false` — адаптера нет и `REQUIRE_GPU` не стоит.
fn gpu_present() -> bool {
    match Gpu::headless() {
        Ok(_) => true,
        Err(e) => {
            assert!(
                !env_set("REQUIRE_GPU"),
                "REQUIRE_GPU=1, but no GPU adapter: {e}"
            );
            eprintln!("SKIPPED: no GPU adapter: {e}");
            false
        }
    }
}

fn display_present() -> bool {
    let present = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|n| std::env::var_os(n).is_some_and(|v| !v.is_empty()));
    if !present {
        assert!(
            !env_set("REQUIRE_DISPLAY"),
            "REQUIRE_DISPLAY=1, but no display"
        );
        eprintln!("SKIPPED: no DISPLAY or WAYLAND_DISPLAY");
    }
    present
}

fn out_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run(out: &Path, window: bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_trains"));
    cmd.args(["--script", SCRIPT, "--out"]).arg(out);
    if window {
        cmd.arg("--window");
    }
    let mut child = cmd.stdout(Stdio::null()).spawn().expect("запуск trains");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("ожидание") {
            break status;
        }
        if start.elapsed() > TIMEOUT {
            child.kill().expect("kill");
            panic!("trains не завершился за {TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status}, window = {window}");
}

fn report(dir: &Path) -> stapel::serde_json::Value {
    let text = std::fs::read_to_string(dir.join("report.json")).expect("report.json");
    stapel::serde_json::from_str(&text).expect("json")
}

fn assert_same(a: &Path, b: &Path) {
    let (ra, rb) = (report(a), report(b));
    for key in ["adapter", "backend"] {
        assert_eq!(
            ra[key], rb[key],
            "{key}: сравнение пикселей между адаптерами не имеет смысла"
        );
    }
    assert_eq!(ra, rb);
    let snapshots = ra["snapshots"].as_array().expect("snapshots");
    assert!(snapshots.len() > 1);
    for s in snapshots {
        let png = s["png"].as_str().expect("png");
        let read = |dir: &Path| std::fs::read(dir.join(png)).expect("png");
        let (pa, pb) = (read(a), read(b));
        assert!(!pa.is_empty(), "{png}");
        assert!(pa == pb, "{png}: пиксели разошлись");
    }
}

#[test]
fn two_offscreen_processes_agree() {
    if !gpu_present() {
        return;
    }
    let (a, b) = (out_dir("replay-a"), out_dir("replay-b"));
    run(&a, false);
    run(&b, false);
    assert_same(&a, &b);
}

#[test]
fn window_agrees_with_offscreen() {
    if !gpu_present() || !display_present() {
        return;
    }
    let (off, win) = (out_dir("replay-off"), out_dir("replay-win"));
    run(&off, false);
    run(&win, true);
    assert_same(&off, &win);
}
