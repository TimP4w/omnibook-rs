use adw::{prelude::*, ActionRow, Clamp, PreferencesGroup};
use gtk::{Box, Label, ScrolledWindow};

pub fn make_layout() -> (Box, Box) {
    let widget = Box::new(gtk::Orientation::Vertical, 0);
    let scrolled = ScrolledWindow::builder().vexpand(true).build();
    widget.append(&scrolled);
    let content = Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).build();
    scrolled.set_child(Some(&content));
    let clamp = Clamp::builder().maximum_size(800).tightening_threshold(600).build();
    content.append(&clamp);
    let inner = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24).margin_top(24).margin_bottom(24).margin_start(12).margin_end(12)
        .build();
    clamp.set_child(Some(&inner));
    (widget, inner)
}

pub fn make_status_row(group: &PreferencesGroup, title: &str) -> Label {
    let label = Label::new(Some("—"));
    label.add_css_class("title-3");
    let row = ActionRow::builder().title(title).build();
    row.add_suffix(&label);
    group.add(&row);
    label
}
