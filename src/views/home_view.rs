use adw::{prelude::*, ActionRow, ApplicationWindow, PreferencesGroup};
use gtk::{Box, Picture};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use super::ui;

pub struct HomeView {
    pub widget: Box,
}

impl HomeView {
    pub fn new(_window: &ApplicationWindow) -> Self {
        let (widget, inner) = ui::make_layout();
        let lscpu = Self::load_lscpu();
        Self::fill_content(&inner, &lscpu);
        Self { widget }
    }

    fn fill_content(inner_box: &Box, lscpu: &HashMap<String, String>) {
        inner_box.append(&Self::make_logo());

        // System Overview
        let overview = PreferencesGroup::builder().title("System Overview").build();
        for (title, subtitle) in [
            ("Device Model", Self::get_device_model()),
            ("Operating System", Self::get_os()),
            ("Kernel", Self::get_kernel()),
            ("Memory", Self::get_ram()),
        ] {
            overview.add(
                &ActionRow::builder()
                    .title(title)
                    .subtitle(subtitle)
                    .build(),
            );
        }
        inner_box.append(&overview);

        // CPU
        let cpu_group = PreferencesGroup::builder().title("CPU").build();
        for (title, subtitle) in [
            ("Model", Self::get_cpu_model(lscpu)),
            (
                "Sockets",
                lscpu
                    .get("socket(s)")
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string()),
            ),
            (
                "Virtual Processors",
                lscpu
                    .get("cpu(s)")
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string()),
            ),
            ("Clock (current/base/max)", Self::get_cpu_clocks(lscpu)),
            ("Cache (L1/L2/L3)", Self::get_cpu_cache(lscpu)),
            (
                "cpufreq Driver",
                Self::read_sysfs("/sys/devices/system/cpu/cpufreq/policy0/scaling_driver"),
            ),
            (
                "cpufreq Governor",
                Self::read_sysfs("/sys/devices/system/cpu/cpufreq/policy0/scaling_governor"),
            ),
        ] {
            cpu_group.add(
                &ActionRow::builder()
                    .title(title)
                    .subtitle(subtitle)
                    .build(),
            );
        }
        inner_box.append(&cpu_group);

        // GPU
        let gpu_group = PreferencesGroup::builder().title("GPU").build();
        gpu_group.add(
            &ActionRow::builder()
                .title("Model")
                .subtitle(Self::get_gpu_model())
                .build(),
        );
        inner_box.append(&gpu_group);
    }

    fn make_logo() -> Box {
        let logo_box = Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();

        let logo_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("hp-logo.svg");

        if logo_path.exists() {
            let picture = Picture::for_filename(logo_path.to_string_lossy().as_ref());
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_size_request(72, 72);
            logo_box.append(&picture);
        } else {
            logo_box.append(&adw::Avatar::new(64, Some("HP"), true));
        }

        let label = gtk::Label::new(Some("HP OmniBook"));
        label.add_css_class("title-3");
        logo_box.append(&label);

        logo_box
    }

    fn get_device_model() -> String {
        let val = Self::read_sysfs("/sys/devices/virtual/dmi/id/product_name");
        if val.is_empty() || val.to_lowercase() == "unknown" {
            "Unknown".to_string()
        } else {
            val
        }
    }

    fn get_os() -> String {
        if let Ok(data) = fs::read_to_string("/etc/os-release") {
            for line in data.lines() {
                if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                    return rest.trim().trim_matches('"').to_string();
                }
            }
        }
        "Unknown".to_string()
    }

    fn get_kernel() -> String {
        Self::read_sysfs("/proc/sys/kernel/osrelease")
    }

    fn get_ram() -> String {
        if let Ok(data) = fs::read_to_string("/proc/meminfo") {
            for line in data.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let kb: f64 = rest
                        .trim()
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let gb = kb / 1024.0 / 1024.0;
                    return format!("{:.1} GB", gb);
                }
            }
        }
        "Unknown".to_string()
    }

    fn get_cpu_model(lscpu: &HashMap<String, String>) -> String {
        if let Some(model) = lscpu.get("model name") {
            return model.clone();
        }
        if let Ok(data) = fs::read_to_string("/proc/cpuinfo") {
            for line in data.lines() {
                if line.to_lowercase().starts_with("model name") {
                    if let Some(rest) = line.splitn(2, ':').nth(1) {
                        return rest.trim().to_string();
                    }
                }
            }
        }
        "Unknown".to_string()
    }

    fn get_cpu_clocks(lscpu: &HashMap<String, String>) -> String {
        let current = Self::get_current_mhz(lscpu);
        let base = lscpu
            .get("cpu min mhz")
            .map(|s| Self::fmt_freq(s))
            .unwrap_or_else(|| "Unknown".to_string());
        let max = lscpu
            .get("cpu max mhz")
            .map(|s| Self::fmt_freq(s))
            .unwrap_or_else(|| "Unknown".to_string());
        format!("{} / {} / {}", current, base, max)
    }

    fn get_current_mhz(lscpu: &HashMap<String, String>) -> String {
        if let Ok(data) = fs::read_to_string("/proc/cpuinfo") {
            for line in data.lines() {
                if line.to_lowercase().starts_with("cpu mhz") {
                    if let Some(val) = line.splitn(2, ':').nth(1) {
                        return Self::fmt_freq(val.trim());
                    }
                }
            }
        }
        lscpu
            .get("cpu mhz")
            .map(|s| Self::fmt_freq(s))
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn get_cpu_cache(lscpu: &HashMap<String, String>) -> String {
        let l1d = lscpu.get("l1d cache");
        let l1i = lscpu.get("l1i cache");
        let l2 = lscpu.get("l2 cache");
        let l3 = lscpu.get("l3 cache");

        let l1 = match (l1d, l1i) {
            (Some(d), Some(i)) => Some(format!("L1d {} / L1i {}", d, i)),
            (Some(d), None) => Some(d.clone()),
            (None, Some(i)) => Some(i.clone()),
            (None, None) => None,
        };

        let parts: Vec<String> = [
            l1,
            l2.map(|s| format!("L2 {}", s)),
            l3.map(|s| format!("L3 {}", s)),
        ]
        .into_iter()
        .flatten()
        .collect();

        if parts.is_empty() {
            "Unknown".to_string()
        } else {
            parts.join(" | ")
        }
    }

    fn get_gpu_model() -> String {
        if let Ok(output) = Command::new("lspci").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let lower = line.to_lowercase();
                if lower.contains("vga")
                    || lower.contains("3d controller")
                    || lower.contains("display controller")
                {
                    // Content inside square brackets is the friendly name
                    if let (Some(s), Some(e)) = (line.find('['), line.find(']')) {
                        let name = line[s + 1..e].trim();
                        if !name.is_empty() {
                            return name.to_string();
                        }
                    }
                    // Fallback: third colon-delimited field
                    let parts: Vec<&str> = line.splitn(3, ':').collect();
                    if parts.len() >= 3 {
                        let candidate = parts[2].split('(').next().unwrap_or("").trim();
                        if !candidate.is_empty() {
                            return candidate.to_string();
                        }
                    }
                }
            }
        }
        "Unknown".to_string()
    }

    fn fmt_freq(val: &str) -> String {
        match val.trim().parse::<f64>() {
            Ok(mhz) => format!("{:.0} MHz", mhz),
            Err(_) => val.trim().to_string(),
        }
    }

    fn read_sysfs(path: &str) -> String {
        fs::read_to_string(path)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "Unknown".to_string())
    }

    fn load_lscpu() -> HashMap<String, String> {
        let mut info = HashMap::new();
        if let Ok(output) = Command::new("lscpu").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.is_empty() || !line.contains(':') {
                    continue;
                }
                let mut parts = line.splitn(2, ':');
                let key = parts.next().unwrap_or("").trim().to_lowercase();
                let val = parts.next().unwrap_or("").trim().to_string();
                info.insert(key, val);
            }
        }
        info
    }
}
