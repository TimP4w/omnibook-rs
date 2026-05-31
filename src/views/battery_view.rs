use adw::{prelude::*, ActionRow, ApplicationWindow, PreferencesGroup};
use gtk::{cairo, Box, CheckButton, DrawingArea, Image};
use omnibook_rs::sysfs::read_sysfs_opt;
use super::ui;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Ring buffer of (timestamp, battery_pct) samples.
type History = Rc<RefCell<VecDeque<(Instant, f64)>>>;
// Max entries retained in the live ring (30s × 960 = 8 h of live samples)
const MAX_SAMPLES: usize = 960;
// How far back to seed from UPower history on startup (8 hours)
const HISTORY_WINDOW_SECS: u64 = 8 * 3600;

pub struct BatteryView {
    pub widget: Box,
}

impl BatteryView {
    pub fn new(_window: &ApplicationWindow) -> Self {
        let (widget, inner_box) = ui::make_layout();

        let battery_path = Self::find_battery_path();

        // Battery Status group
        let status_group = PreferencesGroup::builder().title("Battery Status").build();
        inner_box.append(&status_group);

        let charge_icon = Image::new();
        let charge_row = ActionRow::builder().title("Battery Level").build();
        charge_row.add_suffix(&charge_icon);
        status_group.add(&charge_row);

        let health_row = ActionRow::builder().title("Battery Health").build();
        status_group.add(&health_row);

        // Charge History graph — seeded from UPower persistent history on startup.
        let history: History = {
            let initial = Self::find_upower_history(&battery_path)
                .map(|p| Self::load_upower_history(&p, HISTORY_WINDOW_SECS))
                .unwrap_or_default();
            let mut dq = VecDeque::with_capacity(initial.len() + MAX_SAMPLES);
            dq.extend(initial);
            Rc::new(RefCell::new(dq))
        };

        let graph_label = gtk::Label::builder()
            .label("Charge History")
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build();
        inner_box.append(&graph_label);

        let drawing_area = DrawingArea::new();
        drawing_area.set_content_height(160);
        drawing_area.set_hexpand(true);
        drawing_area.add_css_class("card");

        let history_for_draw = history.clone();
        drawing_area.set_draw_func(move |_area, cr, width, height| {
            Self::draw_history(&history_for_draw.borrow(), cr, width, height);
        });
        inner_box.append(&drawing_area);

        // Battery Info group
        let meta_group = PreferencesGroup::builder().title("Battery Info").build();
        inner_box.append(&meta_group);

        let vendor_row = ActionRow::builder().title("Vendor").build();
        let model_row = ActionRow::builder().title("Model").build();
        let serial_row = ActionRow::builder().title("Serial Number").build();
        let tech_row = ActionRow::builder().title("Technology").build();
        let cycles_row = ActionRow::builder().title("Cycle Count").build();
        meta_group.add(&vendor_row);
        meta_group.add(&model_row);
        meta_group.add(&serial_row);
        meta_group.add(&tech_row);
        meta_group.add(&cycles_row);

        // Power Profiles group
        let profile_group = PreferencesGroup::builder().title("Power Profile").build();
        inner_box.append(&profile_group);

        const PROFILES: &[(&str, &str, &str)] = &[
            ("balanced",    "Balanced",     "Standard performance and power usage"),
            ("performance", "Performance",  "Maximum performance, higher power usage"),
            ("power-saver", "Power Saver",  "Reduced performance, extended battery life"),
        ];
        let anchor_check = CheckButton::new();
        let profile_checks: Vec<CheckButton> = std::iter::once(anchor_check.clone())
            .chain(PROFILES[1..].iter().map(|_| CheckButton::builder().group(&anchor_check).build()))
            .collect();

        // Energy and timing group
        let energy_group = PreferencesGroup::builder()
            .title("Energy and Timing")
            .build();
        inner_box.append(&energy_group);

        let time_row = ActionRow::builder().title("Time Estimate").build();
        energy_group.add(&time_row);

        let energy_row = ActionRow::builder()
            .title("Energy (Design / Full / Now)")
            .build();
        energy_group.add(&energy_row);

        let rate_row = ActionRow::builder().title("Energy Transfer Rate").build();
        energy_group.add(&rate_row);

        let voltage_row = ActionRow::builder()
            .title("Voltage (Min / Now)")
            .build();
        energy_group.add(&voltage_row);

        // Build profile rows and set initial state before connecting signals
        let current_profile = Self::get_power_profile();
        for (&(id, title, subtitle), check) in PROFILES.iter().zip(profile_checks.iter()) {
            check.set_active(current_profile == id);
            let row = ActionRow::builder()
                .title(title).subtitle(subtitle).activatable_widget(check).build();
            row.add_prefix(check);
            profile_group.add(&row);
        }

        // Initial battery info update
        Self::update_all(
            &battery_path,
            &charge_row,
            &charge_icon,
            &health_row,
            &vendor_row,
            &model_row,
            &serial_row,
            &tech_row,
            &cycles_row,
            &time_row,
            &energy_row,
            &rate_row,
            &voltage_row,
            &history,
            &drawing_area,
        );

        // Connect power profile toggles (after initial state is set)
        for (&(id, ..), check) in PROFILES.iter().zip(profile_checks.iter()) {
            check.connect_toggled(move |btn| {
                if btn.is_active() {
                    let _ = Command::new("powerprofilesctl").args(["set", id]).status();
                }
            });
        }

        // 30s refresh timer for battery info
        let (cr, ci, hr) = (charge_row.clone(), charge_icon.clone(), health_row.clone());
        let (vr, mr, sr, tr, cyr) = (
            vendor_row.clone(),
            model_row.clone(),
            serial_row.clone(),
            tech_row.clone(),
            cycles_row.clone(),
        );
        let (tir, er, rr, vor) = (
            time_row.clone(),
            energy_row.clone(),
            rate_row.clone(),
            voltage_row.clone(),
        );
        let bp = battery_path.clone();
        let hist_cb = history.clone();
        let da_cb = drawing_area.clone();
        gtk::glib::timeout_add_seconds_local(30, move || {
            Self::update_all(&bp, &cr, &ci, &hr, &vr, &mr, &sr, &tr, &cyr, &tir, &er, &rr, &vor, &hist_cb, &da_cb);
            gtk::glib::ControlFlow::Continue
        });

        Self { widget }
    }

