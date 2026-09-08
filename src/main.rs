mod file_entry;
mod operations;

use file_entry::{format_time, human_size, load_directory, FileEntry};
use operations::*;

use std::os::unix::fs::PermissionsExt;

use gtk::prelude::*;
use gtk::{
    gdk, gio, glib, graphene, Application, ApplicationWindow, Box as GtkBox, Button, DrawingArea,
    Entry, FlowBox, FlowBoxChild, GestureClick, GestureDrag, HeaderBar, Label, ListBox, ListBoxRow,
    Orientation, Overlay, Paned, PopoverMenu, ScrolledWindow, SearchEntry, Separator,
    Stack, ToggleButton,
};
use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const APP_ID: &str = "io.sharkmanager.SharkManager";

type RefreshFn = Rc<dyn Fn()>;
type RefreshCell = Rc<RefCell<Option<RefreshFn>>>;
type AcceptSlot = Rc<RefCell<Option<Box<dyn FnOnce(String)>>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Icons,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortCol {
    Name,
    Size,
    Modified,
    Type,
}

struct AppState {
    current_path: RefCell<PathBuf>,
    history: RefCell<Vec<PathBuf>>,
    history_pos: Cell<usize>,
    show_hidden: Cell<bool>,
    view_mode: Cell<ViewMode>,
    search_text: RefCell<String>,
    clipboard: RefCell<Option<(Vec<PathBuf>, bool)>>, // (paths, is_cut)
    selected: RefCell<HashSet<PathBuf>>,
    sort_col: Cell<SortCol>,
    sort_asc: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    monitored: RefCell<Option<PathBuf>>,
    rescan_pending: Cell<bool>,
    rename_timer: RefCell<Option<glib::SourceId>>,
    thumb_gen: Cell<u64>,
    search_timer: RefCell<Option<glib::SourceId>>,
}

impl AppState {
    fn new(initial: PathBuf) -> Self {
        Self {
            current_path: RefCell::new(initial.clone()),
            history: RefCell::new(vec![initial]),
            history_pos: Cell::new(0),
            show_hidden: Cell::new(false),
            view_mode: Cell::new(ViewMode::Icons),
            search_text: RefCell::new(String::new()),
            clipboard: RefCell::new(None),
            selected: RefCell::new(HashSet::new()),
            sort_col: Cell::new(SortCol::Name),
            sort_asc: Cell::new(true),
            monitor: RefCell::new(None),
            monitored: RefCell::new(None),
            rescan_pending: Cell::new(false),
            rename_timer: RefCell::new(None),
            thumb_gen: Cell::new(0),
            search_timer: RefCell::new(None),
        }
    }
}

