use omnibook_rs::config::{daemon_socket_path, daemon_state_path, haptic_config_path, presence_config_path};
use omnibook_rs::ipc::DaemonIpc;
use omnibook_rs::haptic::device::HapticDevice;
use omnibook_rs::presence_config::PresenceConfig;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    apply_saved_haptic();

    let prox_sensors = discover_prox_sensors();
    let attn_channels = discover_attention_channels();

    if prox_sensors.is_empty() && attn_channels.is_empty() {
        eprintln!("[omnibookd] warning: no proximity or attention sensors found");
    }

    run_sensor_loop(prox_sensors, attn_channels);
}

// ── Haptic restore ────────────────────────────────────────────────────────────

fn apply_saved_haptic() {
    let path = haptic_config_path();
    let Ok(content) = fs::read_to_string(&path) else { return };
    let Ok(val) = content.trim().parse::<u8>() else {
        eprintln!("[omnibookd] warning: invalid haptic config");
        return;
    };
    let device = HapticDevice::new();
    if device.get_device_path().is_some() {
        if let Err(e) = device.set_intensity(val) {
            eprintln!("[omnibookd] warning: haptic set failed: {}", e);
        }
    }
}

// ── Shared state ─────────────────────────────────────────────────────────────

fn write_state(presence: &str, attention: &str) {
    let content = format!("presence={}\nattention={}\n", presence, attention);
    let _ = fs::write(daemon_state_path(), content);
}

// ── Sensor discovery ──────────────────────────────────────────────────────────

struct ProxSensor {
    prox_0: PathBuf,
    prox_1: PathBuf,
}

struct AttnChannel {
    path: PathBuf,
}

fn scan_iio<T>(check: impl Fn(&PathBuf) -> Option<T>) -> Vec<T> {
    let Ok(entries) = fs::read_dir("/sys/bus/iio/devices") else { return vec![] };
    let mut all: Vec<_> = entries.flatten().collect();
    all.sort_by_key(|e| e.path());
    all.iter().filter_map(|e| {
        let dev = e.path();
        if dev.is_dir() { check(&dev) } else { None }
    }).collect()
}

fn discover_prox_sensors() -> Vec<ProxSensor> {
    scan_iio(|dev| {
        let p0 = dev.join("in_proximity0_raw");
        let p1 = dev.join("in_proximity1_raw");
        (p0.exists() && p1.exists()).then_some(ProxSensor { prox_0: p0, prox_1: p1 })
    })
}

fn discover_attention_channels() -> Vec<AttnChannel> {
    scan_iio(|dev| {
        let p = dev.join("in_attention_input");
        p.exists().then_some(AttnChannel { path: p })
    })
}

// ── State types ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Debug)]
enum ProxState {
    VeryNear,
    Near,
    Far,
    Away,
}

impl ProxState {
    fn display(&self) -> &'static str {
        match self {
            ProxState::VeryNear => "Very Near",
            ProxState::Near     => "Near",
            ProxState::Far      => "Far",
            ProxState::Away     => "Away",
        }
    }
}

#[derive(PartialEq, Clone, Debug)]
enum AttentionState {
    Looking,
    NotLooking,
}

// ── Sensor reading ────────────────────────────────────────────────────────────

fn read_raw(path: &PathBuf) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

const PROX1_VERY_NEAR: i64 = 250;
const PROX1_NEAR: i64      = 800;

fn device_prox_state(sensor: &ProxSensor) -> ProxState {
    match read_raw(&sensor.prox_0) {
        Some(1) => match read_raw(&sensor.prox_1) {
            Some(v) if v < PROX1_VERY_NEAR => ProxState::VeryNear,
            Some(v) if v < PROX1_NEAR      => ProxState::Near,
            _                              => ProxState::Far,
        },
        _ => ProxState::Away,
    }
}

fn consensus_prox(sensors: &[ProxSensor]) -> ProxState {
    if sensors.is_empty() { return ProxState::Away }
    let states: Vec<ProxState> = sensors.iter().map(device_prox_state).collect();
    if states.iter().all(|s| *s == ProxState::Away) { return ProxState::Away }

    let n = states.len();
    let vn = states.iter().filter(|s| **s == ProxState::VeryNear).count();
    let nr = states.iter().filter(|s| **s == ProxState::Near).count();
    let fr = states.iter().filter(|s| **s == ProxState::Far).count();
    let half = n / 2;

    if vn > half      { ProxState::VeryNear }
    else if nr > half { ProxState::Near }
    else if fr > half { ProxState::Far }
    else if vn >= nr && vn >= fr { ProxState::VeryNear }
    else if nr >= fr  { ProxState::Near }
    else              { ProxState::Far }
}

