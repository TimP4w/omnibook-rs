use adw::{prelude::*, ActionRow, ApplicationWindow, ComboRow, EntryRow, PreferencesGroup};
use gtk::{Box, SpinButton};
use omnibook_rs::config::{daemon_socket_path, daemon_state_path, presence_config_path};
use omnibook_rs::presence_config::PresenceConfig;
use omnibook_rs::ipc;
use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use super::ui;

pub struct PresenceView {
    pub widget: Box,
}

const PROX_THRESHOLD_LABELS:         &[&str] = &["Very Near (<0.25 m)", "Near (<0.8 m)", "Far (>0.8 m)", "Away (no detection)"];
const PROX_THRESHOLD_VALS:           &[&str] = &["Very Near", "Near", "Far", "Away"];
const DELAY_LABELS:                  &[&str] = &["Immediately", "10 seconds", "30 seconds", "1 minute", "5 minutes"];
const DELAY_SECS:                    &[u32]  = &[0, 10, 30, 60, 300];
const PROX_AWAY_ACTION_LABELS:       &[&str] = &["None", "Lock Screen", "Custom Command"];
const PROX_AWAY_ACTION_VALS:         &[&str] = &["none", "lock", "custom"];
const PROX_RETURN_ACTION_LABELS:     &[&str] = &["None", "Wake Screen", "Custom Command"];
const PROX_RETURN_ACTION_VALS:       &[&str] = &["none", "wake", "custom"];
const ATTN_NOTLOOKING_ACTION_LABELS: &[&str] = &["None", "Dim Screen", "Lock Screen", "Custom Command"];
const ATTN_NOTLOOKING_ACTION_VALS:   &[&str] = &["none", "dim", "lock", "custom"];
const ATTN_LOOKING_ACTION_LABELS:    &[&str] = &["None", "Custom Command"];
const ATTN_LOOKING_ACTION_VALS:      &[&str] = &["none", "custom"];

fn val_idx(vals: &[&str], val: &str) -> u32 {
    vals.iter().position(|&v| v == val).unwrap_or(0) as u32
}

fn delay_idx(secs: u32) -> u32 {
    DELAY_SECS.iter().position(|&s| s == secs).unwrap_or(1) as u32
}

fn make_combo(title: &str, labels: &[&str], selected: u32) -> ComboRow {
    ComboRow::builder().title(title).model(&gtk::StringList::new(labels)).selected(selected).build()
}

fn make_entry(title: &str, text: &str) -> EntryRow {
    let e = EntryRow::builder().title(title).build();
    e.set_text(text);
    e
}

fn make_spin(title: &str, val: u8) -> (ActionRow, SpinButton) {
    let spin = SpinButton::with_range(0.0, 100.0, 5.0);
    spin.set_value(val as f64);
    spin.set_valign(gtk::Align::Center);
    spin.set_width_request(90);
    let row = ActionRow::builder().title(title).build();
    row.add_suffix(&spin);
    (row, spin)
}

#[derive(Clone)]
struct Backend {
    cfg: Rc<RefCell<PresenceConfig>>,
    writer: Option<Arc<Mutex<UnixStream>>>,
    fallback_path: PathBuf,
}

impl Backend {
    fn save(&self) {
        let cfg = self.cfg.borrow();
        match &self.writer {
            Some(w) => { let _ = ipc::send_set_config(w, &cfg); }
            None    => { cfg.save_atomic(&self.fallback_path); }
        }
    }

    fn wire_combo<T: Copy + 'static, F: Fn(&mut PresenceConfig, T) + 'static>(
        &self, combo: &ComboRow, vals: &'static [T], f: F,
    ) {
        let b = self.clone();
        combo.connect_selected_notify(move |c| { f(&mut b.cfg.borrow_mut(), vals[c.selected() as usize]); b.save(); });
    }

    fn wire_entry<F: Fn(&mut PresenceConfig, &str) + 'static>(&self, entry: &EntryRow, f: F) {
        let b = self.clone();
        entry.connect_changed(move |e| { f(&mut b.cfg.borrow_mut(), &e.text()); b.save(); });
    }

    fn wire_spin<F: Fn(&mut PresenceConfig, u8) + 'static>(&self, spin: &SpinButton, f: F) {
        let b = self.clone();
        spin.connect_value_changed(move |s| { f(&mut b.cfg.borrow_mut(), s.value() as u8); b.save(); });
    }
}