fn main() -> glib::ExitCode {
    let app = adw::Application::new(Some(APP_ID), Default::default());
    app.connect_startup(|_| {
        // CSS
        let provider = gtk::CssProvider::new();
        provider.load_from_string(
            r#"
            /* ===== Sidebar ===== */
            .shark-sidebar { background: transparent; }
            .shark-sidebar row { border-radius: 8px; margin: 1px 6px; padding: 0px; }
            .shark-sidebar row:hover { background: alpha(@view_fg_color, 0.06); }
            .shark-sidebar row:selected { background: alpha(@accent_bg_color, 0.20); }
            .shark-sidebar row:selected image { color: @accent_bg_color; }
            .shark-sidebar .sidebar-title {
                font-weight: 700;
                font-size: 0.75em;
                letter-spacing: 0.08em;
                color: alpha(@view_fg_color, 0.55);
                padding: 10px 10px 2px 10px;
                min-height: 14px;
            }

            /* ===== Breadcrumbs ===== */
            .shark-breadcrumb { padding: 4px 2px; }
            .shark-breadcrumb button {
                padding: 4px 9px;
                border-radius: 7px;
                border: none;
                background: transparent;
                font-weight: 500;
            }
            .shark-breadcrumb button:hover { background: alpha(@view_fg_color, 0.08); }
            .shark-breadcrumb button.active {
                background: alpha(@accent_bg_color, 0.22);
                color: @accent_bg_color;
                font-weight: 700;
            }
            .shark-breadcrumb .sep { color: alpha(@view_fg_color, 0.4); padding: 0 1px; }

            /* ===== Icon view (FlowBox) ===== */
            flowbox { background: transparent; }
            flowboxchild {
                padding: 3px;
                border-radius: 10px;
                background: transparent;
                transition: background 120ms ease;
            }
            flowboxchild:hover { background: alpha(@view_fg_color, 0.06); }
            flowboxchild.selected { background: alpha(@accent_bg_color, 0.18); }
            flowboxchild:selected { background: alpha(@accent_bg_color, 0.20); }
            .shark-tile {
                padding: 6px 4px 5px 4px;
                min-width: 86px;
                border-radius: 8px;
            }
            .shark-tile .tile-label {
                font-size: 0.80em;
                line-height: 1.15;
            }
            .shark-tile .tile-label.selected { color: @accent_bg_color; }
            .folder-icon { color: @accent_bg_color; }

            /* ===== List view ===== */
            .shark-list-header {
                padding: 3px 10px 2px 4px;
                border-bottom: 1px solid alpha(@borders, 0.6);
            }
            .shark-list-header .sort-button {
                padding: 3px 6px;
                border-radius: 6px;
                font-weight: 700;
                font-size: 0.78em;
                color: alpha(@view_fg_color, 0.7);
                border: none;
                background: transparent;
            }
            .shark-list-header .sort-button:hover {
                background: alpha(@view_fg_color, 0.08);
                color: @view_fg_color;
            }
            .shark-list-header .sort-arrow { font-size: 0.75em; }
            listbox.shark-listbox { background: transparent; border: none; }
            listboxrow.shark-row {
                border-radius: 8px;
                margin: 0px 6px;
                transition: background 100ms ease;
            }
            listboxrow.shark-row:hover { background: alpha(@view_fg_color, 0.05); }
            listboxrow.shark-row.selected { background: alpha(@accent_bg_color, 0.18); }
            listboxrow.shark-row:selected { background: alpha(@accent_bg_color, 0.20); }
            .shark-row-content { padding: 3px 4px; border-radius: 8px; }

            /* ===== Floating status pill ===== */
            .shark-floatbar {
                background: @card_bg_color;
                border: 1px solid alpha(@borders, 0.5);
                border-radius: 12px;
                padding: 6px 14px;
                font-size: 0.85em;
            }

            headerbar { padding: 2px; }
            searchbar { border: none; }
            "#
        );
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| build_ui(app.upcast_ref()));
    app.run()
}

fn build_ui(app: &Application) {
    let initial = std::env::var("SHARK_START_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
    let window = ApplicationWindow::builder()
        .application(app)
        .title("SharkManager 🦈")
        .default_width(1220)
        .default_height(780)
        .build();

    // HeaderBar
    let header = HeaderBar::new();
    header.set_show_title_buttons(true);

    let btn_back = Button::from_icon_name("go-previous-symbolic");
    btn_back.set_tooltip_text(Some("Back (Alt+Left)"));
    let btn_forward = Button::from_icon_name("go-next-symbolic");
    btn_forward.set_tooltip_text(Some("Forward (Alt+Right)"));
    let btn_sidebar = ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show Sidebar (F9)")
        .active(true)
        .build();

    let btn_view_toggle = ToggleButton::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("Toggle list/icons")
        .build();

    let btn_sort = Button::from_icon_name("view-sort-ascending-symbolic");
    btn_sort.set_tooltip_text(Some("Sort Options"));

    let btn_search_toggle = ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search (Ctrl+F)")
        .build();

    let btn_new_folder = Button::from_icon_name("folder-new-symbolic");
    btn_new_folder.set_tooltip_text(Some("New folder (Ctrl+Shift+N)"));

    let search_entry = SearchEntry::builder()
        .placeholder_text("Search…")
        .width_request(280)
        .build();

    header.pack_start(&btn_sidebar);
    header.pack_start(&btn_back);
    header.pack_start(&btn_forward);
    header.pack_end(&btn_new_folder);
    header.pack_end(&btn_sort);
    header.pack_end(&btn_view_toggle);
    header.pack_end(&btn_search_toggle);

    // Breadcrumbs live in the header center (Nautilus pathbar page).
    let breadcrumb_box = GtkBox::new(Orientation::Horizontal, 2);
    breadcrumb_box.add_css_class("shark-breadcrumb");
    breadcrumb_box.set_hexpand(true);

    let path_entry = Entry::builder()
        .placeholder_text("Enter path…")
        .hexpand(true)
        .build();

    // Title stack swaps breadcrumbs / location entry / search field.
    let title_stack = Stack::new();
    title_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    title_stack.set_hexpand(true);
    title_stack.set_halign(gtk::Align::Fill);
    title_stack.add_named(&breadcrumb_box, Some("pathbar"));
    title_stack.add_named(&path_entry, Some("location"));
    title_stack.add_named(&search_entry, Some("search"));
    title_stack.set_visible_child_name("pathbar");
    header.set_title_widget(Some(&title_stack));

    // Title widget — Nautilus-style tabs live in their own strip
    // under the header; the header center holds the search field.
    let tabview = adw::TabView::new();
    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(&tabview));
    let btn_add_tab = Button::from_icon_name("list-add-symbolic");
    btn_add_tab.add_css_class("flat");
    btn_add_tab.set_tooltip_text(Some("New tab (Ctrl+T)"));

    window.set_titlebar(Some(&header));

    // Main layout: Paned sidebar + content
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_resize_start_child(false);
    paned.set_resize_end_child(true);

    // Sidebar
    let sidebar_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .width_request(210)
        .build();
    let sidebar = build_sidebar();
    sidebar_scroll.set_child(Some(&sidebar));
    unsafe {
        window.set_data("sidebar", sidebar.clone());
    }

    let active_pane: Rc<RefCell<Option<Rc<Pane>>>> = Rc::new(RefCell::new(None));

    let shared = Shared {
        window: window.clone(),
        breadcrumb_box: breadcrumb_box.clone(),
        title_stack: title_stack.clone(),
        btn_back: btn_back.clone(),
        btn_forward: btn_forward.clone(),
        btn_view_toggle: btn_view_toggle.clone(),
        search_entry: search_entry.clone(),
    };

    paned.set_start_child(Some(&sidebar_scroll));
    paned.set_position(220);
    paned.set_vexpand(true);

    let view_col = GtkBox::new(Orientation::Vertical, 0);
    let tab_strip = GtkBox::new(Orientation::Horizontal, 0);
    tab_bar.set_hexpand(true);
    tab_strip.append(&tab_bar);
    tab_strip.append(&btn_add_tab);
    tabview.set_vexpand(true);
    view_col.append(&tab_strip);
    view_col.append(&tabview);
    paned.set_end_child(Some(&view_col));

    window.set_child(Some(&paned));

    {
        let ap = active_pane.clone();
        let shared = shared.clone();
        let tv = tabview.clone();
        btn_add_tab.connect_clicked(move |_| {
            open_new_tab(&shared, &tv, &ap);
        });
    }

    // Tab switch: activate the pane behind the selected page.
    {
        let active_pane = active_pane.clone();
        let shared = shared.clone();
        tabview.connect_selected_page_notify(move |tv| {
            let Some(page) = tv.selected_page() else { return };
            let child = page.child();
            unsafe {
                if let Some(p) = child.data::<Rc<Pane>>("pane") {
                    *active_pane.borrow_mut() = Some(p.as_ref().clone());
                    sync_controls(&shared, p.as_ref());
                    p.as_ref().refresh_fn()();
                }
            }
        });
    }

    // Tab × button: close the page, or the window when it's the last one.
    {
        let win = window.clone();
        tabview.connect_close_page(move |tv, _page| {
            if tv.n_pages() <= 1 {
                win.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    let first_pane = build_pane(shared.clone(), initial, &tabview);
    *active_pane.borrow_mut() = Some(first_pane.clone());
    first_pane.refresh_fn()();

    // per-pane views, refresh, gestures and context menus are built in
    // build_pane(); shared controls below simply drive the active pane.

    // Sidebar actions: connect rows to navigation (on the active pane)
    {
        let ap = active_pane.clone();
        sidebar.connect_row_activated(move |_, row| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            unsafe {
                if let Some(p) = row.data::<PathBuf>("path") {
                    let path = p.as_ref().clone();
                    if path.exists() {
                        navigate_to(&pane.state, path);
                        pane.refresh_fn()();
                    }
                }
            }
        });
    }

    // Sidebar right-click: rename/remove bookmarks, add current folder.
    {
        let ap = active_pane.clone();
        let win = window.clone();
        let list = sidebar.clone();
        let right = GestureClick::new();
        right.set_button(3);
        right.connect_pressed(move |gest, _, x, y| {
            let hit = list
                .row_at_y(y as i32)
                .and_then(|row| unsafe {
                    if row.data::<()>("bookmark").is_some() {
                        row.data::<PathBuf>("path").map(|p| {
                            let path = p.as_ref().clone();
                            let items = load_bookmarks();
                            let name = items
                                .iter()
                                .find(|b| b.path == path)
                                .map(|b| b.name.clone())
                                .unwrap_or_else(|| {
                                    path.file_name()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default()
                                });
                            Bookmark { name, path }
                        })
                    } else {
                        None
                    }
                });
            let current = ap
                .borrow()
                .as_ref()
                .map(|p| p.state.current_path.borrow().clone())
                .unwrap_or_else(|| PathBuf::from("/"));
            show_sidebar_menu(&win, &list, hit, rect_at(&list, &win, x, y), current);
            gest.set_state(gtk::EventSequenceState::Claimed);
        });
        sidebar.add_controller(right);
    }

    // Header button handlers (operate on the active pane)
    {
        let ap = active_pane.clone();
        btn_back.connect_clicked(move |_| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            let state = pane.state.clone();
            let mut pos = state.history_pos.get();
            if pos > 0 {
                pos -= 1;
                state.history_pos.set(pos);
                let hist = state.history.borrow();
                if let Some(p) = hist.get(pos) {
                    *state.current_path.borrow_mut() = p.clone();
                }
                pane.refresh_fn()();
            }
        });
    }
    {
        let ap = active_pane.clone();
        btn_forward.connect_clicked(move |_| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            let state = pane.state.clone();
            let pos = state.history_pos.get();
            let hist_len = state.history.borrow().len();
            if pos + 1 < hist_len {
                let new_pos = pos + 1;
                state.history_pos.set(new_pos);
                let hist = state.history.borrow();
                if let Some(p) = hist.get(new_pos) {
                    *state.current_path.borrow_mut() = p.clone();
                }
                pane.refresh_fn()();
            }
        });
    }
    {
        let scroll = sidebar_scroll.clone();
        btn_sidebar.connect_toggled(move |btn| {
            scroll.set_visible(btn.is_active());
        });
    }
    btn_view_toggle.connect_toggled({
        let ap = active_pane.clone();
        move |btn| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            let mode = if btn.is_active() {
                ViewMode::List
            } else {
                ViewMode::Icons
            };
            pane.state.view_mode.set(mode);
            pane.refresh_fn()();
        }
    });
    btn_new_folder.connect_clicked({
        let ap = active_pane.clone();
        let window = window.clone();
        move |_| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            prompt_new_folder(&window, &pane.state, pane.refresh_fn());
        }
    });
    btn_sort.connect_clicked({
        let ap = active_pane.clone();
        let window = window.clone();
        let btn = btn_sort.clone();
        move |_| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            show_sort_menu(&window, &btn, &pane.state, pane.refresh_fn());
        }
    });
    btn_search_toggle.connect_toggled({
        let ap = active_pane.clone();
        let stack = title_stack.clone();
        let entry = search_entry.clone();
        move |btn| {
            if btn.is_active() {
                stack.set_visible_child_name("search");
                entry.grab_focus();
            } else {
                let Some(pane) = ap.borrow().as_ref().cloned() else { return };
                entry.set_text("");
                *pane.state.search_text.borrow_mut() = String::new();
                stack.set_visible_child_name("pathbar");
                pane.refresh_fn()();
            }
        }
    });
    search_entry.connect_search_changed({
        let ap = active_pane.clone();
        move |e| {
            let Some(pane) = ap.borrow().as_ref().cloned() else { return };
            *pane.state.search_text.borrow_mut() = e.text().to_string();
            if let Some(id) = pane.state.search_timer.borrow_mut().take() {
                id.remove();
            }
            let pane2 = pane.clone();
            *pane.state.search_timer.borrow_mut() =
                Some(glib::timeout_add_local_once(
                    Duration::from_millis(200),
                    move || {
                        pane2.refresh_fn()();
                    },
                ));
        }
    });
    {
        let toggle = btn_search_toggle.clone();
        let key_esc = gtk::EventControllerKey::new();
        key_esc.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                toggle.set_active(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        search_entry.add_controller(key_esc);
    }

    // Path entry handling (Ctrl+L swaps the title stack page)
    {
        let title_stack_ = title_stack.clone();
        let path_entry_ = path_entry.clone();
        let window_ = window.clone();
        let ap = active_pane.clone();
        let key_ctrl = gtk::EventControllerKey::new();
        let title_stack_c = title_stack_.clone();
        let path_entry_c = path_entry_.clone();
        let search_toggle_c = btn_search_toggle.clone();
        let sidebar_c = sidebar.clone();
        let sidebar_toggle_c = btn_sidebar.clone();
        let ap_k = ap.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, mods| {
            let Some(pane) = ap_k.borrow().as_ref().cloned() else {
                return glib::Propagation::Proceed;
            };
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::l {
                title_stack_c.set_visible_child_name("location");
                path_entry_c.set_text(&pane.state.current_path.borrow().display().to_string());
                path_entry_c.grab_focus();
                path_entry_c.select_region(0, -1);
                return glib::Propagation::Stop;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::f {
                search_toggle_c.set_active(true);
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::F5 {
                pane.refresh_fn()();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::F9 {
                let show = !sidebar_c.is_visible();
                sidebar_c.set_visible(show);
                sidebar_toggle_c.set_active(show);
                return glib::Propagation::Stop;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::h {
                let show = !pane.state.show_hidden.get();
                pane.state.show_hidden.set(show);
                pane.refresh_fn()();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window_.add_controller(key_ctrl);

        path_entry.connect_activate({
            let title_stack = title_stack_.clone();
            let ap = ap.clone();
            move |e| {
                let txt = e.text().to_string();
                let p = PathBuf::from(txt.trim());
                let Some(pane) = ap.borrow().as_ref().cloned() else {
                    title_stack.set_visible_child_name("pathbar");
                    return;
                };
                if p.exists() {
                    let target = if p.is_dir() { p } else { p.parent().unwrap_or(Path::new("/")).to_path_buf() };
                    navigate_to(&pane.state, target);
                } else {
                    // try canonicalize?
                }
                title_stack.set_visible_child_name("pathbar");
                pane.refresh_fn()();
            }
        });
        {
            let title_stack_c = title_stack.clone();
            let key_ctl = gtk::EventControllerKey::new();
            key_ctl.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    title_stack_c.set_visible_child_name("pathbar");
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            path_entry.add_controller(key_ctl);
        }
    }

    // Keyboard shortcuts for file ops (act on the active pane)
    {
        let ap = active_pane.clone();
        let shared_k = shared.clone();
        let tabview_k = tabview.clone();
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, key, _, mods| {
            let Some(pane) = ap.borrow().as_ref().cloned() else {
                return glib::Propagation::Proceed;
            };
            let state_c = pane.state.clone();
            let flow_c = pane.flow.clone();
            let list_box_c = pane.list_box.clone();
            let stack_c = pane.stack.clone();
            let status_sel_c = pane.status_sel.clone();
            let window_c = shared_k.window.clone();
            let do_refresh_c = pane.refresh_fn();
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
            match key {
                gdk::Key::Delete => {
                    // Delete: trash or shift+delete permanent
                    let paths = selected_paths(&state_c, &flow_c, &list_box_c, &stack_c);
                    if !paths.is_empty() {
                        if shift {
                            confirm_and_delete_permanent(&window_c, &paths, do_refresh_c.clone());
                        } else {
                            confirm_and_trash(&window_c, &paths, do_refresh_c.clone());
                        }
                    }
                    return glib::Propagation::Stop;
                },
                gdk::Key::F2 => {
                    let paths = selected_paths(&state_c, &flow_c, &list_box_c, &stack_c);
                    if paths.len() == 1 {
                        prompt_rename(&window_c, &paths[0], do_refresh_c.clone());
                    }
                    return glib::Propagation::Stop;
                },
                gdk::Key::F5 => { do_refresh_c(); return glib::Propagation::Stop; },
                _ => {}
            }
            if ctrl {
                match key {
                    gdk::Key::c => {
                        let paths = selected_paths(&state_c, &flow_c, &list_box_c, &stack_c);
                        if !paths.is_empty() {
                            set_system_clipboard_files(&paths);
                            *state_c.clipboard.borrow_mut() = Some((paths, false));
                        }
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::x => {
                        let paths = selected_paths(&state_c, &flow_c, &list_box_c, &stack_c);
                        if !paths.is_empty() {
                            set_system_clipboard_files(&paths);
                            *state_c.clipboard.borrow_mut() = Some((paths, true));
                        }
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::v => {
                        let dest = state_c.current_path.borrow().clone();
                        do_paste(&window_c, &state_c, do_refresh_c.clone(), dest);
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::a => {
                        // Select all visible items (both views share the
                        // same entries, so collecting from the flow is enough).
                        let mut set = HashSet::new();
                        let mut c = flow_c.first_child();
                        while let Some(w) = c {
                            unsafe {
                                if let Some(p) = w.data::<PathBuf>("path") {
                                    set.insert(p.as_ref().clone());
                                }
                            }
                            c = w.next_sibling();
                        }
                        *state_c.selected.borrow_mut() = set.clone();
                        apply_selection(&flow_c, &list_box_c, &status_sel_c, &set);
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::n => {
                        if shift {
                            prompt_new_folder(&window_c, &state_c, do_refresh_c.clone());
                        } else {
                            prompt_new_file(&window_c, &state_c, do_refresh_c.clone());
                        }
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::t => {
                        open_new_tab(&shared_k, &tabview_k, &ap);
                        return glib::Propagation::Stop;
                    },
                    gdk::Key::w => {
                        close_selected_tab(&shared_k.window, &tabview_k);
                        return glib::Propagation::Stop;
                    },
                    _ => {}
                }
            }
            // Backspace -> up
            if key == gdk::Key::BackSpace {
                let cur = state_c.current_path.borrow().clone();
                if let Some(parent) = cur.parent() {
                    navigate_to(&state_c, parent.to_path_buf());
                    do_refresh_c();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key);
    }

    // Initial refresh already ran when the first tab was created.
    window.present();

    if std::env::var("SHARK_DEBUG_LAYOUT").is_ok() {
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), {
            let window = window.clone();
            let flow = first_pane.flow.clone();
            move || {
            eprintln!("[layout] window {}x{}", window.width(), window.height());
            let mut n = 0i32;
            let mut ws = 0i32;
            let mut hs = 0i32;
            let mut child = flow.first_child();
            while let Some(c) = child {
                ws += c.width().max(0);
                hs += c.height().max(0);
                n += 1;
                child = c.next_sibling();
            }
            if n > 0 {
                eprintln!("[layout] tiles n={} avg_w={} avg_h={}", n, ws / n, hs / n);
            }
            }
        });
    }
}

/// Window-wide widgets shared by every tab/pane.
#[derive(Clone)]
struct Shared {
    window: ApplicationWindow,
    breadcrumb_box: GtkBox,
    title_stack: Stack,
    btn_back: Button,
    btn_forward: Button,
    btn_view_toggle: ToggleButton,
    search_entry: SearchEntry,
}

/// One browser tab: its own state and its own view widgets.
#[allow(dead_code)]
struct Pane {
    state: Rc<AppState>,
    flow: FlowBox,
    list_box: ListBox,
    stack: Stack,
    marquee: DrawingArea,
    marquee_rect: Rc<Cell<Option<gdk::Rectangle>>>,
    status_sel: Label,
    status_count: Label,
    adw_page: adw::TabPage,
    refresh: Rc<RefCell<Option<RefreshFn>>>,
}

impl Pane {
    fn refresh_fn(&self) -> RefreshFn {
        self.refresh
            .borrow()
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Rc::new(|| {}))
    }
}

/// Make the shared header controls (search, hidden toggle, view toggle) match
/// the given pane after a tab switch.
fn sync_controls(shared: &Shared, pane: &Pane) {
    shared
        .search_entry
        .set_text(&pane.state.search_text.borrow());
    shared
        .btn_view_toggle
        .set_active(pane.state.view_mode.get() == ViewMode::List);
}

/// Open a new tab after the selected one (Nautilus behaviour).
fn open_new_tab(
    shared: &Shared,
    tabview: &adw::TabView,
    active_pane: &Rc<RefCell<Option<Rc<Pane>>>>,
) {
    let start = active_pane
        .borrow()
        .as_ref()
        .map(|p| p.state.current_path.borrow().clone())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"));
    build_pane(shared.clone(), start, tabview);
}

/// Close the selected tab; closing the last one closes the window.
fn close_selected_tab(window: &ApplicationWindow, tabview: &adw::TabView) {
    let Some(page) = tabview.selected_page() else {
        return;
    };
    if tabview.n_pages() <= 1 {
        window.close();
    } else {
        tabview.close_page(&page);
    }
}

/// Build one tab: per-tab AppState, icon/list views, rubber-band marquee,
/// statusbar and the refresh closure, then insert it into the tab view.
fn build_pane(
    shared: Shared,
    initial: PathBuf,
    tabview: &adw::TabView,
) -> Rc<Pane> {
    let state = Rc::new(AppState::new(initial));

    // --- icon view ---
    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let flow_scroll = ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    let flow = FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .homogeneous(true)
        .valign(gtk::Align::Start)
        .max_children_per_line(12)
        .min_children_per_line(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    flow_scroll.set_child(Some(&flow));

    // --- list view ---
    let list_scroll = ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    let list_container = GtkBox::new(Orientation::Vertical, 0);
    let list_header = GtkBox::new(Orientation::Horizontal, 8);
    list_header.set_margin_top(2);
    list_header.set_margin_bottom(2);
    list_header.set_margin_start(6);
    list_header.set_margin_end(6);
    list_header.add_css_class("shark-list-header");
    let header_spacer = Label::new(None);
    header_spacer.set_width_request(18);
    list_header.append(&header_spacer);

    let mut sort_arrows: Vec<(SortCol, Label)> = Vec::new();
    let mut sort_buttons: Vec<(SortCol, Button)> = Vec::new();
    for (col, title, width) in [
        (SortCol::Name, "Name", None),
        (SortCol::Size, "Size", Some(110)),
        (SortCol::Modified, "Modified", Some(160)),
        (SortCol::Type, "Type", Some(210)),
    ] {
        let btn = Button::with_label(title);
        btn.set_has_frame(false);
        btn.add_css_class("sort-button");
        let arrow = Label::new(None);
        arrow.add_css_class("sort-arrow");
        arrow.add_css_class("dim-label");
        let h = GtkBox::new(Orientation::Horizontal, 2);
        h.append(&btn);
        h.append(&arrow);
        match width {
            Some(w) => h.set_width_request(w),
            None => {
                h.set_hexpand(true);
                btn.set_hexpand(true);
                btn.set_halign(gtk::Align::Start);
            }
        }
        list_header.append(&h);
        sort_arrows.push((col, arrow));
        sort_buttons.push((col, btn));
    }

    let list_box = ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::None);
    list_box.add_css_class("shark-listbox");
    list_container.append(&list_header);
    list_container.append(&Separator::new(Orientation::Horizontal));
    list_container.append(&list_box);
    list_scroll.set_child(Some(&list_container));

    stack.add_named(&flow_scroll, Some("icons"));
    stack.add_named(&list_scroll, Some("list"));
    stack.set_visible_child_name("icons");

    // --- floating status pill (Nautilus-style) ---
    let status_sel = Label::new(Some(""));
    status_sel.set_xalign(1.0);
    let status_count = Label::new(Some("0 items"));
    status_count.add_css_class("dim-label");
    let pill = GtkBox::new(Orientation::Horizontal, 12);
    pill.add_css_class("shark-floatbar");
    pill.set_halign(gtk::Align::Start);
    pill.set_valign(gtk::Align::End);
    pill.set_margin_start(12);
    pill.set_margin_bottom(12);
    pill.set_can_target(false);
    pill.append(&status_count);
    pill.append(&status_sel);

    // --- overlay + marquee ---
    let overlay = Overlay::new();
    overlay.set_child(Some(&stack));
    let marquee = DrawingArea::new();
    marquee.set_can_target(false);
    marquee.set_hexpand(true);
    marquee.set_vexpand(true);
    let marquee_rect: Rc<Cell<Option<gdk::Rectangle>>> = Rc::new(Cell::new(None));
    let marquee_rect_d = marquee_rect.clone();
    marquee.set_draw_func(move |_, cr, _, _| {
        if let Some(r) = marquee_rect_d.get() {
            cr.set_source_rgba(0.30, 0.59, 0.94, 0.18);
            cr.rectangle(r.x() as f64, r.y() as f64, r.width() as f64, r.height() as f64);
            let _ = cr.fill();
            cr.set_source_rgba(0.30, 0.59, 0.94, 0.75);
            cr.set_line_width(1.0);
            cr.rectangle(r.x() as f64, r.y() as f64, r.width() as f64, r.height() as f64);
            let _ = cr.stroke();
        }
    });
    overlay.add_overlay(&marquee);
    overlay.set_clip_overlay(&marquee, true);
    overlay.add_overlay(&pill);

    wire_box_drag(
        flow.upcast_ref::<gtk::Widget>(),
        &state,
        &flow,
        &list_box,
        &status_sel,
        &marquee,
        marquee_rect.clone(),
    );
    wire_box_drag(
        list_box.upcast_ref::<gtk::Widget>(),
        &state,
        &flow,
        &list_box,
        &status_sel,
        &marquee,
        marquee_rect.clone(),
    );

    let right_box = GtkBox::new(Orientation::Vertical, 0);
    right_box.append(&overlay);

    // --- tab page: title + icon handled by AdwTabView ---
    let pos = match tabview.selected_page() {
        Some(cur) => tabview.page_position(&cur) + 1,
        None => tabview.n_pages(),
    };
    let adw_page = tabview.insert(&right_box, pos);
    adw_page.set_icon(Some(&gio::ThemedIcon::new("folder")));
    adw_page.set_title("…");
    tabview.set_selected_page(&adw_page);

    let pane_ref = Rc::new(Pane {
        state: state.clone(),
        flow: flow.clone(),
        list_box: list_box.clone(),
        stack: stack.clone(),
        marquee,
        marquee_rect,
        status_sel: status_sel.clone(),
        status_count: status_count.clone(),
        adw_page: adw_page.clone(),
        refresh: Rc::new(RefCell::new(None)),
    });
    unsafe {
        right_box.set_data("pane", pane_ref.clone());
    }

    let pane_refresh: RefreshFn = {
        let cell = pane_ref.refresh.clone();
        Rc::new(move || {
            if let Some(f) = cell.borrow().as_ref() {
                f();
            }
        })
    };

    // --- sort header buttons ---
    for (col, btn) in sort_buttons {
        let st = state.clone();
        let rf = pane_refresh.clone();
        btn.connect_clicked(move |_| set_sort(&st, col, rf.clone()));
    }

    // --- background menu on empty space ---
    {
        let window = shared.window.clone();
        let g = GestureClick::new();
        g.set_button(3);
        let flow_c = flow.clone();
        let rf = pane_refresh.clone();
        let st = state.clone();
        g.connect_pressed(move |_, _, x, y| {
            show_background_menu(&window, &st, rect_at(&flow_c, &window, x, y), rf.clone());
        });
        flow.add_controller(g);
    }
    {
        let window = shared.window.clone();
        let g = GestureClick::new();
        g.set_button(3);
        let list_c = list_box.clone();
        let rf = pane_refresh.clone();
        let st = state.clone();
        g.connect_pressed(move |_, _, x, y| {
            show_background_menu(&window, &st, rect_at(&list_c, &window, x, y), rf.clone());
        });
        list_box.add_controller(g);
    }

    // --- drop files onto the views (background) ---
    {
        let st = state.clone();
        let window = shared.window.clone();
        install_drop(
            &flow.clone().upcast::<gtk::Widget>(),
            &window,
            &state,
            pane_refresh.clone(),
            move || st.current_path.borrow().clone(),
        );
    }
    {
        let st = state.clone();
        let window = shared.window.clone();
        install_drop(
            &list_box.clone().upcast::<gtk::Widget>(),
            &window,
            &state,
            pane_refresh.clone(),
            move || st.current_path.borrow().clone(),
        );
    }

    // --- refresh closure ---
    {
        let state_c = state.clone();
        let window = shared.window.clone();
        let flow = flow.clone();
        let list_box = list_box.clone();
        let stack = stack.clone();
        let breadcrumb_box = shared.breadcrumb_box.clone();
        let title_stack = shared.title_stack.clone();
        let status_sel = status_sel.clone();
        let status_count = status_count.clone();
        let btn_back = shared.btn_back.clone();
        let btn_forward = shared.btn_forward.clone();
        let btn_view_toggle = shared.btn_view_toggle.clone();
        let sort_arrows = sort_arrows.clone();
        let adw_page = adw_page.clone();
        let refresh = pane_ref.refresh.clone();
        let refresh_clone = refresh.clone();
        let refresh_indir = refresh.clone();

        let do_refresh_indir: RefreshFn = Rc::new(move || {
            if let Some(f) = refresh_indir.borrow().as_ref() {
                f();
            }
        });

        let dr: RefreshFn = Rc::new(move || {
            let cur = state_c.current_path.borrow().clone();
            let show_hidden = state_c.show_hidden.get();
            let view_mode = state_c.view_mode.get();
            let search = state_c.search_text.borrow().clone();

            setup_monitor(&state_c, do_refresh_indir.clone());

            let title = cur
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| cur.display().to_string());
            adw_page.set_title(&title);
            window.set_title(Some(&format!("{} — SharkManager 🦈", cur.display())));

            // Sort indicator arrows
            for (_col, lbl) in &sort_arrows {
                lbl.set_text("");
            }
            for (col, lbl) in &sort_arrows {
                if *col == state_c.sort_col.get() {
                    lbl.set_text(if state_c.sort_asc.get() { "▴" } else { "▾" });
                }
            }

            // History button sensitivity
            let hist = state_c.history.borrow();
            let pos = state_c.history_pos.get();
            btn_back.set_sensitive(pos > 0);
            btn_forward.set_sensitive(pos + 1 < hist.len());

            // Breadcrumbs (if not editing / searching)
            if title_stack.visible_child_name().as_deref() == Some("pathbar") {
                rebuild_breadcrumbs(
                    &breadcrumb_box,
                    &cur,
                    {
                        let state2 = state_c.clone();
                        move |p: PathBuf| {
                            navigate_to(&state2, p);
                        }
                    },
                    {
                        let refresh_cell = refresh_clone.clone();
                        move || {
                            if let Some(f) = refresh_cell.borrow().as_ref() {
                                f();
                            }
                        }
                    },
                );
            }

            // Load entries
            let mut entries = match load_directory(&cur, show_hidden, &search) {
                Ok(v) => v,
                Err(e) => {
                    show_error(&window, &format!("Cannot open folder: {e}"));
                    return;
                }
            };
            sort_entries(&mut entries, state_c.sort_col.get(), state_c.sort_asc.get());

            // Clear views
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
            while let Some(row) = list_box.first_child() {
                list_box.remove(&row);
            }
            state_c.selected.borrow_mut().clear();

            if entries.is_empty() {
                status_count.set_text("0 items");
                status_sel.set_text("");
            } else {
                let dirs = entries.iter().filter(|e| e.is_dir).count();
                let files = entries.len() - dirs;
                status_count.set_text(&format!(
                    "{} items · {} folders, {} files",
                    entries.len(),
                    dirs,
                    files
                ));
                status_sel.set_text("");
            }

            match view_mode {
                ViewMode::Icons => {
                    btn_view_toggle.set_icon_name("view-list-symbolic");
                    stack.set_visible_child_name("icons");
                }
                ViewMode::List => {
                    btn_view_toggle.set_icon_name("view-grid-symbolic");
                    stack.set_visible_child_name("list");
                }
            }

            let drop_refresh: RefreshFn = {
                let cell = refresh_clone.clone();
                Rc::new(move || {
                    if let Some(f) = cell.borrow().as_ref() {
                        f();
                    }
                })
            };

            let thumb_gen = state_c.thumb_gen.get().wrapping_add(1);
            state_c.thumb_gen.set(thumb_gen);

            if view_mode == ViewMode::Icons {
                for entry in &entries {
                    let w = build_icon_tile(entry, thumb_gen);
                let child = FlowBoxChild::new();
                child.set_child(Some(&w));
                unsafe {
                    child.set_data("path", entry.path.clone());
                }
                let state_c2 = state_c.clone();
                let path = entry.path.clone();
                let is_dir = entry.is_dir;
                let refresh_cell2 = refresh_clone.clone();
                let flow_c = flow.clone();
                let list_c = list_box.clone();
                let status_c = status_sel.clone();
                let window_c = window.clone();
                let drop_c = drop_refresh.clone();
                let w_c = w.clone();
                let gesture = GestureClick::new();
                gesture.set_button(1);
                gesture.connect_pressed(move |gest, n, _, _| {
                    if n == 1 {
                        let was = state_c2.selected.borrow().contains(&path);
                        click_select(&state_c2, &flow_c, &list_c, &status_c, &path);
                        if was {
                            let lbl = unsafe {
                                w_c.data::<Label>("name_label").map(|g| g.as_ref().clone())
                            };
                            if let Some(lbl) = lbl {
                                arm_inline_rename(
                                    &state_c2,
                                    &w_c,
                                    &lbl,
                                    path.clone(),
                                    true,
                                    &window_c,
                                    drop_c.clone(),
                                );
                            }
                        }
                    } else if n == 2 {
                        disarm_inline_rename(&state_c2);
                        if is_dir {
                            navigate_to(&state_c2, path.clone());
                            if let Some(f) = refresh_cell2.borrow().as_ref() {
                                f();
                            }
                        } else {
                            let _ = open_with_default(&path);
                        }
                        gest.set_state(gtk::EventSequenceState::Claimed);
                    }
                });
                child.add_controller(gesture);
                let state_c3 = state_c.clone();
                let window_rc = window.clone();
                let path_rc = entry.path.clone();
                let is_dir_rc = entry.is_dir;
                let refresh_cell3 = refresh_clone.clone();
                let right = GestureClick::new();
                right.set_button(3);
                let child_c = child.clone();
                right.connect_pressed(move |gest, _, x, y| {
                    show_context_menu(
                        &window_rc,
                        &state_c3,
                        &path_rc,
                        is_dir_rc,
                        rect_at(&child_c, &window_rc, x, y),
                        refresh_cell3.clone(),
                    );
                    gest.set_state(gtk::EventSequenceState::Claimed);
                });
                child.add_controller(right);
                attach_drag_source(&child.clone().upcast::<gtk::Widget>(), &state_c, entry.path.clone());
                install_drop_for_item(
                    &child.clone().upcast::<gtk::Widget>(),
                    &window,
                    &state_c,
                    entry.path.clone(),
                    entry.is_dir,
                    drop_refresh.clone(),
                );
                flow.append(&child);
                }
            }

            if view_mode == ViewMode::List {
                for entry in &entries {
                    let row_widget = build_list_row(entry);
                let row = ListBoxRow::new();
                row.add_css_class("shark-row");
                row.set_child(Some(&row_widget));
                unsafe {
                    row.set_data("path", entry.path.clone());
                }
                let state_c2 = state_c.clone();
                let path = entry.path.clone();
                let is_dir = entry.is_dir;
                let refresh_cell2 = refresh_clone.clone();
                let flow_c = flow.clone();
                let list_c = list_box.clone();
                let status_c = status_sel.clone();
                let window_c = window.clone();
                let drop_c = drop_refresh.clone();
                let box_c = row_widget.clone();
                let gesture = GestureClick::new();
                gesture.set_button(1);
                gesture.connect_pressed(move |gest, n, _, _| {
                    if n == 1 {
                        let was = state_c2.selected.borrow().contains(&path);
                        click_select(&state_c2, &flow_c, &list_c, &status_c, &path);
                        if was {
                            let lbl = unsafe {
                                box_c.data::<Label>("name_label").map(|g| g.as_ref().clone())
                            };
                            if let Some(lbl) = lbl {
                                arm_inline_rename(
                                    &state_c2,
                                    &box_c,
                                    &lbl,
                                    path.clone(),
                                    false,
                                    &window_c,
                                    drop_c.clone(),
                                );
                            }
                        }
                    } else if n == 2 {
                        disarm_inline_rename(&state_c2);
                        if is_dir {
                            navigate_to(&state_c2, path.clone());
                            if let Some(f) = refresh_cell2.borrow().as_ref() {
                                f();
                            }
                        } else {
                            let _ = open_with_default(&path);
                        }
                        gest.set_state(gtk::EventSequenceState::Claimed);
                    }
                });
                row.add_controller(gesture);
                let state_c3 = state_c.clone();
                let window_rc = window.clone();
                let path_rc = entry.path.clone();
                let is_dir_rc = entry.is_dir;
                let refresh_cell3 = refresh_clone.clone();
                let right = GestureClick::new();
                right.set_button(3);
                let row_c = row.clone();
                right.connect_pressed(move |gest, _, x, y| {
                    show_context_menu(
                        &window_rc,
                        &state_c3,
                        &path_rc,
                        is_dir_rc,
                        rect_at(&row_c, &window_rc, x, y),
                        refresh_cell3.clone(),
                    );
                    gest.set_state(gtk::EventSequenceState::Claimed);
                });
                row.add_controller(right);
                attach_drag_source(&row.clone().upcast::<gtk::Widget>(), &state_c, entry.path.clone());
                install_drop_for_item(
                    &row.clone().upcast::<gtk::Widget>(),
                    &window,
                    &state_c,
                    entry.path.clone(),
                    entry.is_dir,
                    drop_refresh.clone(),
                );
                list_box.append(&row);
                }
            }
        });
        *refresh.borrow_mut() = Some(dr);
    }

    pane_ref
}

fn navigate_to(state: &AppState, path: PathBuf) {
    let p = if path.is_file() {
        path.parent().unwrap_or(Path::new("/")).to_path_buf()
    } else {
        path
    };
    let canon = p.canonicalize().unwrap_or(p);
    {
        let mut hist = state.history.borrow_mut();
        let pos = state.history_pos.get();
        // truncate forward
        hist.truncate(pos + 1);
        if hist.last() != Some(&canon) {
            hist.push(canon.clone());
            state.history_pos.set(hist.len() - 1);
        }
        // limit history
        if hist.len() > 100 {
            hist.remove(0);
            state.history_pos.set(hist.len() - 1);
        }
    }
    *state.current_path.borrow_mut() = canon;
}

/// (Re)create a gio::FileMonitor on the current directory so the view refreshes
/// automatically when files are added/removed/renamed by other apps. Only one
/// monitor is kept per tab (recreated when the path changes).
fn setup_monitor(state: &Rc<AppState>, do_refresh: RefreshFn) {
    let path = state.current_path.borrow().clone();
    if state.monitored.borrow().as_deref() == Some(path.as_path()) {
        return;
    }
    let file = gio::File::for_path(&path);
    match file.monitor_directory(gio::FileMonitorFlags::WATCH_MOUNTS | gio::FileMonitorFlags::SEND_MOVED, None::<&gio::Cancellable>) {
        Ok(m) => {
            let st = state.clone();
            let dr = do_refresh.clone();
            m.connect_changed(move |_, _f, _other, event| {
                if matches!(event, gio::FileMonitorEvent::Deleted) {
                    let cur = st.current_path.borrow().clone();
                    if let Some(parent) = cur.parent() {
                        navigate_to(&st, parent.to_path_buf());
                        dr();
                        return;
                    }
                }
                // Debounce bursts of events into at most one refresh per 250ms.
                if !st.rescan_pending.replace(true) {
                    let st2 = st.clone();
                    let dr2 = dr.clone();
                    glib::timeout_add_local(Duration::from_millis(250), move || {
                        st2.rescan_pending.set(false);
                        dr2();
                        glib::ControlFlow::Break
                    });
                }
            });
            *state.monitor.borrow_mut() = Some(m);
            *state.monitored.borrow_mut() = Some(path);
        }
        Err(_) => {
            *state.monitor.borrow_mut() = None;
            *state.monitored.borrow_mut() = None;
        }
    }
}

fn set_sort(state: &Rc<AppState>, col: SortCol, do_refresh: RefreshFn) {
    if state.sort_col.get() == col {
        state.sort_asc.set(!state.sort_asc.get());
    } else {
        state.sort_col.set(col);
        state.sort_asc.set(true);
    }
    do_refresh();
}

fn sort_entries(entries: &mut [FileEntry], col: SortCol, asc: bool) {
    let dir_rank = |e: &FileEntry| if e.is_dir { 0u8 } else { 1u8 };
    entries.sort_by(|a, b| {
        let d = dir_rank(a).cmp(&dir_rank(b));
        if d != std::cmp::Ordering::Equal {
            return d;
        }
        let c = match col {
            SortCol::Name => a
                .name_lower
                .cmp(&b.name_lower)
                .then_with(|| a.name.cmp(&b.name)),
            SortCol::Size => a.size.cmp(&b.size),
            SortCol::Modified => a.modified.cmp(&b.modified),
            SortCol::Type => a
                .mime_lower
                .cmp(&b.mime_lower)
                .then_with(|| a.name_lower.cmp(&b.name_lower)),
        };
        if asc {
            c
        } else {
            c.reverse()
        }
    });
}

fn rebuild_breadcrumbs<F, R>(box_: &GtkBox, path: &Path, on_navigate: F, on_refresh_bg: R)
where
    F: Fn(PathBuf) + Clone + 'static,
    R: Fn() + Clone + 'static,
{
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
    let mut comps: Vec<PathBuf> = Vec::new();
    let mut cur = PathBuf::new();
    for comp in path.components() {
        cur.push(comp.as_os_str());
        comps.push(cur.clone());
    }
    if comps.is_empty() {
        comps.push(PathBuf::from("/"));
    }
    for (i, p) in comps.iter().enumerate() {
        let name = if i == 0 {
            // root
            if p == &PathBuf::from("/") {
                "/".to_string()
            } else {
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            }
        } else {
            p.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        if name.is_empty() {
            continue;
        }
        let is_last = i == comps.len() - 1;
        let btn = Button::with_label(&name);
        btn.set_has_frame(false);
        if is_last {
            btn.add_css_class("active");
        }
        let p_clone = p.clone();
        let on_navigate_c = on_navigate.clone();
        let on_refresh_bg_c = on_refresh_bg.clone();
        btn.connect_clicked(move |_| {
            on_navigate_c(p_clone.clone());
            on_refresh_bg_c();
        });
        box_.append(&btn);
        if !is_last {
            let sep = Label::new(Some("›"));
            sep.add_css_class("sep");
            box_.append(&sep);
        }
    }
}

struct ThumbJob {
    id: u64,
    path: PathBuf,
    size: i32,
    video: bool,
    cache_key: String,
    gen: u64,
}

struct ThumbDone {
    id: u64,
    cache_key: String,
    gen: u64,
    pix: Option<RawPix>,
}

struct RawPix {
    bytes: glib::Bytes,
    w: i32,
    h: i32,
    stride: i32,
    alpha: bool,
}

fn raw_pixbuf(p: &gdk_pixbuf::Pixbuf) -> RawPix {
    RawPix {
        bytes: p.read_pixel_bytes(),
        w: p.width(),
        h: p.height(),
        stride: p.rowstride(),
        alpha: p.has_alpha(),
    }
}

std::thread_local! {
    static THUMB_PENDING: std::cell::RefCell<std::collections::HashMap<u64, glib::WeakRef<gtk::Image>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

static THUMB_NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static THUMB_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static THUMB_POLLER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn thumb_results() -> &'static std::sync::Mutex<Vec<ThumbDone>> {
    static Q: std::sync::OnceLock<std::sync::Mutex<Vec<ThumbDone>>> =
        std::sync::OnceLock::new();
    Q.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn thumb_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, gdk::Texture>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, gdk::Texture>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn thumb_cache_key(entry: &FileEntry, px: i32, video: bool) -> String {
    let mtime = entry
        .modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{}|{}|{}|{}|{}",
        entry.path.display(),
        entry.size,
        mtime,
        px,
        video as u8
    )
}

fn cached_thumb(entry: &FileEntry, px: i32, video: bool) -> Option<gdk::Texture> {
    let key = thumb_cache_key(entry, px, video);
    thumb_cache().lock().ok()?.get(&key).cloned()
}

fn decode_scaled(path: &Path, size: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let s = size.max(16);
    gdk_pixbuf::Pixbuf::from_file_at_size(path, s, s).ok()
}

struct ThumbPool {
    senders: [std::sync::mpsc::Sender<ThumbJob>; 4],
    next: std::sync::atomic::AtomicUsize,
}

fn thumb_pool() -> &'static ThumbPool {
    static POOL: std::sync::OnceLock<ThumbPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let senders = [(); 4].map(|_| {
            let (tx, rx) = std::sync::mpsc::channel::<ThumbJob>();
            std::thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    let raw = if job.video {
                        gen_video_thumb(&job.path)
                            .and_then(|f| decode_scaled(&f, job.size))
                            .map(|p| raw_pixbuf(&p))
                    } else {
                        decode_scaled(&job.path, job.size).map(|p| raw_pixbuf(&p))
                    };
                    if let Ok(mut q) = thumb_results().lock() {
                        q.push(ThumbDone {
                            id: job.id,
                            cache_key: job.cache_key,
                            gen: job.gen,
                            pix: raw,
                        });
                    }
                    THUMB_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
            tx
        });
        ThumbPool {
            senders,
            next: std::sync::atomic::AtomicUsize::new(0),
        }
    })
}

fn thumb_pump() {
    let done: Vec<ThumbDone> = thumb_results()
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    for d in done {
        let weak = THUMB_PENDING.with(|m| m.borrow_mut().remove(&d.id));
        let Some(weak) = weak else { continue };
        let Some(img) = weak.upgrade() else { continue };
        let cur = unsafe { img.data::<u64>("thumb_gen").map(|g| *g.as_ref()) }
            .unwrap_or(u64::MAX);
        if cur != d.gen {
            continue;
        }
        if let Some(r) = d.pix {
            let pix = gdk_pixbuf::Pixbuf::from_bytes(
                &r.bytes,
                gdk_pixbuf::Colorspace::Rgb,
                r.alpha,
                8,
                r.w,
                r.h,
                r.stride,
            );
            let tex = gdk::Texture::for_pixbuf(&pix);
            if let Ok(mut cache) = thumb_cache().lock() {
                if cache.len() >= 800 {
                    cache.clear();
                }
                cache.insert(d.cache_key, tex.clone());
            }
            img.set_paintable(Some(&tex));
        }
    }
}

fn ensure_thumb_poller() {
    if THUMB_POLLER_ON.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        thumb_pump();
        if THUMB_IN_FLIGHT.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            thumb_pump();
            THUMB_POLLER_ON.store(false, std::sync::atomic::Ordering::Relaxed);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn request_thumb(entry: &FileEntry, img: &gtk::Image, size: i32, video: bool, gen: u64) {
    let key = thumb_cache_key(entry, size, video);
    unsafe {
        img.set_data("thumb_gen", gen);
    }
    let id = THUMB_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    THUMB_PENDING.with(|m| m.borrow_mut().insert(id, img.downgrade()));
    THUMB_IN_FLIGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pool = thumb_pool();
    let n = pool.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let job = ThumbJob { id, path: entry.path.clone(), size, video, cache_key: key, gen };
    if pool.senders[n % pool.senders.len()].send(job).is_err() {
        THUMB_PENDING.with(|m| m.borrow_mut().remove(&id));
        THUMB_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
    ensure_thumb_poller();
}

fn build_icon_tile(entry: &FileEntry, gen: u64) -> GtkBox {
    const ICON_SIZE: i32 = 64;
    const THUMB_SIZE: i32 = 80;

    let vbox = GtkBox::new(Orientation::Vertical, 2);
    vbox.set_halign(gtk::Align::Center);
    vbox.set_valign(gtk::Align::Start);
    vbox.set_width_request(110);
    vbox.add_css_class("shark-tile");

    // Icon / preview
    if entry.is_dir {
        let icon = gtk::Image::from_icon_name(if entry.is_symlink {
            "folder-symbolic"
        } else {
            "folder"
        });
        icon.set_pixel_size(ICON_SIZE);
        if !entry.is_symlink {
            icon.add_css_class("folder-icon");
        }
        vbox.append(&icon);
    } else if entry.mime_approx.starts_with("video/") {
        let icon = gtk::Image::from_icon_name("video-x-generic");
        icon.set_pixel_size(ICON_SIZE);
        vbox.append(&icon);
        request_thumb(entry, &icon, THUMB_SIZE, true, gen);
    } else if entry.mime_approx.starts_with("image/") {
        if let Some(tex) = cached_thumb(entry, THUMB_SIZE, false) {
            let img = gtk::Image::from_paintable(Some(&tex));
            img.set_pixel_size(THUMB_SIZE);
            img.set_halign(gtk::Align::Center);
            vbox.append(&img);
        } else {
            let icon = gtk::Image::from_icon_name(&entry.icon_name);
            icon.set_pixel_size(ICON_SIZE);
            vbox.append(&icon);
            request_thumb(entry, &icon, THUMB_SIZE, false, gen);
        }
    } else {
        let icon = gtk::Image::from_icon_name(&entry.icon_name);
        icon.set_pixel_size(ICON_SIZE);
        vbox.append(&icon);
    }

    let label = Label::new(Some(&entry.name));
    label.add_css_class("tile-label");
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label.set_justify(gtk::Justification::Center);
    label.set_lines(2);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_max_width_chars(14);
    if entry.is_hidden {
        label.add_css_class("dim-label");
    }
    if entry.is_symlink {
        label.add_css_class("dim-label");
    }
    vbox.append(&label);
    unsafe {
        vbox.set_data("name_label", label.clone());
    }

    vbox
}

fn gen_video_thumb(path: &Path) -> Option<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("sharkmanager")
        .join("thumbs");
    let meta = std::fs::metadata(path).ok()?;
    let mut h = DefaultHasher::new();
    path.to_string_lossy().as_bytes().hash(&mut h);
    meta.len().hash(&mut h);
    if let Ok(m) = meta.modified() {
        if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
            d.as_secs().hash(&mut h);
        }
    }
    let key = format!("{:016x}", h.finish());
    let out = cache.join(format!("{}.jpg", key));
    if out.exists() {
        return Some(out);
    }
    std::fs::create_dir_all(&cache).ok()?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = cache.join(format!("{}.{}.{}.jpg", key, std::process::id(), n));
    let input = path.to_string_lossy().to_string();
    use std::process::Stdio;
    let ok = std::process::Command::new("ffmpegthumbnailer")
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&tmp)
        .arg("-s")
        .arg("320")
        .arg("-q")
        .arg("6")
        .arg("-t")
        .arg("10")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !ok.map(|s| s.success()).unwrap_or(false) || !tmp.exists() {
        let _ = std::process::Command::new("ffmpeg")
            .arg("-y")
            .arg("-ss")
            .arg("2")
            .arg("-i")
            .arg(&input)
            .arg("-frames:v")
            .arg("1")
            .arg("-vf")
            .arg("scale=320:-2")
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if tmp.exists() {
        std::fs::rename(&tmp, &out).ok()?;
        return Some(out);
    }
    None
}

fn build_list_row(entry: &FileEntry) -> GtkBox {
    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(1);
    hbox.set_margin_bottom(1);
    hbox.add_css_class("shark-row-content");

    let icon = gtk::Image::from_icon_name(&entry.icon_name);
    icon.set_pixel_size(18);
    hbox.append(&icon);

    let name_label = Label::new(Some(&entry.name));
    name_label.set_xalign(0.0);
    name_label.set_ellipsize(pango::EllipsizeMode::End);
    name_label.set_hexpand(true);
    if entry.is_hidden {
        name_label.add_css_class("dim-label");
    }
    hbox.append(&name_label);
    unsafe {
        hbox.set_data("name_label", name_label.clone());
    }

    let size_str = if entry.is_dir {
        "—".to_string()
    } else {
        human_size(entry.size)
    };
    let size_label = Label::new(Some(&size_str));
    size_label.set_xalign(1.0);
    size_label.set_width_request(110);
    size_label.add_css_class("dim-label");
    hbox.append(&size_label);

    let mod_label = Label::new(Some(&format_time(entry.modified)));
    mod_label.set_xalign(0.0);
    mod_label.set_width_request(160);
    mod_label.add_css_class("dim-label");
    hbox.append(&mod_label);

    let type_label = Label::new(Some(&entry.mime_approx));
    type_label.set_xalign(0.0);
    type_label.set_width_request(210);
    type_label.set_ellipsize(pango::EllipsizeMode::End);
    type_label.add_css_class("dim-label");
    hbox.append(&type_label);

    hbox
}

fn xdg_user_dir(name: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let expect = format!("XDG_{}_DIR", name.to_ascii_uppercase());
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let sources = [
        config_home.join("user-dirs.dirs"),
        PathBuf::from("/etc/xdg/user-dirs.dirs"),
    ];
    let mut dirs: Vec<PathBuf> = Vec::new();
    for source in sources {
        if let Ok(content) = std::fs::read_to_string(&source) {
            for line in content.lines() {
                let line = line.trim();
                let Some(val) = line.strip_prefix(&expect) else {
                    continue;
                };
                let val = val
                    .strip_prefix('=')
                    .unwrap_or(val)
                    .trim()
                    .trim_matches('"')
                    .trim();
                let expanded = if let Some(rest) = val.strip_prefix("$HOME/") {
                    home.join(rest)
                } else if let Some(rest) = val.strip_prefix("${HOME}/") {
                    home.join(rest)
                } else if let Some(rest) = val.strip_prefix("~/") {
                    home.join(rest)
                } else {
                    PathBuf::from(val)
                };
                if expanded.is_dir() && !dirs.contains(&expanded) {
                    dirs.push(expanded);
                }
            }
        }
    }
    let fallback = home.join(name.to_ascii_lowercase());
    if fallback.is_dir() && !dirs.contains(&fallback) {
        dirs.push(fallback);
    }
    dirs.into_iter().next()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bookmark {
    name: String,
    path: PathBuf,
}

fn bookmark_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".config/sharkmanager/bookmarks"));
    }
    out
}

fn load_bookmarks() -> Vec<Bookmark> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for file in bookmark_files() {
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, ' ');
            let uri = parts.next().unwrap_or("");
            let label = parts.next().unwrap_or("").trim();
            if !uri.starts_with("file://") {
                continue;
            }
            let path = PathBuf::from(percent_decode(uri.trim_start_matches("file://")));
            if !seen.insert(path.clone()) {
                continue;
            }
            let name = if label.is_empty() {
                path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            } else {
                label.to_string()
            };
            out.push(Bookmark { name, path });
        }
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/?:@&=+$,;_-.!~*'()".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

fn save_bookmarks(items: &[Bookmark]) {
    let mut text = String::new();
    for b in items {
        let uri = format!("file://{}", percent_encode(&b.path.to_string_lossy()));
        if b.name.trim().is_empty() {
            text.push_str(&uri);
        } else {
            text.push_str(&format!("{} {}", uri, b.name.trim()));
        }
        text.push('\n');
    }
    for file in bookmark_files() {
        let Some(parent) = file.parent() else { continue };
        if std::fs::create_dir_all(parent).is_err() {
            continue;
        }
        let _ = std::fs::write(&file, &text);
    }
}

fn append_bookmark_rows(list: &ListBox) {
    let items = load_bookmarks();
    if items.is_empty() {
        return;
    }
    let lbl = Label::new(Some("Bookmarks"));
    lbl.set_xalign(0.0);
    lbl.add_css_class("sidebar-title");
    let header = ListBoxRow::new();
    header.set_selectable(false);
    header.set_activatable(false);
    header.set_child(Some(&lbl));
    unsafe {
        header.set_data("bookmark_header", ());
    }
    list.append(&header);
    for b in items {
        let row = ListBoxRow::new();
        let h = GtkBox::new(Orientation::Horizontal, 8);
        h.set_margin_top(4);
        h.set_margin_bottom(4);
        h.set_margin_start(8);
        h.set_margin_end(8);
        let img = gtk::Image::from_icon_name("folder");
        img.set_pixel_size(18);
        h.append(&img);
        row.set_child(Some(&h));
        row.set_tooltip_text(Some(&format!("{}\n{}", b.name, b.path.display())));
        unsafe {
            row.set_data("path", b.path);
            row.set_data("bookmark", ());
        }
        list.append(&row);
    }
}

fn refresh_bookmark_rows(list: &ListBox) {
    let mut dead = Vec::new();
    let mut c = list.first_child();
    while let Some(w) = c {
        let tagged = unsafe { w.data::<()>("bookmark").is_some() || w.data::<()>("bookmark_header").is_some() };
        if tagged {
            dead.push(w.clone());
        }
        c = w.next_sibling();
    }
    for w in dead {
        list.remove(&w);
    }
    append_bookmark_rows(list);
}

fn build_sidebar() -> ListBox {
    let list = ListBox::new();
    list.add_css_class("shark-sidebar");
    list.add_css_class("sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);

    let add_section = |title: &str, list: &ListBox| {
        let lbl = Label::new(Some(title));
        lbl.set_xalign(0.0);
        lbl.add_css_class("sidebar-title");
        let row = ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_child(Some(&lbl));
        list.append(&row);
    };

    let add_item = |icon: &str, label: &str, path: PathBuf, list: &ListBox| {
        let row = ListBoxRow::new();
        let h = GtkBox::new(Orientation::Horizontal, 8);
        h.set_margin_top(4);
        h.set_margin_bottom(4);
        h.set_margin_start(8);
        h.set_margin_end(8);
        let img = gtk::Image::from_icon_name(icon);
        img.set_pixel_size(18);
        let lbl = Label::new(Some(label));
        lbl.set_xalign(0.0);
        lbl.set_hexpand(true);
        h.append(&img);
        h.append(&lbl);
        row.set_child(Some(&h));
        unsafe { row.set_data("path", path); }
        list.append(&row);
    };

    add_section("Places", &list);
    if let Some(home) = dirs::home_dir() {
        add_item("go-home-symbolic", "Home", home.clone(), &list);
        let places = [
            ("folder-documents-symbolic", "DOCUMENTS"),
            ("folder-download-symbolic", "DOWNLOAD"),
            ("folder-pictures-symbolic", "PICTURES"),
            ("folder-videos-symbolic", "VIDEOS"),
            ("folder-music-symbolic", "MUSIC"),
            ("user-desktop-symbolic", "DESKTOP"),
        ];
        for (icon, xdg_name) in places {
            let path = xdg_user_dir(xdg_name);
            let Some(path) = path else { continue };
            let label = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| xdg_name.to_ascii_lowercase());
add_item(icon, &label, path, &list);
            }
    }
    add_item("drive-harddisk-symbolic", "File System", PathBuf::from("/"), &list);
    add_item("user-trash-symbolic", "Trash", dirs::home_dir().unwrap_or(PathBuf::from("/")).join(".local/share/Trash/files"), &list);
    // Bookmarks (icon-only rows with tooltips; editable via right-click)
    append_bookmark_rows(&list);
    // Devices / network
    add_section("Devices", &list);
    for (name, mnt) in mounted_devices() {
        add_item("drive-removable-media-symbolic", &name, mnt, &list);
    }

    list
}

fn mounted_devices() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let data = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    for line in data.lines() {
        let mut it = line.split_whitespace();
        let src = it.next().unwrap_or("");
        let mnt = it.next().unwrap_or("");
        if !src.starts_with("/dev/") || mnt == "/" {
            continue;
        }
        if mnt.starts_with("/boot") || mnt.starts_with("/nix") || mnt.starts_with("/snap") {
            continue;
        }
        let mb = PathBuf::from(mnt);
        let name = mb
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                src.rsplit('/')
                    .next()
                    .unwrap_or(mnt)
                    .to_string()
            });
        out.push((name, mb));
    }
    out.sort();
    out.dedup_by_key(|(_, p)| p.clone());
    out
}

