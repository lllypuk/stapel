//! Сценарии через бинарник: повторы офскрином и прогоны в окне дают равные отчёты и пиксели.

#![expect(
    clippy::print_stderr,
    reason = "SKIPPED в stderr, как у офскрин-тестов движка"
)]

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use stapel::Gpu;

const SWITCH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/scripts/switch.toml");
const SWITCH_TICKS: u32 = 500;
const SWITCH_SNAPSHOTS: &[u32] = &[0, 5, 9, 279, 285, 342, 472, 500];
const MINIMAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/scripts/minimal.toml");
const TIMEOUT: Duration = Duration::from_secs(300);
const KILL_GRACE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// На X11 одно окно реплея может перекрыть другое и приостановить его сценарий.
static WINDOW_REPLAY: Mutex<()> = Mutex::new(());

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

fn try_wait_for(
    timeout: Duration,
    mut try_wait: impl FnMut() -> io::Result<Option<ExitStatus>>,
) -> io::Result<Option<ExitStatus>> {
    let start = Instant::now();
    loop {
        if let Some(status) = try_wait()? {
            return Ok(Some(status));
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Ok(None);
        }
        std::thread::sleep(POLL_INTERVAL.min(remaining));
    }
}

fn reap_after_kill(
    pid: u32,
    grace: Duration,
    try_wait: impl FnMut() -> io::Result<Option<ExitStatus>>,
) -> String {
    // SIGKILL не гарантирует немедленный выход (например, при непрерываемом ожидании в драйвере).
    match try_wait_for(grace, try_wait) {
        Ok(Some(status)) => format!("PID {pid} завершён: {status}"),
        Ok(None) => format!("PID {pid} не завершился за {grace:?} после попытки SIGKILL"),
        Err(e) => format!("не удалось получить статус завершения PID {pid}: {e}"),
    }
}

fn wait_for_replay(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let failure = match try_wait_for(timeout, || child.try_wait()) {
        Ok(Some(status)) => return Ok(status),
        Ok(None) => format!("тайм-аут {timeout:?}"),
        Err(e) => format!("ошибка ожидания: {e}"),
    };
    let pid = child.id();
    let kill_error = child.kill().err();
    let cleanup = reap_after_kill(pid, KILL_GRACE, || child.try_wait());
    if let Some(e) = kill_error {
        return Err(format!(
            "{failure}; ошибка SIGKILL для PID {pid}: {e}; {cleanup}"
        ));
    }
    Err(format!("{failure}; {cleanup}"))
}

fn run(script: &str, out: &Path, window: bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_trains"));
    cmd.args(["--script", script, "--out"]).arg(out);
    if window {
        cmd.arg("--window");
    }
    let mut child = cmd
        .stdout(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    let status = wait_for_replay(&mut child, TIMEOUT).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?}: {status}");
}

fn assert_same(a: &Path, b: &Path, expected_ticks: u32, expected_snapshots: &[u32]) {
    let report = |dir: &Path| std::fs::read(dir.join("report.json")).expect("report.json");
    let (a_report, b_report) = (report(a), report(b));
    let parse = |bytes: &[u8]| -> stapel::serde_json::Value {
        stapel::serde_json::from_slice(bytes).expect("json")
    };
    let (ra, rb) = (parse(&a_report), parse(&b_report));
    for key in ["adapter", "backend"] {
        assert_eq!(
            ra[key], rb[key],
            "{key}: сравнение пикселей между адаптерами не имеет смысла"
        );
    }
    assert_eq!(ra, rb);
    assert!(a_report == b_report, "report.json: байты разошлись");
    // Равенство само по себе не ловит снимки, одинаково пропущенные обоими процессами.
    assert_eq!(ra["ticks"], expected_ticks);
    let snapshots = ra["snapshots"].as_array().expect("snapshots");
    assert_eq!(snapshots.len(), expected_snapshots.len());
    for (s, &tick) in snapshots.iter().zip(expected_snapshots) {
        assert_eq!(s["tick"], tick);
        let png = format!("tick-{tick:06}.png");
        assert_eq!(s["png"], png);
        let read = |dir: &Path| std::fs::read(dir.join(&png)).expect("png");
        let (pa, pb) = (read(a), read(b));
        assert!(pa.starts_with(b"\x89PNG\r\n\x1a\n"), "{png}: не PNG");
        assert!(pa == pb, "{png}: пиксели разошлись");
    }
}

#[test]
fn two_offscreen_processes_agree() {
    if !gpu_present() {
        return;
    }
    let (a, b) = (out_dir("replay-a"), out_dir("replay-b"));
    run(SWITCH, &a, false);
    run(SWITCH, &b, false);
    assert_same(&a, &b, SWITCH_TICKS, SWITCH_SNAPSHOTS);
}

fn assert_window_matches_offscreen(script: &str, prefix: &str, ticks: u32, snapshots: &[u32]) {
    let gpu = gpu_present();
    let display = display_present();
    if !gpu || !display {
        return;
    }
    let off = out_dir(&format!("{prefix}-off"));
    let win = out_dir(&format!("{prefix}-win"));
    run(script, &off, false);
    {
        let _window = WINDOW_REPLAY.lock().expect("оконный реплей уже упал");
        run(script, &win, true);
    }
    assert_same(&off, &win, ticks, snapshots);
}

#[test]
#[ignore = "нужен дисплей и до 10 с на прогон: make test и make cov включают"]
fn window_agrees_with_offscreen() {
    assert_window_matches_offscreen(SWITCH, "replay", SWITCH_TICKS, SWITCH_SNAPSHOTS);
}

#[test]
#[ignore = "нужен дисплей: make test и make cov включают"]
fn minimal_window_agrees_with_offscreen() {
    assert_window_matches_offscreen(MINIMAL, "minimal-replay", 1, &[0, 1]);
}

#[test]
fn replay_timeout_kills_and_reaps_child() {
    use std::os::unix::process::ExitStatusExt;

    let mut child = Command::new("sleep").arg("60").spawn().expect("sleep");
    let pid = child.id();
    let error = wait_for_replay(&mut child, Duration::ZERO).expect_err("тайм-аут");
    assert!(
        error.contains(&format!("тайм-аут 0ns; PID {pid} завершён")),
        "{error}"
    );
    assert!(!Path::new(&format!("/proc/{pid}")).exists(), "PID {pid}");
    let status = child.try_wait().expect("статус").expect("процесс завершён");
    assert_eq!(status.signal(), Some(9));
}

#[test]
fn reap_after_kill_reports_pid_when_grace_expires() {
    let grace = Duration::from_millis(10);
    let start = Instant::now();
    let mut polls = 0;
    let diagnostic = reap_after_kill(1234, grace, || {
        polls += 1;
        assert!(polls < 10, "ожидание после SIGKILL не ограничено");
        Ok(None)
    });
    assert!(start.elapsed() >= grace);
    assert_eq!(
        diagnostic,
        "PID 1234 не завершился за 10ms после попытки SIGKILL"
    );
}