fn wire_prox_away(b: &Backend, thresh: &ComboRow, delay: &ComboRow, action: &ComboRow, custom: &EntryRow) {
    b.wire_combo(thresh, PROX_THRESHOLD_VALS, |c, v: &str| c.prox_away_threshold = v.into());
    b.wire_combo(delay, DELAY_SECS, |c, v| c.prox_away_delay = v);
    let cw = custom.downgrade();
    b.wire_combo(action, PROX_AWAY_ACTION_VALS, move |c, v: &str| {
        if let Some(w) = cw.upgrade() { w.set_visible(v == "custom"); }
        c.prox_away_action = v.into();
    });
    b.wire_entry(custom, |c, v| c.prox_away_custom = v.into());
}

fn wire_prox_return(b: &Backend, action: &ComboRow, custom: &EntryRow) {
    let cw = custom.downgrade();
    b.wire_combo(action, PROX_RETURN_ACTION_VALS, move |c, v: &str| {
        if let Some(w) = cw.upgrade() { w.set_visible(v == "custom"); }
        c.prox_return_action = v.into();
    });
    b.wire_entry(custom, |c, v| c.prox_return_custom = v.into());
}

fn wire_attn_notlooking(b: &Backend, delay: &ComboRow, action: &ComboRow, dim_row: &ActionRow, dim_spin: &SpinButton, custom: &EntryRow) {
    b.wire_combo(delay, DELAY_SECS, |c, v| c.attn_notlooking_delay = v);
    let (dr, cw) = (dim_row.downgrade(), custom.downgrade());
    b.wire_combo(action, ATTN_NOTLOOKING_ACTION_VALS, move |c, v: &str| {
        if let Some(r) = dr.upgrade() { r.set_visible(v == "dim"); }
        if let Some(w) = cw.upgrade() { w.set_visible(v == "custom"); }
        c.attn_notlooking_action = v.into();
    });
    b.wire_spin(dim_spin, |c, v| c.attn_notlooking_dim = v);
    b.wire_entry(custom, |c, v| c.attn_notlooking_custom = v.into());
}

fn wire_attn_looking(b: &Backend, action: &ComboRow, custom: &EntryRow) {
    let cw = custom.downgrade();
    b.wire_combo(action, ATTN_LOOKING_ACTION_VALS, move |c, v: &str| {
        if let Some(w) = cw.upgrade() { w.set_visible(v == "custom"); }
        c.attn_looking_action = v.into();
    });
    b.wire_entry(custom, |c, v| c.attn_looking_custom = v.into());
}