fn rect_at(widget: &impl gtk::prelude::WidgetExt, window: &ApplicationWindow, x: f64, y: f64) -> gdk::Rectangle {
    let (wx, wy) = widget
        .compute_point(window, &graphene::Point::new(x as f32, y as f32))
        .map(|p| (p.x() as f64, p.y() as f64))
        .unwrap_or((x, y));
    gdk::Rectangle::new(wx.round() as i32, wy.round() as i32, 1, 1)
}

fn clamp_pass(p: gtk::Popover, w: gtk::ApplicationWindow) {
    if let Some(s) = p.surface().and_then(|s| s.downcast::<gdk::Popup>().ok()) {
        let (px, py) = (s.position_x(), s.position_y());
        let (pw, ph) = (s.width(), s.height());
        let (ww, wh) = (w.width(), w.height());
        let tx = px.clamp(0, (ww - pw).max(0));
        let ty = py.clamp(0, (wh - ph).max(0));
        let (dx, dy) = (tx - px, ty - py);
        if dx != 0 || dy != 0 {
            let (ox, oy) = p.offset();
            p.set_offset(ox + dx, oy + dy);
            p.popdown();
            p.popup();
        }
    }
}

/// Keep a popover inside the visible window area. GDK places popups against the
/// monitor work area (not the window), so with a short/narrow window a context
/// menu anchored near the edge hangs outside the window. Like most menus, we
/// slide it back in once it is placed.
fn clamp_popover_to_window(pop: &gtk::Popover, window: &ApplicationWindow) {
    let p = std::rc::Rc::new(pop.clone());
    let w = std::rc::Rc::new(window.clone());
    pop.connect_map(move |_| {
        if unsafe { p.data::<()>("clamp").is_some() } {
            return;
        }
        unsafe { p.set_data("clamp", ()) };
        let pi = p.clone();
        let wi = w.clone();
        glib::idle_add_local(move || {
            clamp_pass((*pi).clone(), (*wi).clone());
            let pt = pi.clone();
            let wt = wi.clone();
            glib::timeout_add_local(Duration::from_millis(450), move || {
                clamp_pass((*pt).clone(), (*wt).clone());
                glib::ControlFlow::Break
            });
            glib::ControlFlow::Break
        });
    });
}