    fn find_battery_path() -> Option<PathBuf> {
        let base = Path::new("/sys/class/power_supply");
        let batteries: Vec<(PathBuf, String)> = fs::read_dir(base).ok()?.flatten()
            .filter_map(|e| {
                let p = e.path();
                let is_battery = fs::read_to_string(p.join("type"))
                    .map(|s| s.trim().to_lowercase() == "battery").unwrap_or(false);
                if !is_battery { return None; }
                let scope = fs::read_to_string(p.join("scope"))
                    .map(|s| s.trim().to_lowercase()).unwrap_or_default();
                Some((p, scope))
            })
            .collect();
        // Prefer scope=system/primary (excludes peripheral batteries); fall back to any non-device
        batteries.iter().find(|(_, s)| s == "system" || s == "primary").map(|(p, _)| p.clone())
            .or_else(|| batteries.iter().find(|(_, s)| s != "device").map(|(p, _)| p.clone()))
    }

    fn read_float(path: &Path) -> Option<f64> {
        read_sysfs_opt(path).and_then(|s| s.parse().ok())
    }

    fn update_all(
        battery_path: &Option<PathBuf>,
        charge_row: &ActionRow,
        charge_icon: &Image,
        health_row: &ActionRow,
        vendor_row: &ActionRow,
        model_row: &ActionRow,
        serial_row: &ActionRow,
        tech_row: &ActionRow,
        cycles_row: &ActionRow,
        time_row: &ActionRow,
        energy_row: &ActionRow,
        rate_row: &ActionRow,
        voltage_row: &ActionRow,
        history: &History,
        drawing_area: &DrawingArea,
    ) {
        let bp = match battery_path {
            Some(p) => p,
            None => {
                charge_row.set_subtitle("Battery not found");
                charge_icon.set_icon_name(Some("battery-missing-symbolic"));
                health_row.set_subtitle("-");
                return;
            }
        };

        let capacity = read_sysfs_opt(&bp.join("capacity"));
        let status = read_sysfs_opt(&bp.join("status"));
        let health = read_sysfs_opt(&bp.join("health"));

        // Metadata
        vendor_row.set_subtitle(
            read_sysfs_opt(&bp.join("manufacturer"))
                .as_deref()
                .unwrap_or("-"),
        );
        model_row.set_subtitle(
            read_sysfs_opt(&bp.join("model_name"))
                .as_deref()
                .unwrap_or("-"),
        );
        serial_row.set_subtitle(
            read_sysfs_opt(&bp.join("serial_number"))
                .as_deref()
                .unwrap_or("-"),
        );
        tech_row.set_subtitle(
            read_sysfs_opt(&bp.join("technology"))
                .as_deref()
                .unwrap_or("-"),
        );
        cycles_row.set_subtitle(
            read_sysfs_opt(&bp.join("cycle_count"))
                .as_deref()
                .unwrap_or("-"),
        );

        // Health: use sysfs value or compute from energy ratio
        let health_display = health.unwrap_or_else(|| {
            let full = Self::read_float(&bp.join("energy_full"));
            let design = Self::read_float(&bp.join("energy_full_design"));
            match (full, design) {
                (Some(f), Some(d)) if d > 0.0 => {
                    let pct = ((f / d) * 100.0).clamp(0.0, 100.0) as i32;
                    format!("~{}% of design", pct)
                }
                _ => "-".to_string(),
            }
        });
        health_row.set_subtitle(&health_display);

        // Charge level and status
        let charge_text = match &capacity {
            Some(cap) => {
                let status_str = status
                    .as_deref()
                    .map(|s| {
                        let mut c = s.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().to_string() + c.as_str(),
                        }
                    })
                    .unwrap_or_default();
                if status_str.is_empty() {
                    format!("{}%", cap)
                } else {
                    format!("{}% - {}", cap, status_str)
                }
            }
            None => "Unknown".to_string(),
        };
        charge_row.set_subtitle(&charge_text);
        charge_icon
            .set_icon_name(Some(&Self::icon_for_level(capacity.as_deref(), status.as_deref())));