fn consensus_attention(channels: &[AttnChannel]) -> AttentionState {
    if channels.is_empty() { return AttentionState::NotLooking }
    let values: Vec<i64> = channels.iter().filter_map(|ch| read_raw(&ch.path)).collect();
    if values.iter().any(|&v| v == 100) { AttentionState::Looking } else { AttentionState::NotLooking }
}

// ── Action execution ──────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum DimMethod { Brightnessctl, GnomeDbus }

fn detect_dim_method() -> DimMethod {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("gnome") || process_running("gnome-shell") {
        DimMethod::GnomeDbus
    } else {
        DimMethod::Brightnessctl
    }
}

fn do_dim(pct: u8, method: DimMethod) {
    match method {
        DimMethod::GnomeDbus => {
            let _ = Command::new("gdbus")
                .args(["call", "--session",
                    "--dest", "org.gnome.Shell",
                    "--object-path", "/org/gnome/Shell/Brightness",
                    "--method", "org.gnome.Shell.Brightness.SetAutoBrightnessTarget",
                    &format!("{}", pct as f64 / 100.0)])
                .spawn();
        }
        DimMethod::Brightnessctl => {
            let _ = Command::new("brightnessctl")
                .args(["--save", "set", &format!("{}%", pct)])
                .spawn();
        }
    }
}

fn do_restore(method: DimMethod) {
    match method {
        DimMethod::GnomeDbus => {
            let _ = Command::new("gdbus")
                .args(["call", "--session",
                    "--dest", "org.gnome.Shell",
                    "--object-path", "/org/gnome/Shell/Brightness",
                    "--method", "org.gnome.Shell.Brightness.SetAutoBrightnessTarget",
                    "1.0"])
                .spawn();
        }
        DimMethod::Brightnessctl => {
            let _ = Command::new("brightnessctl").args(["--restore"]).spawn();
        }
    }
}