fn app_key(app: &gio::AppInfo) -> String {
    app.id()
        .map(|s| s.to_string())
        .or_else(|| app.commandline().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default()
}

#[derive(Clone)]
struct CachedApp {
    key: String,
    name: String,
}

/// Installed apps (id + display name), cached briefly so right-clicking doesn't
/// rescan every .desktop file on the main thread (which used to stall the UI).
type AppCache = (Instant, Vec<CachedApp>);

fn all_apps_cached() -> Vec<CachedApp> {
    const TTL: Duration = Duration::from_secs(90);
    static CACHE: OnceLock<Mutex<Option<AppCache>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().unwrap();
    if let Some((at, apps)) = &*guard {
        if at.elapsed() < TTL {
            return apps.clone();
        }
    }
    let apps: Vec<CachedApp> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| {
            let k = app_key(a);
            k != "sharkmanager.desktop" && k != "io.sharkmanager.SharkManager.desktop"
        })
        .map(|a| CachedApp {
            key: app_key(&a),
            name: a.name().to_string(),
        })
        .collect();
    *guard = Some((Instant::now(), apps.clone()));
    apps
}

/// Dolphin-style: recommended apps for the type first, then every other
/// installed application (deduped). `rec_count` marks the split between the
/// two groups. Returns `(apps, rec_count)`.
fn apps_for_path(path: &Path) -> Option<(Vec<CachedApp>, usize)> {
    let file_name = path.file_name().and_then(|s| s.to_str());
    let (ctype, _) = gio::content_type_guess(file_name, &[]);
    if ctype.is_empty() {
        return None;
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut apps: Vec<CachedApp> = Vec::new();
    for app in gio::AppInfo::recommended_for_type(&ctype) {
        let key = app_key(&app);
        if seen.insert(key.clone()) {
            apps.push(CachedApp {
                key,
                name: app.name().to_string(),
            });
        }
    }
    let rec_count = apps.len();
    for app in all_apps_cached() {
        if seen.insert(app.key.clone()) {
            apps.push(app);
        }
    }
    if apps.is_empty() {
        return None;
    }
    Some((apps, rec_count))
}

fn show_context_menu(
    window: &ApplicationWindow,
    state: &Rc<AppState>,
    path: &Path,
    is_dir: bool,
    pos: gdk::Rectangle,
    do_refresh: RefreshCell,
) {
    let path_buf2 = path.to_path_buf();
    let action_group = gio::SimpleActionGroup::new();
    window.insert_action_group("ctx", Some(&action_group));

    // The whole selection when the right-clicked item is inside it, else just
    // that item. All file operations act on this set (Dolphin behaviour).
    let items = context_items(state, path);
    let single = items.len() == 1;

    // Use gio Menu + PopoverMenu
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append(Some("Open"), Some("ctx.open"));
    if !is_dir {
        if let Some((apps, rec_count)) = apps_for_path(path) {
            let file_name = path.file_name().and_then(|s| s.to_str());
            let (ctype, _) = gio::content_type_guess(file_name, &[]);
            let default_key = gio::AppInfo::default_for_type(&ctype, false)
                .as_ref()
                .map(app_key);

            // One parameterised action for the whole list (id of the app),
            // instead of one action per app, so right-clicking stays instant.
            let pb = path_buf2.clone();
            let ct = ctype.clone();
            let win = window.clone();
            let def_source = window.title().unwrap_or_default().to_string();
            let open_act = gio::SimpleAction::new("open_with", Some(glib::VariantTy::STRING));
            open_act.connect_activate(move |_, param| {
                let key = param.and_then(|v| v.str()).map(str::to_owned);
                if let Some(key) = key {
                    if let Some(app) = gio::DesktopAppInfo::new(&key) {
                        let file = gio::File::for_path(&pb);
                        if app.launch(&[file], None::<&gio::AppLaunchContext>).is_ok()
                            && app.set_as_default_for_type(&ct).is_ok()
                        {
                            let flash = format!("Default set to {} for {}", app.name(), ct);
                            let prev = def_source.clone();
                            let win2 = win.clone();
                            win2.set_title(Some(&flash));
                            glib::timeout_add_local(
                                Duration::from_millis(1600),
                                move || {
                                    if win2.title().as_deref() == Some(flash.as_str()) {
                                        win2.set_title(Some(&prev));
                                    }
                                    glib::ControlFlow::Break
                                },
                            );
                        }
                    }
                }
            });
            action_group.add_action(&open_act);

            let open_sub = gio::Menu::new();
            let rec_sec = gio::Menu::new();
            let other_sec = gio::Menu::new();
            let total = apps.len();
            for (idx, app) in apps.iter().enumerate() {
                let label = app.name.clone();
                let is_def = default_key.as_deref() == Some(app.key.as_str());
                let shown = if is_def { format!("\u{2713} {}", label) } else { label };
                let item =
                    gio::MenuItem::new(Some(shown.as_str()), Some("ctx.open_with"));
                item.set_action_and_target_value(
                    Some("ctx.open_with"),
                    Some(&glib::Variant::from(app.key.as_str())),
                );
                (if idx < rec_count { &rec_sec } else { &other_sec }).append_item(&item);
            }

            if rec_count > 0 {
                open_sub.append_section(Some("Recommended"), &rec_sec);
            }
            if rec_count < total {
                open_sub.append_section(Some("Other Applications"), &other_sec);
            }
            section.append_submenu(Some("Open With"), &open_sub);
        }
    }
    menu.append_section(None, &section);

    let edit = gio::Menu::new();
    edit.append(Some("Cut"), Some("ctx.cut"));
    edit.append(Some("Copy"), Some("ctx.copy"));
    let rename_item = gio::MenuItem::new(Some("Rename… (F2)"), Some("ctx.rename"));
    rename_item.set_attribute_value(
        "hidden-when",
        Some(&glib::Variant::from("action-disabled")),
    );
    edit.append_item(&rename_item);
    menu.append_section(None, &edit);

    let del = gio::Menu::new();
    del.append(Some("Move to Trash"), Some("ctx.trash"));
    del.append(Some("Delete Permanently…"), Some("ctx.delete"));
    del.append(Some("Properties"), Some("ctx.props"));
    menu.append_section(None, &del);

    // Need to create popover anchored at click? Simpler: create PopoverMenu positioned near window
    // For quick demo, use Alert dialog style? But we use PopoverMenu
    let pop = PopoverMenu::from_model(Some(&menu));
    pop.set_has_arrow(false);
    pop.set_pointing_to(Some(&pos));
    pop.set_parent(window);

    let path_buf = path.to_path_buf();
    let state_c = state.clone();
    let do_refresh_c = do_refresh.clone();
    let open_act = gio::SimpleAction::new("open", None);
    open_act.connect_activate(move |_, _| {
        if path_buf.is_dir() {
            navigate_to(&state_c, path_buf.clone());
            if let Some(f) = do_refresh_c.borrow().as_ref() {
                f();
            }
        } else {
            let _ = open_with_default(&path_buf);
        }
    });
    action_group.add_action(&open_act);

    let state_c3 = state.clone();
    let items3 = items.clone();
    let cut_act = gio::SimpleAction::new("cut", None);
    cut_act.connect_activate(move |_, _| {
        set_system_clipboard_files(&items3);
        *state_c3.clipboard.borrow_mut() = Some((items3.clone(), true));
    });
    action_group.add_action(&cut_act);

    let state_c4 = state.clone();
    let items4 = items.clone();
    let copy_act = gio::SimpleAction::new("copy", None);
    copy_act.connect_activate(move |_, _| {
        set_system_clipboard_files(&items4);
        *state_c4.clipboard.borrow_mut() = Some((items4.clone(), false));
    });
    action_group.add_action(&copy_act);

    let window_c5 = window.clone();
    let do_refresh5 = do_refresh.clone();
    let items5 = items.clone();
    let rename_act = gio::SimpleAction::new("rename", None);
    rename_act.set_enabled(single);
    rename_act.connect_activate(move |_, _| {
        if items5.len() == 1 {
            prompt_rename(&window_c5, &items5[0], {
                let f = do_refresh5.borrow().as_ref().cloned();
                f.unwrap_or_else(|| Rc::new(|| {}))
            });
        }
    });
    action_group.add_action(&rename_act);

    let window_c6 = window.clone();
    let do_refresh6 = do_refresh.clone();
    let items6 = items.clone();
    let trash_act = gio::SimpleAction::new("trash", None);
    trash_act.connect_activate(move |_, _| {
        confirm_and_trash(&window_c6, &items6, {
            let f = do_refresh6.borrow().as_ref().cloned();
            f.unwrap_or_else(|| Rc::new(|| {}))
        });
    });
    action_group.add_action(&trash_act);

    let window_c7 = window.clone();
    let do_refresh7 = do_refresh.clone();
    let items7 = items.clone();
    let delete_act = gio::SimpleAction::new("delete", None);
    delete_act.connect_activate(move |_, _| {
        confirm_and_delete_permanent(&window_c7, &items7, {
            let f = do_refresh7.borrow().as_ref().cloned();
            f.unwrap_or_else(|| Rc::new(|| {}))
        });
    });
    action_group.add_action(&delete_act);

    let window_c8 = window.clone();
    let items8 = items;
    let props_act = gio::SimpleAction::new("props", None);
    props_act.connect_activate(move |_, _| {
        if items8.len() == 1 {
            show_properties(&window_c8, &items8[0]);
        } else {
            show_properties_multi(&window_c8, &items8);
        }
    });
    action_group.add_action(&props_act);

    clamp_popover_to_window(pop.upcast_ref(), window);
    pop.popup();
}

fn add_bookmark_dir(path: &Path) {
    let mut items = load_bookmarks();
    if items.iter().any(|b| b.path == path) {
        return;
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    items.push(Bookmark { name, path: path.to_path_buf() });
    save_bookmarks(&items);
}

fn refresh_sidebar_bookmarks(window: &ApplicationWindow) {
    let list: Option<ListBox> =
        unsafe { window.data::<ListBox>("sidebar").map(|r| r.as_ref().clone()) };
    if let Some(list) = list {
        refresh_bookmark_rows(&list);
    }
}

fn show_sidebar_menu(
    window: &ApplicationWindow,
    list: &ListBox,
    hit: Option<Bookmark>,
    pos: gdk::Rectangle,
    current: PathBuf,
) {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    if hit.is_some() {
        section.append(Some("Rename…"), Some("sbar.rename"));
        section.append(Some("Remove"), Some("sbar.remove"));
    } else {
        section.append(Some("Add Bookmark"), Some("sbar.add"));
    }
    menu.append_section(None, &section);
    let pop = PopoverMenu::from_model(Some(&menu));
    pop.set_has_arrow(false);
    pop.set_pointing_to(Some(&pos));
    pop.set_parent(window);
    let g = gio::SimpleActionGroup::new();
    window.insert_action_group("sbar", Some(&g));
    if let Some(bm) = hit {
        let w = window.clone();
        let list_c = list.clone();
        let old_name = bm.name.clone();
        let old_path = bm.path.clone();
        let a = gio::SimpleAction::new("rename", None);
        a.connect_activate(move |_, _| {
            let list_c2 = list_c.clone();
            let oname = old_name.clone();
            let opath = old_path.clone();
            prompt_entry(
                &w,
                "Rename Bookmark",
                &opath.display().to_string(),
                "Bookmark name",
                &oname.clone(),
                "Rename",
                move |name| {
                    let name = name.trim().to_string();
                    if name.is_empty() || name == oname {
                        return;
                    }
                    let mut items = load_bookmarks();
                    if let Some(b) = items.iter_mut().find(|b| b.path == opath) {
                        b.name = name;
                        save_bookmarks(&items);
                        refresh_bookmark_rows(&list_c2);
                    }
                },
            );
        });
        g.add_action(&a);
        let list_c = list.clone();
        let a = gio::SimpleAction::new("remove", None);
        a.connect_activate(move |_, _| {
            let mut items = load_bookmarks();
            items.retain(|b| b.path != bm.path);
            save_bookmarks(&items);
            refresh_bookmark_rows(&list_c);
        });
        g.add_action(&a);
    } else {
        let w = window.clone();
        let a = gio::SimpleAction::new("add", None);
        a.connect_activate(move |_, _| {
            add_bookmark_dir(&current);
            refresh_sidebar_bookmarks(&w);
        });
        g.add_action(&a);
    }
    clamp_popover_to_window(pop.upcast_ref(), window);
    pop.popup();
}

fn show_sort_menu(
    window: &ApplicationWindow,
    anchor: &Button,
    state: &Rc<AppState>,
    refresh: RefreshFn,
) {
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    for (label, val) in [
        ("Name", "name"),
        ("Size", "size"),
        ("Modified", "modified"),
        ("Type", "type"),
    ] {
        s1.append(Some(label), Some(&format!("view.sort_by::'{val}'")));
    }
    menu.append_section(Some("Sort By"), &s1);
    let s2 = gio::Menu::new();
    s2.append(Some("Descending Order"), Some("view.sort_desc"));
    s2.append(Some("Show Hidden Files"), Some("view.show_hidden"));
    menu.append_section(None, &s2);

    let pop = PopoverMenu::from_model(Some(&menu));
    pop.set_has_arrow(false);
    pop.set_parent(anchor);

    let g = gio::SimpleActionGroup::new();
    window.insert_action_group("view", Some(&g));

    let cur_col = match state.sort_col.get() {
        SortCol::Size => "size",
        SortCol::Modified => "modified",
        SortCol::Type => "type",
        _ => "name",
    }
    .to_string();
    let st = state.clone();
    let rf = refresh.clone();
    let act = gio::SimpleAction::new_stateful(
        "sort_by",
        Some(glib::VariantTy::STRING),
        &cur_col.to_variant(),
    );
    act.connect_activate(move |a, p| {
        let v: String = p.and_then(|v| v.get()).unwrap_or_default();
        let col = match v.as_str() {
            "size" => SortCol::Size,
            "modified" => SortCol::Modified,
            "type" => SortCol::Type,
            _ => SortCol::Name,
        };
        a.set_state(&v.to_variant());
        st.sort_col.set(col);
        rf();
    });
    g.add_action(&act);

    let st = state.clone();
    let rf = refresh.clone();
    let desc = gio::SimpleAction::new_stateful("sort_desc", None, &(!st.sort_asc.get()).to_variant());
    desc.connect_activate(move |a, _| {
        let v: bool = a.state().and_then(|s| s.get()).unwrap_or(false);
        a.set_state(&(!v).to_variant());
        st.sort_asc.set(v);
        rf();
    });
    g.add_action(&desc);

    let st = state.clone();
    let rf = refresh.clone();
    let hid = gio::SimpleAction::new_stateful("show_hidden", None, &st.show_hidden.get().to_variant());
    hid.connect_activate(move |a, _| {
        let v: bool = a.state().and_then(|s| s.get()).unwrap_or(false);
        a.set_state(&(!v).to_variant());
        st.show_hidden.set(!v);
        rf();
    });
    g.add_action(&hid);

    clamp_popover_to_window(pop.upcast_ref(), window);
    pop.popup();
}

fn show_background_menu(
    window: &ApplicationWindow,
    state: &Rc<AppState>,
    pos: gdk::Rectangle,
    do_refresh: RefreshFn,
) {
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    s1.append(Some("New Folder…"), Some("bg.new_folder"));
    s1.append(Some("New File…"), Some("bg.new_file"));
    s1.append(Some("Paste"), Some("bg.paste"));
    menu.append_section(None, &s1);
    let s2 = gio::Menu::new();
    s2.append(Some("Open in Terminal"), Some("bg.terminal"));
    s2.append(Some("Add Bookmark"), Some("bg.bookmark"));
    s2.append(Some("Refresh"), Some("bg.refresh"));
    s2.append(Some("Properties"), Some("bg.props"));
    menu.append_section(None, &s2);

    let pop = PopoverMenu::from_model(Some(&menu));
    pop.set_has_arrow(false);
    pop.set_pointing_to(Some(&pos));
    pop.set_parent(window);

    let g = gio::SimpleActionGroup::new();
    window.insert_action_group("bg", Some(&g));

    let state_c = state.clone();
    let w = window.clone();
    let dr = do_refresh.clone();
    let a = gio::SimpleAction::new("new_folder", None);
    a.connect_activate(move |_, _| prompt_new_folder(&w, &state_c, dr.clone()));
    g.add_action(&a);

    let state_c = state.clone();
    let w = window.clone();
    let dr = do_refresh.clone();
    let a = gio::SimpleAction::new("new_file", None);
    a.connect_activate(move |_, _| prompt_new_file(&w, &state_c, dr.clone()));
    g.add_action(&a);

    let state_c = state.clone();
    let w = window.clone();
    let dr = do_refresh.clone();
    let a = gio::SimpleAction::new("paste", None);
    a.connect_activate(move |_, _| {
        let dest = state_c.current_path.borrow().clone();
        do_paste(&w, &state_c, dr.clone(), dest);
    });
    g.add_action(&a);

    let cur = state.current_path.borrow().clone();
    let a = gio::SimpleAction::new("terminal", None);
    a.connect_activate(move |_, _| {
        let _ = std::process::Command::new("xdg-terminal-exec")
            .arg(cur.clone())
            .spawn()
            .or_else(|_| std::process::Command::new("kitty").current_dir(&cur).spawn())
            .or_else(|_| std::process::Command::new("konsole").current_dir(&cur).spawn())
            .or_else(|_| std::process::Command::new("gnome-terminal").current_dir(&cur).spawn());
    });
    g.add_action(&a);

    let dr2 = do_refresh.clone();
    let a = gio::SimpleAction::new("refresh", None);
    a.connect_activate(move |_, _| dr2());
    g.add_action(&a);

    let state_c = state.clone();
    let w = window.clone();
    let a = gio::SimpleAction::new("bookmark", None);
    a.connect_activate(move |_, _| {
        add_bookmark_dir(&state_c.current_path.borrow().clone());
        refresh_sidebar_bookmarks(&w);
    });
    g.add_action(&a);

    let w2 = window.clone();
    let cur2 = state.current_path.borrow().clone();
    let a = gio::SimpleAction::new("props", None);
    a.connect_activate(move |_, _| show_properties(&w2, &cur2));
    g.add_action(&a);

    clamp_popover_to_window(pop.upcast_ref(), window);
    pop.popup();
}

fn get_selected_or_current(
    state: &Rc<AppState>,
    flow: &FlowBox,
    list_box: &ListBox,
    stack: &Stack,
) -> Option<PathBuf> {
    let sel = state.selected.borrow();
    if let Some(p) = sel.iter().next() {
        return Some(p.clone());
    }
    // fallback: check flow/list selection
    let visible = stack.visible_child_name().unwrap_or_else(|| "icons".into());
    if visible == "icons" {
        let children = flow.selected_children();
        if let Some(c) = children.first() {
            unsafe {
                if let Some(p) = c.data::<PathBuf>("path") {
                    return Some(p.as_ref().clone());
                }
            }
        }
    } else {
        if let Some(row) = list_box.selected_row() {
            unsafe {
                if let Some(p) = row.data::<PathBuf>("path") {
                    return Some(p.as_ref().clone());
                }
            }
        }
    }
    None
}

/// All currently selected (box-selected) paths, or the single "current" item
/// if nothing is box-selected. Empty when nothing is selected.
fn selected_paths(
    state: &Rc<AppState>,
    flow: &FlowBox,
    list_box: &ListBox,
    stack: &Stack,
) -> Vec<PathBuf> {
    let sel = state.selected.borrow();
    if !sel.is_empty() {
        return sel.iter().cloned().collect();
    }
    get_selected_or_current(state, flow, list_box, stack)
        .map(|p| vec![p])
        .unwrap_or_default()
}

/// Paths a right-click should operate on (Dolphin behaviour): when the
/// clicked item is part of the current selection, the whole selection is used,
/// otherwise just the clicked item.
fn context_items(state: &AppState, clicked: &Path) -> Vec<PathBuf> {
    let sel = state.selected.borrow();
    if sel.contains(clicked) {
        sel.iter().cloned().collect()
    } else {
        vec![clicked.to_path_buf()]
    }
}

/// Publish a list of files on the system clipboard as the native
/// `gdk::FileList` (serialised to text/uri-list etc.), so other apps like
/// Dolphin / GNOME Files can paste them.
fn file_list_from_paths(paths: &[PathBuf]) -> gdk::FileList {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    gdk::FileList::from_array(&files)
}

fn set_system_clipboard_files(paths: &[PathBuf]) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gdk::ContentProvider::for_value(&glib::Value::from(&file_list_from_paths(paths)));
    let _ = display.clipboard().set_content(Some(&provider));
}

/// Ask the system clipboard for a file list; calls `cb` on the main thread.
fn read_system_clipboard_files(cb: impl FnOnce(Vec<PathBuf>) + 'static) {
    let Some(display) = gdk::Display::default() else {
        cb(Vec::new());
        return;
    };
    let clipboard = display.clipboard();
    clipboard.read_value_async(
        gdk::FileList::static_type(),
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |res| {
            let paths = match res {
                Ok(value) => value
                    .get_owned::<gdk::FileList>()
                    .map(|fl| {
                        fl.files()
                            .iter()
                            .filter_map(|f| f.path())
                            .collect::<Vec<PathBuf>>()
                    })
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            cb(paths);
        },
    );
}

fn paste_paths(
    window: &ApplicationWindow,
    state: &Rc<AppState>,
    paths: &[PathBuf],
    is_cut: bool,
    dest: &Path,
    do_refresh: RefreshFn,
) {
    for src in paths {
        let name = src
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let dst = dest.join(unique_name(dest, &name));
        let res = if is_cut {
            std::fs::rename(src, &dst)
                .map_err(|e| e.to_string())
                .or_else(|_| copy_recursive(src, &dst).and_then(|_| delete_permanent(src)))
        } else {
            copy_recursive(src, &dst)
        };
        if let Err(e) = res {
            show_error(window, &format!("Paste failed: {e}"));
        }
    }
    if is_cut {
        *state.clipboard.borrow_mut() = None;
    }
    do_refresh();
}

/// Paste from the system clipboard if it holds files, otherwise fall back to
/// the app-internal clipboard. Non-file clipboard content is ignored.
fn do_paste(window: &ApplicationWindow, state: &Rc<AppState>, do_refresh: RefreshFn, dest: PathBuf) {
    let window = window.clone();
    let state = state.clone();
    read_system_clipboard_files(move |ext| {
        if !ext.is_empty() {
            let internal_cut = matches!(&*state.clipboard.borrow(), Some((ip, true)) if *ip == ext);
            paste_paths(&window, &state, &ext, internal_cut, &dest, do_refresh.clone());
            return;
        }
        if let Some((paths, is_cut)) = state.clipboard.borrow().clone() {
            paste_paths(&window, &state, &paths, is_cut, &dest, do_refresh);
        }
    });
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn uri_to_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode(
            rest.strip_suffix('/').unwrap_or(rest),
        )));
    }
    if s.contains("://") {
        return None;
    }
    Some(PathBuf::from(s))
}

