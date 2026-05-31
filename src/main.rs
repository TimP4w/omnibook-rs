mod views;

use adw::{
    prelude::*, ActionRow, Application, ApplicationWindow, HeaderBar, NavigationPage,
    NavigationSplitView, ToolbarView,
};
use gtk::{Box, Image, ListBox, Stack};

use crate::views::battery_view::BatteryView;
use crate::views::edid_view::EdidView;
use crate::views::home_view::HomeView;
use crate::views::mouse_view::MouseView;
use crate::views::presence_view::PresenceView;
use crate::views::sensors_view::SensorsView;

const APP_NAME: &str = "dev.timp4w.omnibookrs";
struct SidebarItem {
    icon: &'static str,
    title: &'static str,
    subtitle: &'static str,
    view: &'static str,
}

const SIDEBAR_ITEMS: &[SidebarItem; 6] = &[
    SidebarItem {
        icon: "go-home-symbolic",
        title: "Home",
        subtitle: "System overview",
        view: "home",
    },
    SidebarItem {
        icon: "input-mouse-symbolic",
        title: "Mouse",
        subtitle: "Touchpad settings",
        view: "mouse",
    },
    SidebarItem {
        icon: "battery-symbolic",
        title: "Battery",
        subtitle: "Power management",
        view: "battery",
    },
    SidebarItem {
        icon: "temperature-symbolic",
        title: "Sensors",
        subtitle: "Hardware monitoring",
        view: "sensors",
    },
    SidebarItem {
        icon: "camera-web-symbolic",
        title: "Presence",
        subtitle: "Awareness & actions",
        view: "presence",
    },
    SidebarItem {
        icon: "video-display-symbolic",
        title: "EDID",
        subtitle: "Patch HDR and save panel EDID",
        view: "edid",
    },
];

fn main() {
    let app = Application::builder().application_id(APP_NAME).build();

    app.connect_activate(|app| {
        let stack = Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(200)
            .hexpand(true)
            .vexpand(true)
            .build();

        let sidebar = build_sidebar(&stack);
        let content = build_content(&stack);

        let split = NavigationSplitView::builder()
            .min_sidebar_width(220.0)
            .sidebar(&sidebar)
            .content(&content)
            .build();

        let window = ApplicationWindow::builder()
            .application(app)
            .title("OmniBook RS")
            .default_width(900)
            .default_height(650)
            .content(&split)
            .build();

        attach_views(&stack, &window);

        window.present();
    });

    fn attach_views(stack: &Stack, window: &ApplicationWindow) {
        let home_view = HomeView::new(window);
        let mouse_view = MouseView::new(window);
        let battery_view = BatteryView::new(window);
        let sensors_view = SensorsView::new(window);
        let presence_view = PresenceView::new(window);
        let edid_view = EdidView::new(window);
        stack.add_named(&home_view.widget, Some("home"));
        stack.add_named(&mouse_view.widget, Some("mouse"));
        stack.add_named(&battery_view.widget, Some("battery"));
        stack.add_named(&sensors_view.widget, Some("sensors"));
        stack.add_named(&presence_view.widget, Some("presence"));
        stack.add_named(&edid_view.widget, Some("edid"));
    }

    fn build_content(stack: &Stack) -> NavigationPage {
        let content_box = Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .hexpand(true)
            .vexpand(true)
            .build();
        content_box.append(stack);

        let main_header = HeaderBar::builder()
            .show_title(false)
            .show_start_title_buttons(false)
            .show_end_title_buttons(true)
            .build();

        let main_toolbar = ToolbarView::builder().content(&content_box).build();
        main_toolbar.add_top_bar(&main_header);

        let content_page = NavigationPage::builder()
            .title("home")
            .child(&main_toolbar)
            .build();

        return content_page;
    }

    fn build_sidebar(stack: &Stack) -> NavigationPage {
        let sidebar_list = ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .vexpand(true)
            .build();

        for item in SIDEBAR_ITEMS {
            let row = ActionRow::builder()
                .title(item.title)
                .subtitle(item.subtitle)
                .activatable(true)
                .selectable(true)
                .build();
            let icon = Image::from_icon_name(item.icon);
            row.add_prefix(&icon);
            unsafe { row.set_data("view", item.view) };
            sidebar_list.append(&row);
        }

        let stack_weak = stack.downgrade();
        sidebar_list.connect_row_activated(move |_, row| {
            if let Some(stack) = stack_weak.upgrade() {
                if let Some(view_name_ptr) = unsafe { row.data::<&str>("view") } {
                    let view_name: &str = unsafe { *view_name_ptr.as_ref() };
                    stack.set_visible_child_name(view_name);
                }
            }
        });
        // sidebar_list.connect_row_selected();

        let sidebar_box = Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .width_request(260)
            .height_request(-1)
            .hexpand(false)
            .vexpand(true)
            .css_classes(["omnibook-sidebar"])
            .build();
        sidebar_box.append(&sidebar_list);

        let sidebar_toolbar = ToolbarView::builder().content(&sidebar_box).build();

        let sidebar_header = HeaderBar::builder()
            .show_title(true)
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        sidebar_toolbar.add_top_bar(&sidebar_header);

        let sidebar_page = NavigationPage::builder()
            .title("HP Omnibook Ultra Flip")
            .child(&sidebar_toolbar)
            .build();

        return sidebar_page;
    }

    app.run();
}