fn session_is_locked() -> bool {
    Command::new("loginctl")
        .args(["show-session", "-p", "LockedHint"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("LockedHint=yes"))
        .unwrap_or(false)
}

fn execute_action(action: &str, arg: u8, custom: &str) {
    let dim = detect_dim_method();
    match action {
        "lock" => {
            if session_is_locked() { return }
            let _ = Command::new("loginctl").arg("lock-session").spawn();
        }
        "wake" => wake_display(),
        "dim" => do_dim(arg, dim),
        "brighten" => do_restore(dim),
        "custom" if !custom.is_empty() => { let _ = Command::new("sh").args(["-c", custom]).spawn(); }
        _ => {}
    }
}

fn wake_via_uinput() {
    use libc::{c_int, c_ulong, c_void, ioctl, write};
    use std::os::unix::io::AsRawFd;

    #[repr(C)]
    struct InputEvent { tv_sec: i64, tv_usec: i64, type_: u16, code: u16, value: i32 }

    const EV_SYN: u16 = 0; const EV_KEY: u16 = 1; const SYN_REPORT: u16 = 0;
    const KEY_WAKEUP: u16 = 143;
    const UI_SET_EVBIT:  c_ulong = 0x40045564;
    const UI_SET_KEYBIT: c_ulong = 0x40045565;
    const UI_DEV_CREATE: c_ulong = 0x00005501;
    const UI_DEV_DESTROY:c_ulong = 0x00005502;

    let Ok(file) = std::fs::OpenOptions::new().write(true).open("/dev/uinput") else {
        eprintln!("[omnibookd] warning: cannot open /dev/uinput — add udev rule or join 'input' group");
        return;
    };
    let fd = file.as_raw_fd();
    unsafe {
        ioctl(fd, UI_SET_EVBIT,  EV_SYN  as c_int);
        ioctl(fd, UI_SET_EVBIT,  EV_KEY  as c_int);
        ioctl(fd, UI_SET_KEYBIT, KEY_WAKEUP as c_int);
        let mut dev = [0u8; 1116];
        dev[..14].copy_from_slice(b"omnibookd-wake");
        dev[80] = 3;
        write(fd, dev.as_ptr() as *const c_void, dev.len());
        ioctl(fd, UI_DEV_CREATE);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut ev = |type_: u16, code: u16, value: i32| {
            let e = InputEvent { tv_sec: 0, tv_usec: 0, type_, code, value };
            write(fd, &e as *const _ as *const c_void, std::mem::size_of::<InputEvent>());
        };
        ev(EV_KEY, KEY_WAKEUP, 1); ev(EV_SYN, SYN_REPORT, 0);
        ev(EV_KEY, KEY_WAKEUP, 0); ev(EV_SYN, SYN_REPORT, 0);
        std::thread::sleep(std::time::Duration::from_millis(50));
        ioctl(fd, UI_DEV_DESTROY);
    }
}

fn process_running(name: &str) -> bool {
    Command::new("pgrep").args(["-x", name]).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

fn wake_display() {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();

    let is_gnome = desktop.contains("gnome") || process_running("gnome-shell");
    let is_kde = desktop.contains("kde") || desktop.contains("plasma") || process_running("plasmashell");

    if is_gnome {
        wake_via_uinput();
    } else if is_kde {
        let _ = Command::new("qdbus")
            .args(["org.kde.screensaver", "/ScreenSaver", "SimulateUserActivity"])
            .spawn();
    } else {
        eprintln!("[omnibookd] warning: unknown desktop, wake skipped (use custom command)");
    }
}

// ── Main sensor loop ──────────────────────────────────────────────────────────

fn run_sensor_loop(prox_sensors: Vec<ProxSensor>, attn_channels: Vec<AttnChannel>) {
    let cfg_path = presence_config_path();
    let socket_path = daemon_socket_path();
    let mut ipc = DaemonIpc::bind(&socket_path)
        .map_err(|e| eprintln!("[omnibookd] IPC socket bind failed: {e}"))
        .ok();

    // Wait for IIO subsystem to settle after boot (SYNA3580 takes ~2s)
    thread::sleep(Duration::from_secs(3));

    let init_prox = consensus_prox(&prox_sensors);
    let init_attn = consensus_attention(&attn_channels);
    let mut cfg = PresenceConfig::load(&cfg_path);
    let mut prev_is_away = cfg.prox_meets_away_threshold(init_prox.display());
    let mut prev_is_notlooking = matches!(init_attn, AttentionState::NotLooking);

    let mut away_pending:       Option<Instant> = None;
    let mut away_fired                          = false;
    let mut notlooking_pending: Option<Instant> = None;
    let mut notlooking_fired                    = false;
    let mut screen_dimmed                       = false;

    loop {
        // ── IPC: accept clients, apply any incoming config updates ────────────
        if let Some(ref mut ipc) = ipc {
            for new_cfg in ipc.accept_and_drain() {
                new_cfg.save_atomic(&cfg_path);
                cfg = new_cfg;
            }
        } else {
            // Fallback when IPC is unavailable: reload config from file each tick
            cfg = PresenceConfig::load(&cfg_path);
        }

        // ── Read sensors ──────────────────────────────────────────────────────
        let prox_now     = consensus_prox(&prox_sensors);
        let attn_now     = consensus_attention(&attn_channels);
        let prox_display = prox_now.display();
        let attn_display = if matches!(attn_now, AttentionState::Looking) { "Looking" } else { "Not Looking" };

        write_state(prox_display, attn_display);
        if let Some(ref mut ipc) = ipc {
            ipc.push_state(prox_display, attn_display);
        }

        // ── Proximity ────────────────────────────────────────────────────────
        let is_away = cfg.prox_meets_away_threshold(prox_display);

        if is_away != prev_is_away {
            if is_away {
                away_pending = Some(Instant::now() + Duration::from_secs(cfg.prox_away_delay as u64));
                away_fired = false;
            } else {
                away_pending = None;
                if away_fired {
                    execute_action(&cfg.prox_return_action.clone(), 0, &cfg.prox_return_custom.clone());
                }
                away_fired = false;
            }
            prev_is_away = is_away;
        }

        if is_away && !away_fired {
            if let Some(fire_at) = away_pending {
                if Instant::now() >= fire_at {
                    execute_action(&cfg.prox_away_action.clone(), 0, &cfg.prox_away_custom.clone());
                    away_fired = true;
                    away_pending = None;
                }
            }
        }

        // ── Attention ────────────────────────────────────────────────────────
        let is_notlooking = matches!(attn_now, AttentionState::NotLooking);

        if is_notlooking != prev_is_notlooking {
            if is_notlooking {
                notlooking_pending = Some(Instant::now() + Duration::from_secs(cfg.attn_notlooking_delay as u64));
                notlooking_fired = false;
            } else {
                notlooking_pending = None;
                if notlooking_fired {
                    if screen_dimmed {
                        do_restore(detect_dim_method());
                        screen_dimmed = false;
                    }
                    execute_action(&cfg.attn_looking_action.clone(), 0, &cfg.attn_looking_custom.clone());
                }
                notlooking_fired = false;
            }
            prev_is_notlooking = is_notlooking;
        }

        if is_notlooking && !notlooking_fired {
            if let Some(fire_at) = notlooking_pending {
                if Instant::now() >= fire_at {
                    let action = cfg.attn_notlooking_action.clone();
                    execute_action(&action, cfg.attn_notlooking_dim, &cfg.attn_notlooking_custom.clone());
                    if action == "dim" { screen_dimmed = true; }
                    notlooking_fired = true;
                    notlooking_pending = None;
                }
            }
        }

        thread::sleep(Duration::from_millis(500));
    }
}