/// Turn a `text/uri-list` payload into local file paths.
fn parse_uri_list(s: &str) -> Vec<PathBuf> {
    s.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(uri_to_path)
        .collect()
}

/// Make `host` accept dropped files (copy-only): copies them into `dest()`.
fn install_drop(
    host: &gtk::Widget,
    window: &ApplicationWindow,
    _state: &Rc<AppState>,
    refresh: RefreshFn,
    dest: impl Fn() -> PathBuf + 'static,
) {
    let window = window.clone();
    let handle: Rc<dyn Fn(Vec<PathBuf>)> = Rc::new(move |paths| {
        if paths.is_empty() {
            return;
        }
        let dest = dest();
        for src in paths {
            let name = src
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let dst = dest.join(unique_name(&dest, &name));
            if let Err(e) = copy_recursive(&src, &dst) {
                show_error(&window, &format!("Drop failed: {e}"));
            }
        }
        refresh();
    });

    let h1 = handle.clone();
    let t1 = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    t1.connect_drop(move |_, value, _, _| {
        let Ok(fl) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
        if paths.is_empty() {
            return false;
        }
        h1(paths);
        true
    });
    host.add_controller(t1);

    let h2 = handle;
    let t2 = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
    t2.connect_drop(move |_, value, _, _| {
        let Ok(s) = value.get::<String>() else {
            return false;
        };
        let paths = parse_uri_list(&s);
        if paths.is_empty() {
            return false;
        }
        h2(paths);
        true
    });
    host.add_controller(t2);
}

