use adw::{prelude::*, ActionRow, ApplicationWindow, PreferencesGroup};
use gtk::{Box, Button, Label};
use super::ui;
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

fn find_edp_edid_path() -> Option<std::path::PathBuf> {
    let drm = Path::new("/sys/class/drm");
    let entries = fs::read_dir(drm).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains("-eDP-") {
            let edid = entry.path().join("edid");
            if edid.exists() {
                return Some(edid);
            }
        }
    }
    None
}

pub struct EdidView {
    pub widget: Box,
}

impl EdidView {
    pub fn new(window: &ApplicationWindow) -> Self {
        let (widget, inner_box) = ui::make_layout();

        // Header
        let header = Label::new(Some("EDID Patcher"));
        header.add_css_class("title-2");
        header.set_halign(gtk::Align::Start);
        inner_box.append(&header);

        let desc = Label::new(Some(
            "Reads the panel EDID, patches DisplayID HDR metadata into a CTA-861 block, \
             and lets you save it. GNOME/KDE currently only parse HDR from CTA-861, so \
             DisplayID 2.0 HDR blocks are ignored even though the kernel exposes \
             HDR_OUTPUT_METADATA. This conversion is needed for the desktop to recognize HDR.",
        ));
        desc.set_wrap(true);
        desc.set_xalign(0.0);
        inner_box.append(&desc);

        let status_label = Label::new(Some(""));
        status_label.set_use_markup(true);
        status_label.set_xalign(0.0);
        inner_box.append(&status_label);

        // Actions group
        let action_group = PreferencesGroup::builder().title("Actions").build();
        inner_box.append(&action_group);

        let detected_path = find_edp_edid_path();
        let source_subtitle = detected_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "eDP connector not found".to_string());
        let source_row = ActionRow::builder()
            .title("Source")
            .subtitle(&source_subtitle)
            .build();
        action_group.add(&source_row);

        let btn_box = Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let reload_btn = Button::with_label("Reload");
        let patch_btn = Button::with_label("Patch EDID");
        let save_btn = Button::with_label("Save patched…");
        btn_box.append(&reload_btn);
        btn_box.append(&patch_btn);
        btn_box.append(&save_btn);
        action_group.add(&btn_box);

        // Info group
        let info_group = PreferencesGroup::builder().title("EDID Info").build();
        inner_box.append(&info_group);

        let make_info_row = |title: &str| {
            let row = ActionRow::builder().title(title).subtitle("-").build();
            info_group.add(&row);
            row
        };
        let manufacturer_row = make_info_row("Manufacturer");
        let product_row = make_info_row("Product Code");
        let size_row = make_info_row("Size");
        let extensions_row = make_info_row("Extension Blocks");
        let displayid_row = make_info_row("Has DisplayID");
        let hdr_row = make_info_row("Has HDR Metadata");
        let cta_hdr_row = make_info_row("CTA HDR Present");

        // Shared state
        let edid_bytes: Rc<RefCell<Option<Vec<u8>>>> = Rc::new(RefCell::new(None));
        let patched_bytes: Rc<RefCell<Option<Vec<u8>>>> = Rc::new(RefCell::new(None));

        // Helper to update info rows
        let update_info_rows = {
            let manufacturer_row = manufacturer_row.clone();
            let product_row = product_row.clone();
            let size_row = size_row.clone();
            let extensions_row = extensions_row.clone();
            let displayid_row = displayid_row.clone();
            let hdr_row = hdr_row.clone();
            let cta_hdr_row = cta_hdr_row.clone();
            move |info: Option<EdidInfo>| match info {
                None => {
                    for row in [
                        &manufacturer_row,
                        &product_row,
                        &size_row,
                        &extensions_row,
                        &displayid_row,
                        &hdr_row,
                        &cta_hdr_row,
                    ] {
                        row.set_subtitle("-");
                    }
                }
                Some(i) => {
                    manufacturer_row.set_subtitle(&i.manufacturer);
                    product_row.set_subtitle(&i.product_code);
                    size_row.set_subtitle(&i.size.to_string());
                    extensions_row.set_subtitle(&i.extension_count.to_string());
                    displayid_row.set_subtitle(if i.has_displayid { "Yes" } else { "No" });
                    hdr_row.set_subtitle(if i.has_hdr { "Yes" } else { "No" });
                    cta_hdr_row.set_subtitle(if i.has_cta_hdr { "Yes" } else { "No" });
                }
            }
        };