        // Record sample in history and trigger graph redraw
        if let Some(pct) = capacity.as_deref().and_then(|s| s.parse::<f64>().ok()) {
            let mut hist = history.borrow_mut();
            hist.push_back((Instant::now(), pct));
            if hist.len() > MAX_SAMPLES {
                hist.pop_front();
            }
            drop(hist);
            drawing_area.queue_draw();
        }

        // Energy and timing
        Self::update_energy_and_time(bp, &status, time_row, energy_row, rate_row, voltage_row);
    }

    fn draw_history(
        history: &VecDeque<(Instant, f64)>,
        cr: &cairo::Context,
        width: i32,
        height: i32,
    ) {
        let w = width as f64;
        let h = height as f64;
        let pad_top = 8.0;
        let pad_right = 12.0;
        let pad_bottom = 18.0; // room for the time-span label
        let pad_left = 40.0;   // room for y-axis labels
        let plot_w = w - pad_left - pad_right;
        let plot_h = h - pad_top - pad_bottom;

        if history.len() < 2 {
            return;
        }

        let t0 = history.front().unwrap().0;
        let t1 = history.back().unwrap().0;
        let time_span = t1.duration_since(t0).as_secs_f64();
        if time_span <= 0.0 {
            return;
        }

        // Y-axis labels and grid lines at 0 / 25 / 50 / 75 / 100 %
        cr.set_font_size(10.0);
        for &pct in &[0.0_f64, 25.0, 50.0, 75.0, 100.0] {
            let y = pad_top + (1.0 - pct / 100.0) * plot_h;

            // Grid line
            let alpha = if pct == 0.0 || pct == 50.0 || pct == 100.0 { 0.35 } else { 0.18 };
            cr.set_source_rgba(0.5, 0.5, 0.5, alpha);
            cr.set_line_width(1.0);
            cr.move_to(pad_left, y);
            cr.line_to(w - pad_right, y);
            let _ = cr.stroke();

            // Label — right-aligned just left of the plot area
            let label = format!("{}%", pct as i32);
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.8);
            let ext = match cr.text_extents(&label) {
                Ok(e) => e,
                Err(_) => continue,
            };
            // Clamp so the top label doesn't clip above the widget
            let text_y = (y + ext.height() / 2.0).clamp(ext.height(), h - 2.0);
            cr.move_to(pad_left - ext.width() - 4.0, text_y);
            let _ = cr.show_text(&label);
        }

        // Filled area under the charge line for visual weight
        let current_pct = history.back().unwrap().1;
        let (r, g, b) = if current_pct >= 40.0 {
            (0.35_f64, 0.75_f64, 0.35_f64)
        } else if current_pct >= 15.0 {
            (0.9_f64, 0.65_f64, 0.1_f64)
        } else {
            (0.85_f64, 0.2_f64, 0.2_f64)
        };

        let x_for = |t: Instant| -> f64 {
            pad_left + (t.duration_since(t0).as_secs_f64() / time_span) * plot_w
        };
        let y_for = |pct: f64| -> f64 { pad_top + (1.0 - pct / 100.0) * plot_h };

        let last = history.back().unwrap();
        let x_last = x_for(last.0);

        // Fill under curve
        cr.set_source_rgba(r, g, b, 0.12);
        let y_bottom = pad_top + plot_h;
        cr.move_to(pad_left, y_bottom);
        for (t, pct) in history.iter() {
            cr.line_to(x_for(*t), y_for(*pct));
        }
        cr.line_to(x_last, y_bottom);
        cr.close_path();
        let _ = cr.fill();

        // Charge line
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.0);
        let _ = cr.set_line_join(cairo::LineJoin::Round);
        let _ = cr.set_line_cap(cairo::LineCap::Round);
        for (i, (t, pct)) in history.iter().enumerate() {
            let x = x_for(*t);
            let y = y_for(*pct);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        let _ = cr.stroke();

        // Dot at current position
        cr.arc(x_last, y_for(last.1), 4.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();

        // Projected line via linear regression on the history.
        // Requires at least 3 samples spanning ≥ 5 minutes for a stable slope.
        if history.len() >= 3 && time_span >= 300.0 {
            // Compute slope (pct/sec) using least-squares over all samples.
            let n = history.len() as f64;
            let mut sum_x = 0.0_f64;
            let mut sum_y = 0.0_f64;
            let mut sum_xx = 0.0_f64;
            let mut sum_xy = 0.0_f64;
            for (t, pct) in history.iter() {
                let x = t.duration_since(t0).as_secs_f64();
                sum_x += x;
                sum_y += pct;
                sum_xx += x * x;
                sum_xy += x * pct;
            }
            let denom = n * sum_xx - sum_x * sum_x;
            if denom.abs() > 1e-9 {
                let slope = (n * sum_xy - sum_x * sum_y) / denom; // pct per second
                // Only draw if slope is meaningful (> 0.1%/min = 0.00167%/s)
                if slope.abs() > 0.00167 {
                    let target_pct = if slope < 0.0 { 0.0_f64 } else { 100.0_f64 };
                    // Seconds from *now* (t1) until target_pct is reached
                    let secs_to_target = (target_pct - last.1) / slope;
                    if secs_to_target > 0.0 {
                        // Cap projection to 2× the visible history window
                        let secs_shown = secs_to_target.min(time_span * 2.0);
                        let x_proj = x_last + (secs_shown / time_span) * plot_w;
                        let pct_proj = (last.1 + slope * secs_shown).clamp(0.0, 100.0);

                        // Dashed line in the same hue, dimmed
                        cr.set_source_rgba(r, g, b, 0.5);
                        cr.set_line_width(1.5);
                        let _ = cr.set_dash(&[6.0, 4.0], 0.0);
                        let _ = cr.set_line_cap(cairo::LineCap::Butt);
                        cr.move_to(x_last, y_for(last.1));
                        cr.line_to(x_proj, y_for(pct_proj));
                        let _ = cr.stroke();
                        let _ = cr.set_dash(&[], 0.0); // reset dash
                    }
                }
            }
        }

        // Time span label (bottom, after the y-axis labels area)
        let mins = (time_span / 60.0) as u64;
        let span_text = if mins >= 60 {
            format!("{}h {}m shown", mins / 60, mins % 60)
        } else {
            format!("{}m shown", mins)
        };
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.7);
        cr.set_font_size(10.0);
        cr.move_to(pad_left, h - 3.0);
        let _ = cr.show_text(&span_text);

    }

    /// Find the UPower charge history .dat file for the system battery.
    /// Tries to match by sysfs model_name first; falls back to the file with the most valid entries.
    fn find_upower_history(battery_path: &Option<PathBuf>) -> Option<PathBuf> {
        let base = Path::new("/var/lib/upower");
        let entries: Vec<PathBuf> = fs::read_dir(base)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("history-charge-") && n.ends_with(".dat"))
                    .unwrap_or(false)
            })
            .collect();

        // Try matching by battery model name from sysfs
        if let Some(bp) = battery_path {
            let model = fs::read_to_string(bp.join("model_name"))
                .ok()
                .map(|s| s.trim().replace(' ', "_"));
            if let Some(model) = model {
                for entry in &entries {
                    let fname = entry.file_name().unwrap().to_string_lossy().to_string();
                    let stem = fname.strip_prefix("history-charge-").unwrap_or(&fname);
                    if stem.starts_with(&model) {
                        return Some(entry.clone());
                    }
                }
            }
        }

        // Fallback: file with the most non-zero percentage entries
        entries.into_iter().max_by_key(|p| {
            fs::read_to_string(p)
                .ok()
                .map(|content| {
                    content
                        .lines()
                        .filter(|l| {
                            let mut it = l.splitn(3, '\t');
                            it.next(); // timestamp
                            it.next()
                                .and_then(|v| v.parse::<f64>().ok())
                                .map_or(false, |v| v > 0.0)
                        })
                        .count()
                })
                .unwrap_or(0)
        })
    }

    /// Parse a UPower history-charge .dat file and return the entries within the last
    /// `max_age_secs` seconds as `(Instant, percentage)` pairs, oldest first.
    fn load_upower_history(path: &Path, max_age_secs: u64) -> Vec<(Instant, f64)> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now_instant = Instant::now();
        let cutoff = now_unix.saturating_sub(max_age_secs);

        let mut result = Vec::new();
        for line in content.lines() {
            let mut parts = line.splitn(3, '\t');
            let unix_ts: u64 = match parts.next().and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            let pct: f64 = match parts.next().and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            // Skip zero sentinel entries (UPower writes 0% on disconnect events)
            if unix_ts < cutoff || pct <= 0.0 {
                continue;
            }
            let age = Duration::from_secs(now_unix.saturating_sub(unix_ts));
            if let Some(inst) = now_instant.checked_sub(age) {
                result.push((inst, pct));
            }
        }
        result
    }

    fn icon_for_level(capacity: Option<&str>, status: Option<&str>) -> String {
        let level = match capacity.and_then(|s| s.parse::<i32>().ok()) {
            Some(l) => l,
            None => return "battery-missing-symbolic".to_string(),
        };
        let charging = status.map_or(false, |s| s.to_lowercase() == "charging");
        let base = if level >= 90 {
            "battery-full"
        } else if level >= 70 {
            "battery-good"
        } else if level >= 40 {
            "battery-medium"
        } else if level >= 15 {
            "battery-low"
        } else {
            "battery-caution"
        };
        if charging {
            format!("{}-charging-symbolic", base)
        } else {
            format!("{}-symbolic", base)
        }
    }

    fn update_energy_and_time(
        bp: &Path,
        status: &Option<String>,
        time_row: &ActionRow,
        energy_row: &ActionRow,
        rate_row: &ActionRow,
        voltage_row: &ActionRow,
    ) {
        let mut energy_now = Self::read_float(&bp.join("energy_now"));
        let mut energy_full = Self::read_float(&bp.join("energy_full"));
        let mut energy_design = Self::read_float(&bp.join("energy_full_design"));
        let mut power_now = Self::read_float(&bp.join("power_now"));
        let voltage_now = Self::read_float(&bp.join("voltage_now"));
        let voltage_min = Self::read_float(&bp.join("voltage_min_design"));

        // Fallbacks via charge values
        if energy_now.is_none() {
            if let (Some(cn), Some(vn)) =
                (Self::read_float(&bp.join("charge_now")), voltage_now)
            {
                energy_now = Some(cn * vn / 1_000_000.0);
            }
        }
        if energy_full.is_none() {
            if let (Some(cf), Some(vn)) =
                (Self::read_float(&bp.join("charge_full")), voltage_now)
            {
                energy_full = Some(cf * vn / 1_000_000.0);
            }
        }
        if energy_design.is_none() {
            if let (Some(cd), Some(vn)) =
                (Self::read_float(&bp.join("charge_full_design")), voltage_now)
            {
                energy_design = Some(cd * vn / 1_000_000.0);
            }
        }
        if power_now.is_none() {
            if let (Some(cn), Some(vn)) =
                (Self::read_float(&bp.join("current_now")), voltage_now)
            {
                power_now = Some(cn * vn / 1_000_000.0);
            }
        }

        let fmt_wh = |val: Option<f64>| match val {
            Some(v) => format!("{:.3} Wh", v / 1_000_000.0),
            None => "-".to_string(),
        };
        energy_row.set_subtitle(&format!(
            "{} / {} / {}",
            fmt_wh(energy_design),
            fmt_wh(energy_full),
            fmt_wh(energy_now)
        ));

        rate_row.set_subtitle(&match power_now {
            Some(p) => format!("{:.3} W", p / 1_000_000.0),
            None => "-".to_string(),
        });

        let is_discharging = status.as_deref().map_or(false, |s| s.to_lowercase() == "discharging");
        let is_charging = status.as_deref().map_or(false, |s| s.to_lowercase() == "charging");

        let time_text = match (power_now, energy_now, energy_full) {
            (Some(pn), Some(en), Some(ef)) if pn > 0.0 => {
                let pw = pn / 1_000_000.0;
                let ew = en / 1_000_000.0;
                let efw = ef / 1_000_000.0;
                if is_charging {
                    let remaining = (efw - ew).max(0.0);
                    format!("To full: {}", Self::fmt_hours(remaining / pw))
                } else if is_discharging {
                    format!("To empty: {}", Self::fmt_hours(ew / pw))
                } else {
                    "-".to_string()
                }
            }
            _ if is_charging || is_discharging => "Calculating…".to_string(),
            _ => "-".to_string(),
        };
        time_row.set_subtitle(&time_text);

        let v_min = voltage_min
            .map(|v| format!("{:.3} V", v / 1_000_000.0))
            .unwrap_or_else(|| "-".to_string());
        let v_now = voltage_now
            .map(|v| format!("{:.3} V", v / 1_000_000.0))
            .unwrap_or_else(|| "-".to_string());
        voltage_row.set_subtitle(&format!("{} / {}", v_min, v_now));
    }

    fn fmt_hours(hours: f64) -> String {
        let mins = (hours * 60.0) as i32;
        format!("{}h {}m", mins / 60, mins % 60)
    }

    fn get_power_profile() -> String {
        Command::new("powerprofilesctl")
            .arg("get")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "balanced".to_string())
    }
}