/// Make `child` draggable: dragging starts a copy of the clicked item (or the
/// whole selection when it contains the clicked item).
fn attach_drag_source(child: &gtk::Widget, state: &Rc<AppState>, path: PathBuf) {
    let state = state.clone();
    let ds = gtk::DragSource::new();
    ds.set_actions(gdk::DragAction::COPY);
    let ds_c = ds.clone();
    ds.connect_prepare(move |_, _, _| {
        let paths = context_items(&state, &path);
        if paths.is_empty() {
            return None;
        }
        let value = glib::Value::from(&file_list_from_paths(&paths));
        Some(gdk::ContentProvider::for_value(&value))
    });
    let _ = ds_c;
    child.add_controller(ds);
}

/// A tile/row accepts drops: into the folder itself when it's a directory,
/// otherwise into the parent view.
fn install_drop_for_item(
    host: &gtk::Widget,
    window: &ApplicationWindow,
    state: &Rc<AppState>,
    item_path: PathBuf,
    is_dir: bool,
    refresh: RefreshFn,
) {
    let dest_state = state.clone();
    install_drop(
        host,
        window,
        state,
        refresh,
        move || {
            if is_dir && item_path.is_dir() {
                item_path.clone()
            } else {
                dest_state.current_path.borrow().clone()
            }
        },
    );
}