        // Load EDID
        let load_edid = {
            let edid_bytes = edid_bytes.clone();
            let patched_bytes = patched_bytes.clone();
            let status_label = status_label.clone();
            let patch_btn = patch_btn.clone();
            let save_btn = save_btn.clone();
            let update_info_rows = update_info_rows.clone();
            move || {
                patch_btn.set_sensitive(false);
                save_btn.set_sensitive(false);
                let path = match find_edp_edid_path() {
                    Some(p) => p,
                    None => {
                        *edid_bytes.borrow_mut() = None;
                        *patched_bytes.borrow_mut() = None;
                        update_info_rows(None);
                        Self::set_status(&status_label, "eDP connector not found in /sys/class/drm", None);
                        return;
                    }
                };
                match fs::read(&path) {
                    Ok(data) => {
                        let info = EdidInfo::parse(&data);
                        let already_patched = info.as_ref().map_or(false, |i| i.has_cta_hdr);
                        update_info_rows(info);
                        *edid_bytes.borrow_mut() = Some(data.clone());
                        *patched_bytes.borrow_mut() = None;
                        if already_patched {
                            Self::set_status(&status_label, "Already patched (CTA HDR present)", Some("green"));
                        } else {
                            Self::set_status(
                                &status_label,
                                &format!("Loaded EDID ({} bytes)", data.len()),
                                None,
                            );
                        }
                        patch_btn.set_sensitive(true);
                    }
                    Err(e) => {
                        *edid_bytes.borrow_mut() = None;
                        *patched_bytes.borrow_mut() = None;
                        update_info_rows(None);
                        Self::set_status(&status_label, &format!("Failed to read EDID: {}", e), None);
                    }
                }
            }
        };

        // Reload button
        let load_edid_reload = load_edid.clone();
        reload_btn.connect_clicked(move |_| load_edid_reload());

        // Patch button
        let patch_btn_cb = patch_btn.clone();
        let save_btn_patch = save_btn.clone();
        let status_label_patch = status_label.clone();
        let edid_bytes_patch = edid_bytes.clone();
        let patched_bytes_patch = patched_bytes.clone();
        patch_btn.connect_clicked(move |_| {
            let eb = edid_bytes_patch.borrow();
            let Some(ref data) = *eb else { return };
            if has_cta_hdr(data) {
                drop(eb);
                *patched_bytes_patch.borrow_mut() = None;
                save_btn_patch.set_sensitive(false);
                Self::set_status(
                    &status_label_patch,
                    "Already patched (CTA HDR present)",
                    Some("green"),
                );
                return;
            }
            match create_patched_edid(data) {
                Ok(patched) => {
                    let len = patched.len();
                    drop(eb);
                    *patched_bytes_patch.borrow_mut() = Some(patched);
                    save_btn_patch.set_sensitive(true);
                    Self::set_status(
                        &status_label_patch,
                        &format!("Patched EDID ready ({} bytes)", len),
                        Some("green"),
                    );
                }
                Err(e) => {
                    drop(eb);
                    *patched_bytes_patch.borrow_mut() = None;
                    save_btn_patch.set_sensitive(false);
                    Self::set_status(&status_label_patch, &format!("Failed to patch: {}", e), None);
                }
            }
        });

        // Save button
        let window_save = window.clone();
        let patched_bytes_save = patched_bytes.clone();
        let status_label_save = status_label.clone();
        save_btn.set_sensitive(false);
        save_btn.connect_clicked(move |_| {
            let pb = patched_bytes_save.borrow();
            let Some(ref data) = *pb else { return };
            let data = data.clone();
            drop(pb);

            let dialog = gtk::FileChooserNative::new(
                Some("Save patched EDID"),
                Some(&window_save),
                gtk::FileChooserAction::Save,
                Some("Save"),
                Some("Cancel"),
            );
            dialog.set_current_name("edid-patched.bin");

            let status_label_dialog = status_label_save.clone();
            dialog.connect_response(move |d, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(file) = d.file() {
                        if let Some(path) = file.path() {
                            match fs::write(&path, &data) {
                                Ok(_) => Self::set_status(
                                    &status_label_dialog,
                                    "Saved patched EDID",
                                    Some("green"),
                                ),
                                Err(e) => Self::set_status(
                                    &status_label_dialog,
                                    &format!("Failed to save: {}", e),
                                    None,
                                ),
                            }
                        }
                    }
                }
            });
            dialog.show();
        });

        // Initial load
        load_edid();

        Self { widget }
    }

    fn set_status(label: &Label, text: &str, color: Option<&str>) {
        let escaped = gtk::glib::markup_escape_text(text);
        match color {
            Some(c) => label.set_markup(&format!("<span foreground='{}'>{}</span>", c, escaped)),
            None => label.set_markup(&escaped),
        }
    }
}

#[derive(Clone)]
struct EdidInfo {
    manufacturer: String,
    product_code: String,
    size: usize,
    extension_count: u8,
    has_displayid: bool,
    has_hdr: bool,
    has_cta_hdr: bool,
}

impl EdidInfo {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 128 {
            return None;
        }

        let manufacturer = {
            let a = ((data[8] >> 2) & 0x1F) + 64;
            let b = (((data[8] & 0x03) << 3) | ((data[9] >> 5) & 0x07)) + 64;
            let c = (data[9] & 0x1F) + 64;
            format!(
                "{}{}{}",
                char::from(a),
                char::from(b),
                char::from(c)
            )
        };

        let product_code = (data[10] as u16) | ((data[11] as u16) << 8);
        let extension_count = data[126];

        let mut has_displayid = false;
        let mut has_hdr = false;
        if data.len() >= 256 && extension_count >= 1 && data[128] == 0x70 {
            has_displayid = true;
            let end = (256).min(data.len()).saturating_sub(1);
            for i in 128..end {
                if data[i] == 0x06 && data[i + 1] == 0x05 {
                    has_hdr = true;
                    break;
                }
            }
        }

