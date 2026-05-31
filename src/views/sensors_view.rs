use adw::{prelude::*, ActionRow, ApplicationWindow, PreferencesGroup};
use gtk::Box;
use gtk::Label;
use super::ui;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct SensorsView {
    pub widget: Box,
}

struct TempSensor {
    label: String,
    chip: String,
    path: PathBuf,
}

struct IioChannel {
    name: String,
    device: String,
    device_id: String, // e.g. "iio:device0" — used to disambiguate same-named devices
    raw_path: PathBuf,
    scale_path: Option<PathBuf>,
}

impl SensorsView {
    pub fn new(_window: &ApplicationWindow) -> Self {
        let (widget, inner_box) = ui::make_layout();

        // Discover sensors synchronously (fast sysfs reads)
        let temp_sensors = Self::discover_temp_sensors();
        let iio_channels = Self::discover_iio_channels();

        // Build temperature sensor rows, grouped by chip
        // Separate paths (sent to background thread) from labels (stay on main thread)
        let mut temp_paths: Vec<PathBuf> = Vec::new();
        let mut temp_labels: Vec<Label> = Vec::new();

        if temp_sensors.is_empty() {
            let group = PreferencesGroup::builder()
                .title("Temperature Sensors")
                .build();
            group.add(&ActionRow::builder().title("No temperature sensors found").build());
            inner_box.append(&group);
        } else {
            let mut chips: Vec<String> = Vec::new();
            let mut by_chip: std::collections::HashMap<String, Vec<&TempSensor>> =
                std::collections::HashMap::new();
            for sensor in &temp_sensors {
                by_chip.entry(sensor.chip.clone()).or_default().push(sensor);
                if !chips.contains(&sensor.chip) {
                    chips.push(sensor.chip.clone());
                }
            }
            for chip in &chips {
                let group = PreferencesGroup::builder().title(chip.as_str()).build();
                if let Some(sensors) = by_chip.get(chip) {
                    for sensor in sensors {
                        let val_label = Self::make_sensor_row(&group, &sensor.label, chip);
                        temp_paths.push(sensor.path.clone());
                        temp_labels.push(val_label);
                    }
                }
                inner_box.append(&group);
            }
        }

        // Build IIO sensor rows, grouped by device
        // Separate channel data (sent to background thread) from labels (stay on main thread)
        let mut iio_data: Vec<IioChannel> = Vec::new();
        let mut iio_labels: Vec<Label> = Vec::new();

        if iio_channels.is_empty() {
            let group = PreferencesGroup::builder().title("IIO Sensors").build();
            group.add(&ActionRow::builder().title("No IIO sensors found").build());
            inner_box.append(&group);
        } else {
            // Group by device_id (unique per physical device).
            // If multiple devices share the same display name, append the device_id to distinguish.
            let mut device_ids: Vec<String> = Vec::new();
            let mut by_device_id: std::collections::HashMap<String, Vec<usize>> =
                std::collections::HashMap::new();
            for (i, ch) in iio_channels.iter().enumerate() {
                by_device_id.entry(ch.device_id.clone()).or_default().push(i);
                if !device_ids.contains(&ch.device_id) {
                    device_ids.push(ch.device_id.clone());
                }
            }
            // Count how many device_ids share each display name, for disambiguation
            let mut name_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for id in &device_ids {
                if let Some(i) = by_device_id.get(id).and_then(|v| v.first()) {
                    *name_count.entry(iio_channels[*i].device.clone()).or_insert(0) += 1;
                }
            }
            for device_id in &device_ids {
                let indices = by_device_id.get(device_id).unwrap();
                let display_name = &iio_channels[indices[0]].device;
                let group_title = if *name_count.get(display_name).unwrap_or(&1) > 1 {
                    format!("{} ({})", display_name, device_id)
                } else {
                    display_name.clone()
                };
                let group = PreferencesGroup::builder().title(group_title.as_str()).build();
                for &i in indices {
                    let ch = &iio_channels[i];
                    let friendly = Self::friendly_channel(&ch.name);
                    let val_label = Self::make_sensor_row(&group, &friendly, &group_title);
                    iio_data.push(IioChannel {
                        name: ch.name.clone(),
                        device: ch.device.clone(),
                        device_id: ch.device_id.clone(),
                        raw_path: ch.raw_path.clone(),
                        scale_path: ch.scale_path.clone(),
                    });
                    iio_labels.push(val_label);
                }
                inner_box.append(&group);
            }
        }

        // Spawn a background thread to read sensor values every second.
        // All sysfs I/O happens off the main thread; only label.set_text() runs on the main thread.
        // We share the latest result via Arc<Mutex<Option<Vec<String>>>> and poll it cheaply.
        let latest: Arc<Mutex<Option<Vec<String>>>> = Arc::new(Mutex::new(None));
        let latest_write = latest.clone();

        std::thread::spawn(move || {
            let n_temp = temp_paths.len();
            loop {
                let mut values = Vec::with_capacity(n_temp + iio_data.len());
                for path in &temp_paths {
                    let text = fs::read_to_string(path)
                        .ok()
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .map(|v| format!("{:.1}°C", v / 1000.0))
                        .unwrap_or_else(|| "-".to_string());
                    values.push(text);
                }
                for ch in &iio_data {
                    values.push(Self::read_iio_value(ch));
                }
                *latest_write.lock().unwrap() = Some(values);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });

        // Poll the shared slot on the main thread every 200ms — only label.set_text() runs here,
        // so this is sub-millisecond and never blocks scrolling.
        let n_temp = temp_labels.len();
        gtk::glib::timeout_add_local(
            std::time::Duration::from_millis(200),
            move || {
                if let Some(values) = latest.lock().unwrap().take() {
                    for (i, label) in temp_labels.iter().enumerate() {
                        if let Some(text) = values.get(i) {
                            label.set_text(text);
                        }
                    }
                    for (i, label) in iio_labels.iter().enumerate() {
                        if let Some(text) = values.get(n_temp + i) {
                            label.set_text(text);
                        }
                    }
                }
                gtk::glib::ControlFlow::Continue
            },
        );

        Self { widget }
    }

    fn discover_temp_sensors() -> Vec<TempSensor> {
        let mut sensors = Vec::new();
        let base = std::path::Path::new("/sys/class/hwmon");
        let Ok(entries) = fs::read_dir(base) else {
            return sensors;
        };
        for entry in entries.flatten() {
            let hwpath = entry.path();
            let chip = fs::read_to_string(hwpath.join("name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| {
                    hwpath
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            let Ok(dir_entries) = fs::read_dir(&hwpath) else {
                continue;
            };
            for file in dir_entries.flatten() {
                let name = file.file_name().to_string_lossy().to_string();
                if name.starts_with("temp") && name.ends_with("_input") {
                    let prefix = &name[..name.len() - 6]; // strip "_input"
                    let label = fs::read_to_string(hwpath.join(format!("{}_label", prefix)))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|_| prefix.to_string());
                    sensors.push(TempSensor {
                        label,
                        chip: chip.clone(),
                        path: hwpath.join(&name),
                    });
                }
            }
        }
        // Sort: CPU first, GPU second, then alphabetical
        sensors.sort_by(|a, b| {
            let rank = |s: &TempSensor| {
                let l = s.label.to_lowercase();
                if l.contains("cpu") {
                    0
                } else if l.contains("gpu") {
                    1
                } else {
                    2
                }
            };
            rank(a).cmp(&rank(b)).then(a.label.cmp(&b.label))
        });
        sensors
    }

    fn discover_iio_channels() -> Vec<IioChannel> {
        let mut channels = Vec::new();
        let base = std::path::Path::new("/sys/bus/iio/devices");
        let Ok(entries) = fs::read_dir(base) else {
            return channels;
        };
        let mut all_entries: Vec<_> = entries.flatten().collect();
        all_entries.sort_by_key(|e| e.path());

        for entry in all_entries {
            let devpath = entry.path();
            if !devpath.is_dir() {
                continue;
            }
            let device_id = devpath
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            // Skip trigger nodes
            if device_id.starts_with("trigger") {
                continue;
            }
            let device = fs::read_to_string(devpath.join("name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| device_id.clone());
            let Ok(dir_entries) = fs::read_dir(&devpath) else {
                continue;
            };
            let mut file_entries: Vec<_> = dir_entries.flatten().collect();
            file_entries.sort_by_key(|e| e.file_name());
            for file in file_entries {
                let name = file.file_name().to_string_lossy().to_string();
                if name.starts_with("in_") && name.ends_with("_raw") {
                    let base_name = name[3..name.len() - 4].to_string();
                    let scale_path = devpath.join(format!("in_{}_scale", base_name));
                    channels.push(IioChannel {
                        name: base_name.clone(),
                        device: device.clone(),
                        device_id: device_id.clone(),
                        raw_path: devpath.join(&name),
                        scale_path: if scale_path.exists() { Some(scale_path) } else { None },
                    });
                } else if name.starts_with("in_") && name.ends_with("_input") {
                    let base_name = name[3..name.len() - 6].to_string();
                    channels.push(IioChannel {
                        name: base_name,
                        device: device.clone(),
                        device_id: device_id.clone(),
                        raw_path: devpath.join(&name),
                        scale_path: None,
                    });
                }
            }
        }
        channels
    }

    fn read_iio_value(ch: &IioChannel) -> String {
        let raw: f64 = match fs::read_to_string(&ch.raw_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            Some(v) => v,
            None => return "-".to_string(),
        };

        // Proximity and attention: show raw integer — scale is tiny (0.001) and the raw
        // value is what the daemon thresholds against, so it's more useful here.
        let n = ch.name.to_lowercase();
        if n.starts_with("proximity") || n.starts_with("prox") || n == "attention" {
            return Self::format_iio_value(&ch.name, raw);
        }

        let scaled = if let Some(scale_path) = &ch.scale_path {
            let scale: f64 = fs::read_to_string(scale_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1.0);
            raw * scale
        } else {
            // Fallback scaling by type
            let n = ch.name.to_lowercase();
            if n.starts_with("accel") || n.starts_with("grav") {
                raw / 100_000.0
            } else if n.starts_with("anglvel") && raw.abs() > 1000.0 {
                raw / 1000.0
            } else {
                raw
            }
        };

        Self::format_iio_value(&ch.name, scaled)
    }

    fn format_iio_value(name: &str, val: f64) -> String {
        let n = name.to_lowercase();
        if n == "proximity0" {
            // presence flag: 0 = nobody, 1 = detected
            format!("{}", val as i64)
        } else if n == "proximity1" {
            // raw distance; scale 0.001 → metres
            format!("{} ({:.2} m)", val as i64, val * 0.001)
        } else if n == "attention" {
            format!("{}", val as i64)
        } else if n.starts_with("accel") || n.starts_with("grav") {
            format!("{:.3} m/s²", val)
        } else if n.starts_with("anglvel") {
            format!("{:.3} rad/s", val)
        } else if n.starts_with("angl") {
            format!("{:.1}°", val)
        } else if n.starts_with("magn") {
            format!("{:.3} µT", val)
        } else if n.contains("light") || n.contains("illuminance") {
            format!("{:.1} lux", val)
        } else if n.contains("hinge") {
            format!("{:.1}°", val)
        } else if n.starts_with("rot") {
            format!("{:.4}", val)
        } else {
            format!("{:.3}", val)
        }
    }

    fn make_sensor_row(group: &PreferencesGroup, title: &str, subtitle: &str) -> Label {
        let row = ActionRow::builder().title(title).subtitle(subtitle).build();
        let val_label = Label::new(Some("-"));
        val_label.add_css_class("title-3");
        row.add_suffix(&val_label);
        group.add(&row);
        val_label
    }

    fn friendly_channel(name: &str) -> String {
        const NAMES: &[(&str, &str)] = &[
            ("proximity0",     "Proximity (present)"),
            ("proximity1",     "Proximity (distance)"),
            ("attention",      "Attention"),
            ("accel_x",        "Accelerometer X"),
            ("accel_y",        "Accelerometer Y"),
            ("accel_z",        "Accelerometer Z"),
            ("anglvel_x",      "Gyro X"),
            ("anglvel_y",      "Gyro Y"),
            ("anglvel_z",      "Gyro Z"),
            ("gravity_x",      "Gravity X"),
            ("gravity_y",      "Gravity Y"),
            ("gravity_z",      "Gravity Z"),
            ("magn_x",         "Magnetic X"),
            ("magn_y",         "Magnetic Y"),
            ("magn_z",         "Magnetic Z"),
            ("angl0",          "Hinge Angle"),
            ("angl1",          "Screen Angle"),
            ("angl2",          "Keyboard Angle"),
            ("rot_quaternion", "Rotation (Quaternion)"),
            ("prox",           "Proximity"),
            ("light",          "Ambient Light"),
            ("illuminance",    "Ambient Light"),
            ("hinge_angle",    "Hinge Angle"),
        ];
        let n = name.to_lowercase();
        NAMES.iter().find(|&&(k, _)| k == n).map(|&(_, v)| v.to_string()).unwrap_or_else(|| {
            n.split('_').map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
            }).collect::<Vec<_>>().join(" ")
        })
    }
}