fn rect_overlaps(a: &gdk::Rectangle, b: &gdk::Rectangle) -> bool {
    a.x() < b.x() + b.width()
        && b.x() < a.x() + a.width()
        && a.y() < b.y() + b.height()
        && b.y() < a.y() + a.height()
}

fn children_in_rect(host: &gtk::Widget, rect: &gdk::Rectangle) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut c = host.first_child();
    while let Some(w) = c {
        let bounds = w.compute_bounds(host).map(|r| {
            gdk::Rectangle::new(r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32)
        });
        if let Some(a) = bounds {
            if rect_overlaps(rect, &a) {
                unsafe {
                    if let Some(p) = w.data::<PathBuf>("path") {
                        out.insert(p.as_ref().clone());
                    }
                }
            }
        }
        c = w.next_sibling();
    }
    out
}

/// Rubber-band selection: the only way to select (plain clicks never change
/// the selection). Drag a box; items it covers become selected (Ctrl keeps
/// the existing selection, otherwise it is replaced).
fn wire_box_drag(
    host: &gtk::Widget,
    state: &Rc<AppState>,
    flow: &FlowBox,
    list_box: &ListBox,
    status_sel: &Label,
    marquee: &DrawingArea,
    marquee_rect: Rc<Cell<Option<gdk::Rectangle>>>,
) {
    struct DragBox {
        start: (f64, f64),
        ctrl: bool,
        base: HashSet<PathBuf>,
    }

    let state = state.clone();
    let state_u = state.clone();
    let flow_c = flow.clone();
    let list_c = list_box.clone();
    let status_c = status_sel.clone();
    let marquee_c = marquee.clone();
    let host_c = host.clone();
    let box_state: Rc<RefCell<Option<DragBox>>> = Rc::new(RefCell::new(None));

    let gd = GestureDrag::new();
    gd.set_button(1);
    let box_state_b = box_state.clone();
    gd.connect_drag_begin(move |g, sx, sy| {
        let ctrl = g.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
        let base = state.selected.borrow().clone();
        *box_state_b.borrow_mut() = Some(DragBox {
            start: (sx, sy),
            ctrl,
            base,
        });
    });

    let box_state_u = box_state.clone();
    let marquee_rect_u = marquee_rect.clone();
    gd.connect_drag_update(move |_, ox, oy| {
        let (rect, ctrl, base) = {
            let mut b = box_state_u.borrow_mut();
            let db = match b.as_mut() {
                Some(db) => db,
                None => return,
            };
            let cur = (db.start.0 + ox, db.start.1 + oy);
            // The update/end signals report offsets from the start point.
            let rect = normalized_rect(db.start, cur);
            let (dx, dy) = (rect.width() as f64, rect.height() as f64);
            // Only treat it as a selection drag once it crosses the drag
            // threshold, so accidental click-presses never flash-select.
            if dx * dx + dy * dy <= 16.0 {
                return;
            }
            disarm_inline_rename(&state_u);
            (rect, db.ctrl, db.base.clone())
        };
        if let (Some(a), Some(b)) = (
            host_c.compute_point(&marquee_c, &graphene::Point::new(rect.x() as f32, rect.y() as f32)),
            host_c.compute_point(
                &marquee_c,
                &graphene::Point::new((rect.x() + rect.width()) as f32, (rect.y() + rect.height()) as f32),
            ),
        ) {
            let tr = gdk::Rectangle::new(
                a.x().min(b.x()) as i32,
                a.y().min(b.y()) as i32,
                (a.x() - b.x()).abs() as i32,
                (a.y() - b.y()).abs() as i32,
            );
            marquee_rect_u.set(Some(tr));
            marquee_c.queue_draw();
        }
        let mut set = if ctrl { base } else { HashSet::new() };
        set.extend(children_in_rect(&host_c, &rect));
        apply_selection(&flow_c, &list_c, &status_c, &set);
    });

    let box_state_e = box_state.clone();
    let flow_e = flow.clone();
    let list_e = list_box.clone();
    let status_e = status_sel.clone();
    let marquee_e = marquee.clone();
    let host_e = host.clone();
    gd.connect_drag_end(move |_, ox, oy| {
        let db = match box_state_e.borrow_mut().take() {
            Some(db) => db,
            None => return,
        };
        let cur = (db.start.0 + ox, db.start.1 + oy);
        let rect = normalized_rect(db.start, cur);
        let (dx, dy) = (rect.width() as f64, rect.height() as f64);
        let set = if dx * dx + dy * dy > 16.0 {
            let mut set = if db.ctrl { db.base } else { HashSet::new() };
            set.extend(children_in_rect(&host_e, &rect));
            set
        } else {
            db.base // accidental click: leave selection untouched
        };
        marquee_rect.set(None);
        marquee_e.queue_draw();
        apply_selection(&flow_e, &list_e, &status_e, &set);
    });

    host.add_controller(gd);
}

fn normalized_rect(a: (f64, f64), b: (f64, f64)) -> gdk::Rectangle {
    let x0 = a.0.min(b.0) as i32;
    let y0 = a.1.min(b.1) as i32;
    let x1 = a.0.max(b.0) as i32;
    let y1 = a.1.max(b.1) as i32;
    gdk::Rectangle::new(x0, y0, x1 - x0, y1 - y0)
}

fn apply_selection(
    flow: &FlowBox,
    list_box: &ListBox,
    status_sel: &Label,
    set: &HashSet<PathBuf>,
) {
    let mut child = flow.first_child();
    while let Some(c) = child {
        let sel = unsafe { c.data::<PathBuf>("path").map(|p| set.contains(p.as_ref())) };
        let sel = sel.unwrap_or(false);
        if sel {
            c.add_css_class("selected");
        } else {
            c.remove_css_class("selected");
        }
        child = c.next_sibling();
    }
    let mut row = list_box.first_child();
    while let Some(r) = row {
        let sel = unsafe { r.data::<PathBuf>("path").map(|p| set.contains(p.as_ref())) };
        let sel = sel.unwrap_or(false);
        if sel {
            r.add_css_class("selected");
        } else {
            r.remove_css_class("selected");
        }
        row = r.next_sibling();
    }
    update_status_sel(status_sel, set);
}

fn update_status_sel(status_sel: &Label, set: &HashSet<PathBuf>) {
    if set.is_empty() {
        status_sel.set_text("");
        return;
    }
    let mut bytes: u64 = 0;
    let mut files = 0;
    for p in set {
        if p.is_file() {
            files += 1;
            if let Ok(m) = std::fs::symlink_metadata(p) {
                bytes += m.len();
            }
        }
    }
    let mut s = format!("{} selected", set.len());
    if files > 0 {
        s.push_str(&format!(" · {} {}", files, human_size(bytes)));
    }
    status_sel.set_text(&s);
}