impl PresenceView {
    pub fn new(_window: &ApplicationWindow) -> Self {
        let (widget, inner) = ui::make_layout();
        let config_path = presence_config_path();
        let cfg = PresenceConfig::load(&config_path);

        // Live status
        let status_group = PreferencesGroup::builder().title("Live Status").build();
        let prox_status_label = ui::make_status_row(&status_group, "Presence");
        let attn_status_label = ui::make_status_row(&status_group, "Attention");
        inner.append(&status_group);

        // Proximity — When Away
        let prox_away_group = PreferencesGroup::builder()
            .title("Proximity — When Away")
            .description("Action to take when you move beyond the threshold distance.")
            .build();
        let prox_away_thresh  = make_combo("Threshold", PROX_THRESHOLD_LABELS, val_idx(PROX_THRESHOLD_VALS, &cfg.prox_away_threshold));
        let prox_away_delay   = make_combo("Delay", DELAY_LABELS, delay_idx(cfg.prox_away_delay));
        let prox_away_action  = make_combo("Action", PROX_AWAY_ACTION_LABELS, val_idx(PROX_AWAY_ACTION_VALS, &cfg.prox_away_action));
        let prox_away_custom  = make_entry("Custom Command", &cfg.prox_away_custom);
        prox_away_custom.set_visible(cfg.prox_away_action == "custom");
        prox_away_group.add(&prox_away_thresh);
        prox_away_group.add(&prox_away_delay);
        prox_away_group.add(&prox_away_action);
        prox_away_group.add(&prox_away_custom);
        inner.append(&prox_away_group);

        // Proximity — When Returned
        let prox_return_group = PreferencesGroup::builder()
            .title("Proximity — When Returned")
            .description("Action to take when you come back.")
            .build();
        let prox_return_action = make_combo("Action", PROX_RETURN_ACTION_LABELS, val_idx(PROX_RETURN_ACTION_VALS, &cfg.prox_return_action));
        let prox_return_custom = make_entry("Custom Command", &cfg.prox_return_custom);
        prox_return_custom.set_visible(cfg.prox_return_action == "custom");
        prox_return_group.add(&prox_return_action);
        prox_return_group.add(&prox_return_custom);
        inner.append(&prox_return_group);

        // Attention — When Not Looking
        let attn_notlooking_group = PreferencesGroup::builder()
            .title("Attention — When Not Looking")
            .description("Action to take when you look away from the screen.")
            .build();
        let attn_notlooking_delay  = make_combo("Delay", DELAY_LABELS, delay_idx(cfg.attn_notlooking_delay));
        let attn_notlooking_action = make_combo("Action", ATTN_NOTLOOKING_ACTION_LABELS, val_idx(ATTN_NOTLOOKING_ACTION_VALS, &cfg.attn_notlooking_action));
        let (attn_notlooking_dim_row, attn_notlooking_dim_spin) = make_spin("Dim Level (%)", cfg.attn_notlooking_dim);
        attn_notlooking_dim_row.set_visible(cfg.attn_notlooking_action == "dim");
        let attn_notlooking_custom = make_entry("Custom Command", &cfg.attn_notlooking_custom);
        attn_notlooking_custom.set_visible(cfg.attn_notlooking_action == "custom");
        attn_notlooking_group.add(&attn_notlooking_delay);
        attn_notlooking_group.add(&attn_notlooking_action);
        attn_notlooking_group.add(&attn_notlooking_dim_row);
        attn_notlooking_group.add(&attn_notlooking_custom);
        inner.append(&attn_notlooking_group);

        // Attention — When Looking Again
        let attn_looking_group = PreferencesGroup::builder()
            .title("Attention — When Looking Again")
            .description("Brightness is always restored automatically if screen was dimmed. Optionally run an extra command.")
            .build();
        let attn_looking_action = make_combo("Action", ATTN_LOOKING_ACTION_LABELS, val_idx(ATTN_LOOKING_ACTION_VALS, &cfg.attn_looking_action));
        let attn_looking_custom = make_entry("Custom Command", &cfg.attn_looking_custom);
        attn_looking_custom.set_visible(cfg.attn_looking_action == "custom");
        attn_looking_group.add(&attn_looking_action);
        attn_looking_group.add(&attn_looking_custom);
        inner.append(&attn_looking_group);

        // Backend: try IPC first, fall back to direct file writes
        let (writer, live_state) = match ipc::connect(&daemon_socket_path()) {
            Ok((w, r)) => {
                let state = Arc::new(Mutex::new((String::from("—"), String::from("—"))));
                ipc::spawn_state_reader(r, state.clone());
                (Some(w), Some(state))
            }
            Err(_) => (None, None),
        };
        let backend = Backend { cfg: Rc::new(RefCell::new(cfg)), writer, fallback_path: config_path };

        wire_prox_away(&backend, &prox_away_thresh, &prox_away_delay, &prox_away_action, &prox_away_custom);
        wire_prox_return(&backend, &prox_return_action, &prox_return_custom);
        wire_attn_notlooking(&backend, &attn_notlooking_delay, &attn_notlooking_action, &attn_notlooking_dim_row, &attn_notlooking_dim_spin, &attn_notlooking_custom);
        wire_attn_looking(&backend, &attn_looking_action, &attn_looking_custom);

        // Live status timer
        if let Some(state) = live_state {
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                if let Ok(s) = state.lock() {
                    prox_status_label.set_text(&s.0);
                    attn_status_label.set_text(&s.1);
                }
                gtk::glib::ControlFlow::Continue
            });
        } else {
            let state_path = daemon_state_path();
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                if let Ok(content) = std::fs::read_to_string(&state_path) {
                    for line in content.lines() {
                        if let Some(v) = line.strip_prefix("presence=") { prox_status_label.set_text(v); }
                        else if let Some(v) = line.strip_prefix("attention=") { attn_status_label.set_text(v); }
                    }
                }
                gtk::glib::ControlFlow::Continue
            });
        }

        Self { widget }
    }
}
