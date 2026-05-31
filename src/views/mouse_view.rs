use adw::{prelude::*, ActionRow, ApplicationWindow, PreferencesGroup};
use gtk::{Adjustment, Box, Image, Scale};
use super::ui;
use std::fs;
use omnibook_rs::config::haptic_config_path;
use omnibook_rs::haptic::device::HapticDevice;

const ALLOWED_VALUES: [i32; 5] = [0, 25, 50, 75, 100];

pub struct MouseView {
    pub widget: Box,
}

impl MouseView {
    pub fn new(_window: &ApplicationWindow) -> Self {
        let (widget, inner_box) = ui::make_layout();
        let haptic = HapticDevice::new();
        let initial_intensity = Self::load_last_intensity();

        let haptic_group = PreferencesGroup::builder()
            .title("Haptic Touchpad")
            .description("Synaptics force feedback touchpad settings")
            .build();
        inner_box.append(&haptic_group);

        // Device status row
        let (status_subtitle, css_class, icon_name) = match haptic.get_device_path() {
            Some(p) => (
                format!("Connected: {}", p),
                "success",
                "emblem-ok-symbolic",
            ),
            None => (
                "Synaptics SYNA3580 touchpad not detected".to_string(),
                "warning",
                "dialog-warning-symbolic",
            ),
        };
        let status_icon = Image::from_icon_name(icon_name);
        status_icon.add_css_class(css_class);
        let status_row = ActionRow::builder()
            .title("Device Status")
            .subtitle(status_subtitle)
            .build();
        status_row.add_suffix(&status_icon);
        haptic_group.add(&status_row);

        // Intensity row
        let intensity_row = ActionRow::builder()
            .title("Haptic Intensity")
            .subtitle(format!("Current: {}", initial_intensity))
            .build();
        haptic_group.add(&intensity_row);

        // Scale widget
        let adj = Adjustment::new(initial_intensity as f64, 0.0, 100.0, 25.0, 25.0, 0.0);

        let scale = Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&adj)
            .digits(0)
            .draw_value(true)
            .hexpand(true)
            .value_pos(gtk::PositionType::Left)
            .build();

        for v in ALLOWED_VALUES {
            scale.add_mark(v as f64, gtk::PositionType::Bottom, None);
        }

        let intensity_row_clone = intensity_row.clone();
        let haptic_clone = haptic.clone();
        scale.connect_value_changed(move |s| {
            let v = s.value();
            let snapped = (v / 25.0).round() * 25.0;
            if (v - snapped).abs() > 0.001 {
                s.set_value(snapped);
                return;
            }
            let intensity = snapped as i32;
            intensity_row_clone.set_subtitle(format!("Current: {}", intensity).as_str());
            Self::save_last_intensity(intensity);
            let _ = haptic_clone.set_intensity(intensity as u8);
        });

        let intensity_box = Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        intensity_box.append(&scale);
        haptic_group.add(&intensity_box);

        // Apply initial intensity to device
        if haptic.get_device_path().is_some() {
            let _ = haptic.set_intensity(initial_intensity as u8);
        }

        Self { widget }
    }

    fn load_last_intensity() -> i32 {
        let path = haptic_config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(val) = content.trim().parse::<i32>() {
                return Self::snap_value(val);
            }
        }
        50
    }

    fn save_last_intensity(val: i32) {
        let _ = fs::write(haptic_config_path(), val.to_string());
    }

    fn snap_value(val: i32) -> i32 {
        ALLOWED_VALUES
            .iter()
            .min_by_key(|&&x| (x - val).abs())
            .copied()
            .unwrap_or(50)
    }
}