        Some(EdidInfo {
            manufacturer,
            product_code: format!("0x{:04x}", product_code),
            size: data.len(),
            extension_count,
            has_displayid,
            has_hdr,
            has_cta_hdr: has_cta_hdr(data),
        })
    }
}

fn has_cta_hdr(data: &[u8]) -> bool {
    if data.len() < 128 {
        return false;
    }
    let ext_count = data[126] as usize;
    for ext_index in 0..ext_count {
        let start = 128 + ext_index * 128;
        let end = start + 128;
        if end > data.len() {
            break;
        }
        let block = &data[start..end];
        if block[0] != 0x02 {
            continue; // not CTA-861
        }
        let dtd_offset = if block[2] >= 4 && block[2] <= 127 {
            block[2] as usize
        } else {
            127
        };
        let mut idx = 4usize;
        while idx < dtd_offset {
            let tag_len = block[idx];
            let tag = tag_len >> 5;
            let length = (tag_len & 0x1F) as usize;
            if length == 0 {
                idx += 1;
                continue;
            }
            if tag == 7 && idx + length < block.len() {
                let ext_tag = block[idx + 1];
                if ext_tag == 0x06 {
                    return true;
                }
            }
            idx += 1 + length;
        }
    }
    false
}

fn extract_blocks(edid: &[u8]) -> (i32, i32, i32) {
    // Returns (hdr_start, color_start, amd_start) or -1 if not found
    if edid.len() < 256 {
        return (-1, -1, -1);
    }

    let mut hdr_start = -1i32;
    let mut color_start = -1i32;
    let mut amd_start = -1i32;

    for i in 128..edid.len().min(256).saturating_sub(7) {
        if edid[i] == 0xE6 && edid[i + 1] == 0x06 && edid[i + 2] == 0x05 {
            hdr_start = i as i32;
            break;
        }
    }
    for i in 128..edid.len().min(256).saturating_sub(4) {
        if edid[i] == 0xE3 && edid[i + 1] == 0x05 {
            color_start = i as i32;
            break;
        }
    }
    for i in 128..edid.len().min(256).saturating_sub(20) {
        if edid[i] == 0x00 && edid[i + 1] == 0x00 && edid[i + 2] == 0x1A {
            amd_start = i as i32 - 4;
            break;
        }
    }

    (hdr_start, color_start, amd_start)
}

fn create_patched_edid(original: &[u8]) -> Result<Vec<u8>, String> {
    if original.len() < 256 {
        return Err("EDID missing DisplayID extension block".to_string());
    }

    let (hdr_start, color_start, amd_start) = extract_blocks(original);

    // Patch base block: increment extension count, recompute checksum
    let mut base = original[..128].to_vec();
    base[126] = base[126].saturating_add(1);
    let checksum: u32 = base[..127].iter().map(|&b| b as u32).sum();
    base[127] = ((256 - (checksum % 256)) % 256) as u8;

    // Build new CTA-861 extension block
    let mut cta = vec![0u8; 128];
    cta[0] = 0x02;
    cta[1] = 0x03;
    cta[2] = 0x23; // placeholder dtd offset
    cta[3] = 0x00;
    let mut offset = 4usize;

    // AMD block (20 bytes)
    if amd_start >= 0 {
        let amd = amd_start as usize;
        if amd + 20 <= original.len() {
            cta[offset..offset + 20].copy_from_slice(&original[amd..amd + 20]);
            offset += 20;
        }
    }

    // Color block (6 bytes)
    if color_start >= 0 {
        let cs = color_start as usize;
        if cs + 6 <= original.len() {
            cta[offset] = 0xE5;
            cta[offset + 1] = 0x05;
            cta[offset + 2..offset + 6].copy_from_slice(&original[cs + 2..cs + 6]);
            offset += 6;
        }
    } else {
        cta[offset..offset + 6].copy_from_slice(&[0xE5, 0x05, 0x00, 0x00, 0x80, 0x00]);
        offset += 6;
    }

    // HDR block (7 bytes) — required
    if hdr_start < 0 {
        return Err("HDR Static Metadata block not found in DisplayID".to_string());
    }
    let hs = hdr_start as usize;
    if hs + 7 > original.len() {
        return Err("HDR block truncated in DisplayID".to_string());
    }
    cta[offset] = 0xE6;
    cta[offset + 1] = 0x06;
    cta[offset + 2..offset + 7].copy_from_slice(&original[hs + 2..hs + 7]);
    offset += 7;

    // Pad to dtd_offset (0x23 = 35)
    while offset < 0x23 {
        cta[offset] = 0x00;
        offset += 1;
    }
    cta[2] = offset.max(4) as u8;

    // Checksum for CTA block
    let ck: u32 = cta[..127].iter().map(|&b| b as u32).sum();
    cta[127] = ((256 - (ck % 256)) % 256) as u8;

    // Result: base + displayid block + new CTA block
    let mut result = base;
    result.extend_from_slice(&original[128..256]);
    result.extend_from_slice(&cta);

    Ok(result)
}