fn prompt_entry(
    window: &ApplicationWindow,
    title: &str,
    detail: &str,
    placeholder: &str,
    initial: &str,
    ok_label: &str,
    on_accept: impl FnOnce(String) + 'static,
) {
    let dlg = gtk::Window::builder()
        .title(title)
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .build();
    let v = GtkBox::new(Orientation::Vertical, 10);
    v.set_margin_top(16);
    v.set_margin_bottom(16);
    v.set_margin_start(16);
    v.set_margin_end(16);
    let lbl = Label::new(Some(detail));
    lbl.set_xalign(0.0);
    lbl.set_wrap(true);
    let entry = Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(initial);
    entry.select_region(0, -1);
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let cancel = Button::with_label("Cancel");
    let ok = Button::with_label(ok_label);
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    v.append(&lbl);
    v.append(&entry);
    v.append(&row);
    dlg.set_child(Some(&v));

    let accept: AcceptSlot = AcceptSlot::new(RefCell::new(Some(Box::new(on_accept))));
    let run = {
        let accept = accept.clone();
        let entry_c = entry.clone();
        let dlg_c = dlg.clone();
        move || {
            if let Some(f) = accept.borrow_mut().take() {
                f(entry_c.text().to_string());
            }
            dlg_c.close();
        }
    };
    let run_ok = run.clone();
    ok.connect_clicked(move |_| run_ok());
    entry.connect_activate(move |_| run());
    {
        let dlg_c = dlg.clone();
        cancel.connect_clicked(move |_| dlg_c.close());
    }
    dlg.present();
    glib::idle_add_local_once({
        let entry = entry.clone();
        move || {
            entry.grab_focus();
        }
    });
}

fn prompt_new_folder(window: &ApplicationWindow, state: &Rc<AppState>, do_refresh: RefreshFn) {
    let cur = state.current_path.borrow().clone();
    let w = window.clone();
    prompt_entry(
        window,
        "New Folder",
        &format!("Create folder in {}", cur.display()),
        "Folder name",
        "Untitled Folder",
        "Create",
        move |name| {
            if !name.trim().is_empty() {
                match create_dir(&cur, name.trim()) {
                    Ok(_) => do_refresh(),
                    Err(e) => show_error(&w, &format!("Failed to create folder: {e}")),
                }
            }
        },
    );
}

fn prompt_new_file(window: &ApplicationWindow, state: &Rc<AppState>, do_refresh: RefreshFn) {
    let cur = state.current_path.borrow().clone();
    let w = window.clone();
    prompt_entry(
        window,
        "New File",
        &format!("Create file in {}", cur.display()),
        "File name",
        "Untitled.txt",
        "Create",
        move |name| {
            if !name.trim().is_empty() {
                match create_file(&cur, name.trim()) {
                    Ok(_) => do_refresh(),
                    Err(e) => show_error(&w, &format!("Failed to create file: {e}")),
                }
            }
        },
    );
}

fn prompt_rename(window: &ApplicationWindow, path: &Path, do_refresh: RefreshFn) {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let parent = path.parent().unwrap_or(Path::new("/")).to_path_buf();
    let path_buf = path.to_path_buf();
    let w = window.clone();
    let name_c = name.clone();
    prompt_entry(
        window,
        "Rename",
        &format!("Rename \"{}\"", name),
        "New name",
        &name,
        "Rename",
        move |new_name| {
            if !new_name.trim().is_empty() && new_name != name_c {
                let dst = parent.join(new_name.trim());
                if dst.exists() {
                    show_error(&w, "Destination already exists");
                } else {
                    match rename_path(&path_buf, &dst) {
                        Ok(_) => do_refresh(),
                        Err(e) => show_error(&w, &format!("Rename failed: {e}")),
                    }
                }
            }
        },
    );
}

fn click_select(
    state: &Rc<AppState>,
    flow: &FlowBox,
    list_box: &ListBox,
    status_sel: &Label,
    path: &Path,
) {
    let mut set = HashSet::new();
    set.insert(path.to_path_buf());
    *state.selected.borrow_mut() = set.clone();
    apply_selection(flow, list_box, status_sel, &set);
}

fn disarm_inline_rename(state: &Rc<AppState>) {
    if let Some(id) = state.rename_timer.borrow_mut().take() {
        id.remove();
    }
}

fn arm_inline_rename(
    state: &Rc<AppState>,
    host: &GtkBox,
    label: &Label,
    path: PathBuf,
    center: bool,
    window: &ApplicationWindow,
    refresh: RefreshFn,
) {
    disarm_inline_rename(state);
    let weak_box = host.downgrade();
    let weak_label = label.downgrade();
    let st = state.clone();
    let w = window.clone();
    *state.rename_timer.borrow_mut() = Some(glib::timeout_add_local_once(
        Duration::from_millis(600),
        move || {
            st.rename_timer.borrow_mut().take();
            let (Some(b), Some(l)) = (weak_box.upgrade(), weak_label.upgrade()) else {
                return;
            };
            inline_rename(&b, &l, &path, center, &w, refresh);
        },
    ));
}

fn inline_rename(
    box_: &GtkBox,
    label: &Label,
    path: &Path,
    center: bool,
    window: &ApplicationWindow,
    refresh: RefreshFn,
) {
    let anchor = label.prev_sibling();
    box_.remove(label);
    let entry = Entry::new();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    entry.set_text(&name);
    let stem = name
        .rfind('.')
        .filter(|&i| i > 0)
        .map(|i| name[..i].chars().count() as i32)
        .unwrap_or_else(|| name.chars().count() as i32);
    entry.select_region(0, stem);
    if center {
        gtk::prelude::EntryExt::set_alignment(&entry, 0.5);
        entry.set_width_chars(12);
    } else {
        entry.set_hexpand(true);
    }
    box_.insert_child_after(&entry, anchor.as_ref());
    entry.grab_focus();
    let done = Rc::new(Cell::new(false));
    let commit = {
        let done = done.clone();
        let entry_c = entry.clone();
        let path = path.to_path_buf();
        let w = window.clone();
        let refresh = refresh.clone();
        move || {
            if done.replace(true) {
                return;
            }
            let new_name = entry_c.text().to_string();
            let new_name = new_name.trim();
            if !new_name.is_empty() && new_name != name {
                let dst = path.parent().unwrap_or(Path::new("/")).join(new_name);
                if dst.exists() {
                    show_error(&w, "Destination already exists");
                } else if let Err(e) = rename_path(&path, &dst) {
                    show_error(&w, &format!("Rename failed: {e}"));
                }
            }
            refresh();
        }
    };
    let commit_activate = commit.clone();
    entry.connect_activate(move |_| commit_activate());
    let commit_leave = commit.clone();
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| commit_leave());
    entry.add_controller(focus);
    let done_esc = done.clone();
    let refresh_esc = refresh.clone();
    let key = gtk::EventControllerKey::new();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            done_esc.set(true);
            refresh_esc();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    entry.add_controller(key);
}

fn describe_paths(paths: &[PathBuf]) -> String {
    if paths.len() == 1 {
        return paths[0]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }
    let names: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .collect();
    if names.len() <= 3 {
        names.join(", ")
    } else {
        format!("{}, … ({} items)", names[..3].join(", "), names.len())
    }
}

fn confirm_and_trash(window: &ApplicationWindow, paths: &[PathBuf], do_refresh: RefreshFn) {
    let what = describe_paths(paths);
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(format!("Move {} to Trash?", what))
        .detail("You can restore it from the Trash.")
        .buttons(["Cancel", "Move to Trash"])
        .cancel_button(0)
        .default_button(0)
        .build();
    let paths = paths.to_vec();
    let w = window.clone();
    dialog.choose(Some(window), None::<&gio::Cancellable>, move |res| {
        if res.ok() == Some(1) {
            let mut first_err = None;
            for p in &paths {
                if let Err(e) = trash_path(p) {
                    first_err = Some(e);
                    break;
                }
            }
            match first_err {
                Some(e) => show_error(&w, &format!("Trash failed: {e}")),
                None => do_refresh(),
            }
        }
    });
}

fn confirm_and_delete_permanent(window: &ApplicationWindow, paths: &[PathBuf], do_refresh: RefreshFn) {
    let what = describe_paths(paths);
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(format!("Permanently delete \"{}\"?", what))
        .detail("This cannot be undone.")
        .buttons(["Cancel", "Delete"])
        .cancel_button(0)
        .default_button(0)
        .build();
    let paths = paths.to_vec();
    let w = window.clone();
    dialog.choose(Some(window), None::<&gio::Cancellable>, move |res| {
        if res.ok() == Some(1) {
            let mut first_err = None;
            for p in &paths {
                if let Err(e) = delete_permanent(p) {
                    first_err = Some(e);
                    break;
                }
            }
            match first_err {
                Some(e) => show_error(&w, &format!("Delete failed: {e}")),
                None => do_refresh(),
            }
        }
    });
}

fn show_info(window: &ApplicationWindow, title: &str, body: &gtk::Widget) {
    let dlg = gtk::Window::builder()
        .title(title)
        .transient_for(window)
        .modal(true)
        .default_width(480)
        .build();
    let v = GtkBox::new(Orientation::Vertical, 10);
    v.set_margin_top(16);
    v.set_margin_bottom(16);
    v.set_margin_start(16);
    v.set_margin_end(16);
    v.append(body);
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let close = Button::with_label("Close");
    close.add_css_class("suggested-action");
    row.append(&close);
    v.append(&row);
    dlg.set_child(Some(&v));
    let dlg_c = dlg.clone();
    close.connect_clicked(move |_| dlg_c.close());
    dlg.present();
}

fn show_properties(window: &ApplicationWindow, path: &Path) {
    let meta = std::fs::symlink_metadata(path);
    let title = format!("Properties — {}", path.file_name().unwrap_or_default().to_string_lossy());
    let v = GtkBox::new(Orientation::Vertical, 8);

    let add_row = |title: &str, value: &str, parent: &GtkBox| {
        let h = GtkBox::new(Orientation::Horizontal, 12);
        let t = Label::new(Some(title));
        t.set_xalign(1.0);
        t.set_width_request(120);
        t.add_css_class("dim-label");
        let val = Label::new(Some(value));
        val.set_xalign(0.0);
        val.set_selectable(true);
        val.set_ellipsize(pango::EllipsizeMode::Middle);
        val.set_hexpand(true);
        h.append(&t);
        h.append(&val);
        parent.append(&h);
    };

    add_row("Name:", &path.file_name().unwrap_or_default().to_string_lossy(), &v);
    add_row("Path:", &path.display().to_string(), &v);
    add_row("Type:", mime_guess::from_path(path).first_raw().unwrap_or("-"), &v);
    if let Ok(m) = meta {
        add_row("Size:", &human_size(m.len()), &v);
        if let Ok(modified) = m.modified() {
            add_row("Modified:", &format_time(modified), &v);
        }
        add_row("Permissions:", &format!("{:o}", m.permissions().mode() & 0o777), &v);
        add_row("Symlink:", if m.file_type().is_symlink() { "yes" } else { "no" }, &v);
        if m.file_type().is_symlink() {
            if let Ok(target) = std::fs::read_link(path) {
                add_row("Target:", &target.display().to_string(), &v);
            }
        }
    }
    // Disk usage for dir? quick
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            let count = entries.count();
            add_row("Items:", &format!("{count}"), &v);
        }
    }

    show_info(window, &title, &v.upcast());
}

fn show_properties_multi(window: &ApplicationWindow, paths: &[PathBuf]) {
    let mut total_size: u64 = 0;
    let mut dirs = 0usize;
    let mut files = 0usize;
    let mut bytes_failed = 0u64;
    for p in paths {
        if let Ok(m) = std::fs::symlink_metadata(p) {
            if m.is_dir() {
                dirs += 1;
            } else {
                files += 1;
                total_size += m.len();
            }
        } else {
            bytes_failed += 1;
        }
    }
    let title = format!("Properties — {} items", paths.len());
    let v = GtkBox::new(Orientation::Vertical, 8);
    v.set_margin_top(12);
    v.set_margin_bottom(12);
    v.set_margin_start(12);
    v.set_margin_end(12);

    let add_row = |title: &str, value: &str, parent: &GtkBox| {
        let h = GtkBox::new(Orientation::Horizontal, 12);
        let t = Label::new(Some(title));
        t.set_xalign(1.0);
        t.set_width_request(120);
        t.add_css_class("dim-label");
        let val = Label::new(Some(value));
        val.set_xalign(0.0);
        val.set_selectable(true);
        val.set_hexpand(true);
        h.append(&t);
        h.append(&val);
        parent.append(&h);
    };

    add_row("Items:", &format!("{} ({dirs} folders, {files} files)", paths.len()), &v);
    add_row("Total size:", &human_size(total_size), &v);
    let names: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .collect();
    for (i, n) in names.iter().enumerate().take(8) {
        add_row(&format!("#{}", i + 1), n, &v);
    }
    if names.len() > 8 {
        add_row("…", &format!("and {} more", names.len() - 8), &v);
    }
    if bytes_failed > 0 {
        add_row("Unavailable:", &format!("{bytes_failed}"), &v);
    }

    show_info(window, &title, &v.upcast());
}

fn show_error(window: &ApplicationWindow, msg: &str) {
    let d = gtk::AlertDialog::builder()
        .modal(true)
        .message(msg)
        .buttons(["Close"])
        .cancel_button(0)
        .default_button(0)
        .build();
    d.show(Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_overlap_math() {
        // fully inside
        assert!(rect_overlaps(
            &gdk::Rectangle::new(10, 10, 50, 50),
            &gdk::Rectangle::new(20, 20, 10, 10)
        ));
        // disjoint
        assert!(!rect_overlaps(
            &gdk::Rectangle::new(0, 0, 10, 10),
            &gdk::Rectangle::new(20, 20, 10, 10)
        ));
        // touching edges = no overlap
        assert!(!rect_overlaps(
            &gdk::Rectangle::new(0, 0, 10, 10),
            &gdk::Rectangle::new(10, 0, 10, 10)
        ));
        // zero-size box selects nothing
        assert!(!rect_overlaps(
            &gdk::Rectangle::new(5, 5, 0, 0),
            &gdk::Rectangle::new(5, 5, 10, 10)
        ));
        // partial overlap
        assert!(rect_overlaps(
            &gdk::Rectangle::new(0, 0, 20, 20),
            &gdk::Rectangle::new(10, 10, 20, 20)
        ));
    }

    #[test]
    fn normalized_rect_math() {
        let r = normalized_rect((30.0, 10.0), (10.0, 60.0));
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (10, 10, 20, 50));
        let r = normalized_rect((10.0, 10.0), (30.0, 40.0));
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (10, 10, 20, 30));
        // tiny accidental click -> threshold check rejects it in drag_end
        let r = normalized_rect((10.0, 10.0), (10.9, 10.9));
        let d = (r.width().pow(2) + r.height().pow(2)) as i32;
        assert!(d <= 16);
    }
}
