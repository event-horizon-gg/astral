#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod account;
mod activity;
mod app_updates;
mod application_menu;
mod artwork;
mod batch;
mod bundled;
mod cloud;
mod credentials;
mod daily;
mod discovery;
mod gamepad_settings;
mod icons;
mod installer;
mod library;
mod local_time;
#[cfg(any(target_os = "windows", test))]
mod localized_dates;
mod menu_events;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_dates;
mod notifications;
mod packages;
mod preferences;
mod project;
mod releases;
mod removal;
mod review;
mod schedule;
mod scrollbar;
mod search_input;
mod settings_backup;
mod settings_backup_view;
mod shared_library;
mod startup;
mod sync_addons;
mod sync_engine;
mod sync_format;
mod talents;
mod text_history;
mod tray;
mod type_ahead;
mod typography;
mod ui_icons;

use gpui::{
    App, Application, Bounds, Context, KeyBinding, PathPromptOptions, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use library::{Addon, Client, Definition};
use preferences::Preferences;
use std::{cmp::Ordering, collections::HashMap, path::PathBuf, sync::Arc};

gpui::actions!(astral, [Quit, FindAddon, DismissOverlay]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Discover,
    Sources,
    Backups,
    Settings,
    Account,
}

const QUICK_PAGES: [(Page, &str); 6] = [
    (Page::Library, "package"),
    (Page::Discover, "plus"),
    (Page::Account, "layers"),
    (Page::Backups, "history"),
    (Page::Sources, "globe"),
    (Page::Settings, "gear"),
];

static RAIDER_IO_VERSION: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"v[0-9]{12}").expect("valid version pattern"));

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Discover => "Discover",
            Self::Sources => "Sources",
            Self::Backups => "Activity",
            Self::Settings => "Settings",
            Self::Account => "Profiles & sync",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Updates,
    Manual,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolbarMenu {
    Client,
    Tools,
    Sources,
    Account,
}

struct Astral {
    fixture: bool,
    preferences: Preferences,
    preferences_path: Option<PathBuf>,
    preferences_error: Option<String>,
    folder_error: Option<(Client, String)>,
    detected_roots: [Option<PathBuf>; 2],
    page: Page,
    filter: Filter,
    discovery_source: Option<&'static str>,
    github_search: Option<discovery::Search>,
    project_review: Option<project::ProjectReview>,
    quick_find: Option<usize>,
    quick_find_return: Option<gpui::FocusHandle>,
    quick_find_rows: [gpui::FocusHandle; QUICK_PAGES.len()],
    query: String,
    library_query: String,
    search: gpui::Entity<search_input::SearchInput>,
    search_focus: gpui::FocusHandle,
    profile_name_focus: gpui::FocusHandle,
    profile_review_focus: settings_backup_view::ProfileReviewFocus,
    menu: Option<ToolbarMenu>,
    menu_return_focus: Option<gpui::FocusHandle>,
    client_menu_focus: [gpui::FocusHandle; 2],
    tools_first_focus: gpui::FocusHandle,
    account_first_focus: gpui::FocusHandle,
    root_focus: gpui::FocusHandle,
    main_scroll: gpui::ScrollHandle,
    main_overflows: bool,
    scrollbar_drag: Option<(f32, f32)>,
    brand_icon: Arc<gpui::Image>,
    client_icons: [Arc<gpui::Image>; 2],
    wago_icon: Arc<gpui::Image>,
    catalog: Arc<Vec<Definition>>,
    addons: Vec<Addon>,
    error: Option<String>,
    selected: Option<String>,
    scan_id: u64,
    scanning: bool,
    checking: bool,
    checks: HashMap<String, Result<releases::Check, String>>,
    release_review: Option<review::Review>,
    release_request: u64,
    install_receipt: Option<review::Receipt>,
    removal: Option<removal::Removal>,
    batch: Option<batch::Batch>,
    batch_result: Option<batch::Batch>,
    activity: activity::State,
    account: account::State,
    notifications: notifications::State,
    startup: startup::State,
    app_updates: app_updates::State,
    settings_backups: settings_backup_view::State,
    talent_refreshing: bool,
    daily_running: bool,
    daily_waiting: bool,
    daily_next_at: u64,
    daily_run_error: Option<String>,
    daily_history: Vec<daily::HistoryEntry>,
    daily_history_error: Option<String>,
    daily_schedule_busy: bool,
    daily_schedule_error: Option<String>,
    talent_refresh_result: Option<(PathBuf, Result<(), String>)>,
}

impl Astral {
    fn new(
        roots: [Option<PathBuf>; 2],
        fixture: Option<&std::path::Path>,
        fixture_client: Option<Client>,
        fixture_light: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let preferences_path = match fixture {
            Some(folder) => Some(folder.join("preferences.json")),
            None => preferences::path(),
        };
        let (mut preferences, preferences_error) = match preferences_path.as_deref() {
            Some(path) => match Preferences::load(path) {
                Ok(preferences) => (preferences, None),
                Err(error) => (
                    Preferences::default(),
                    Some(format!("Couldn't read Astral settings. {error}")),
                ),
            },
            None => (
                Preferences::default(),
                Some("Couldn't find a place to save Astral settings.".into()),
            ),
        };
        if fixture.is_some() {
            preferences.folders = Default::default();
            preferences.client = fixture_client.unwrap_or_default();
            preferences.light_theme = fixture_light;
        }
        let detected_roots = preferences.detected_folders(&roots);
        if preferences.folders[preferences.client.index()].is_none()
            && detected_roots[preferences.client.index()].is_none()
            && let Some(client) = Client::ALL.into_iter().find(|client| {
                preferences.folders[client.index()].is_some()
                    || detected_roots[client.index()].is_some()
            })
        {
            preferences.client = client;
        }
        let mut catalog = library::catalog();
        catalog.extend(preferences.projects.iter().cloned());
        let search = cx.new(search_input::SearchInput::new);
        let search_focus = search.read(cx).focus.clone();
        cx.subscribe(
            &search,
            |app: &mut Self, _, event: &search_input::Changed, cx| {
                app.query.clone_from(&event.0);
                if app.page == Page::Library {
                    app.library_query.clone_from(&event.0);
                }
                if app.page == Page::Discover && app.query.is_empty() {
                    app.github_search = None;
                }
                app.selected = None;
                cx.notify();
            },
        )
        .detach();
        let mut app = Self {
            fixture: fixture.is_some(),
            preferences,
            preferences_path,
            preferences_error,
            folder_error: None,
            detected_roots,
            page: Page::Library,
            filter: Filter::All,
            discovery_source: None,
            github_search: None,
            project_review: None,
            quick_find: None,
            quick_find_return: None,
            quick_find_rows: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            query: String::new(),
            library_query: String::new(),
            search,
            search_focus,
            profile_name_focus: cx.focus_handle().tab_stop(true),
            profile_review_focus: settings_backup_view::ProfileReviewFocus::new(cx),
            menu: None,
            menu_return_focus: None,
            client_menu_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            tools_first_focus: cx.focus_handle().tab_stop(true),
            account_first_focus: cx.focus_handle().tab_stop(true),
            root_focus: cx.focus_handle(),
            main_scroll: gpui::ScrollHandle::new(),
            main_overflows: false,
            scrollbar_drag: None,
            brand_icon: artwork::bundled(include_bytes!("../../assets/icon.png")),
            client_icons: [
                Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    include_bytes!("../../assets/games/wow-retail.png").to_vec(),
                )),
                Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    include_bytes!("../../assets/games/wow-forever.png").to_vec(),
                )),
            ],
            catalog: Arc::new(catalog),
            wago_icon: Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                include_bytes!("../../assets/providers/wago.ico").to_vec(),
            )),
            addons: Vec::new(),
            error: None,
            selected: None,
            scan_id: 0,
            scanning: false,
            checking: false,
            checks: HashMap::new(),
            release_review: None,
            release_request: 0,
            install_receipt: None,
            removal: None,
            batch: None,
            batch_result: None,
            activity: activity::State::default(),
            account: account::State::default(),
            notifications: notifications::State::default(),
            startup: startup::State::default(),
            app_updates: app_updates::State::default(),
            settings_backups: settings_backup_view::State::default(),
            talent_refreshing: false,
            daily_running: false,
            daily_waiting: false,
            daily_next_at: 0,
            daily_run_error: None,
            daily_history: Vec::new(),
            daily_history_error: None,
            daily_schedule_busy: false,
            daily_schedule_error: None,
            talent_refresh_result: None,
        };
        app.refresh(cx);
        if !app.fixture {
            app.load_daily_history();
            app.start_daily_timer(cx);
            app.start_auto_sync_timer(cx);
            app.start_app_update_timer(cx);
            if schedule::SUPPORTED && app.preferences.background_updates {
                app.configure_daily_schedule(true, cx);
            }
        }
        app
    }

    fn root(&self, client: Client) -> Option<&PathBuf> {
        self.preferences.folders[client.index()]
            .as_ref()
            .or(self.detected_roots[client.index()].as_ref())
    }

    fn save_preferences(&mut self) {
        if let Some(path) = &self.preferences_path {
            self.preferences_error = self
                .preferences
                .save(path)
                .err()
                .map(|error| format!("Couldn't save Astral settings. {error}"));
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.scan_library(false, cx);
    }

    fn refresh_after_change(&mut self, cx: &mut Context<Self>) {
        self.scan_library(true, cx);
    }

    fn scan_library(&mut self, keep_checks: bool, cx: &mut Context<Self>) {
        let mut previous = HashMap::new();
        if keep_checks {
            for addon in &self.addons {
                if let Some(check) = self.checks.remove(&addon.folder) {
                    previous.insert(addon.folder.clone(), (addon.version.clone(), check));
                }
            }
        }
        self.scan_id += 1;
        self.error = None;
        self.selected = None;
        self.release_review = None;
        self.addons.clear();
        self.checks.clear();
        self.scanning = false;
        self.checking = false;
        if self.page == Page::Backups {
            self.load_backups(cx);
        }
        let client = self.preferences.client;
        if let Some(root) = self.root(client).cloned() {
            let catalog = self.catalog.clone();
            let scan_id = self.scan_id;
            self.scanning = true;
            let task = cx
                .background_executor()
                .spawn(async move { library::scan_library(&root, client, &catalog) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |app, cx| {
                    if app.scan_id != scan_id {
                        return;
                    }
                    app.scanning = false;
                    match result {
                        Ok(addons) => {
                            for addon in &addons {
                                if let Some((version, check)) = previous.remove(&addon.folder)
                                    && version == addon.version
                                {
                                    app.checks.insert(addon.folder.clone(), check);
                                }
                            }
                            app.addons = addons;
                        }
                        Err(error) => app.error = Some(format!("Couldn't read addons. {error}")),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn check_updates(&mut self, only: Option<&str>, cx: &mut Context<Self>) {
        if self.scanning || self.checking || self.addon_batch_running() {
            return;
        }
        let work: Vec<_> = self
            .addons
            .iter()
            .filter(|addon| only.is_none_or(|folder| addon.folder == folder))
            .filter_map(|addon| {
                let index = self.catalog.iter().position(|source| {
                    source.folder == addon.folder
                        && (source.repo.is_some() || source.bundled == Some(true))
                        && source.supports(self.preferences.client)
                })?;
                Some((index, addon.folder.clone(), addon.version.clone()))
            })
            .collect();
        if work.is_empty() {
            return;
        }
        self.checking = true;
        let scan_id = self.scan_id;
        let catalog = self.catalog.clone();
        cx.spawn(async move |this, cx| {
            let agent = releases::client();
            let mut work = work.into_iter();
            loop {
                let tasks: Vec<_> = work
                    .by_ref()
                    .take(3)
                    .map(|(index, folder, installed)| {
                        let catalog = catalog.clone();
                        let agent = agent.clone();
                        let task = cx.background_executor().spawn(async move {
                            releases::check(&agent, &catalog[index], &installed)
                        });
                        (folder, task)
                    })
                    .collect();
                if tasks.is_empty() {
                    break;
                }
                for (folder, task) in tasks {
                    let result = task.await;
                    let should_continue = this.update(cx, |app, cx| {
                        if app.scan_id != scan_id {
                            return false;
                        }
                        app.checks.insert(folder, result);
                        cx.notify();
                        true
                    });
                    if !matches!(should_continue, Ok(true)) {
                        return;
                    }
                }
            }
            let _ = this.update(cx, |app, cx| {
                if app.scan_id == scan_id {
                    app.checking = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn update_status(&self, addon: &Addon) -> &'static str {
        match self.checks.get(&addon.folder) {
            Some(Ok(check)) => match check.comparison {
                Some(Ordering::Greater) => "Update available",
                Some(Ordering::Equal) => "Up to date",
                Some(Ordering::Less) => "Ahead of release",
                None => "Review version",
            },
            Some(Err(_)) => "Check failed",
            None if addon.source != "Manual" => "Not checked",
            None => "",
        }
    }

    // Retained for the post-port feature pass; absent from Electron's drawer.
    #[allow(dead_code)]
    fn refresh_talents(&mut self, cx: &mut Context<Self>) {
        if self.talent_refreshing
            || self.addon_batch_running()
            || self.preferences.client != Client::Retail
        {
            return;
        }
        let Some(root) = self.root(Client::Retail).cloned() else {
            return;
        };
        self.talent_refreshing = true;
        self.talent_refresh_result = None;
        let target = root.clone();
        let task = cx
            .background_executor()
            .spawn(async move { talents::refresh(&target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.talent_refreshing = false;
                app.talent_refresh_result = Some((root, result));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_root(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        if self.fixture {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select your WoW game folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = picker.await else {
                return;
            };
            let folder = result
                .map(|paths| paths.and_then(|paths| paths.into_iter().next()))
                .and_then(|folder| {
                    Ok(folder
                        .map(|path| library::client_for_root(&path).map(|target| (target, path)))
                        .transpose()?)
                });
            let _ = this.update(cx, |app, cx| {
                match folder {
                    Ok(Some((target, path))) => {
                        if app.addon_batch_running() || app.settings_write_in_progress() {
                            return;
                        }
                        app.folder_error = None;
                        app.preferences.folders[target.index()] = Some(path);
                        app.detected_roots = app
                            .preferences
                            .detected_folders(&Client::ALL.map(Client::detect));
                        app.change_client(target, cx);
                    }
                    Err(error) => app.folder_error = Some((client, error.to_string())),
                    Ok(None) => return,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn change_client(&mut self, target: Client, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        self.close_cloud_profile_manager();
        self.batch_result = None;
        self.github_search = None;
        self.project_review = None;
        self.preferences.client = target;
        if self.fixture && self.account_name().is_some() {
            self.open_signed_account_fixture();
        }
        self.save_preferences();
        self.library_query.clear();
        if self.page != Page::Discover {
            self.set_query("", cx);
        }
        self.refresh(cx);
        self.load_last_sync(cx);
        if matches!(self.page, Page::Settings | Page::Account) {
            self.load_settings_backups(None, cx);
        }
        if self.page == Page::Account {
            self.dismiss_cloud_profile_review(cx);
            self.open_account(cx);
        }
    }
}

fn icon(path: &'static str, size: f32, color: u32) -> gpui::Svg {
    gpui::svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

fn keyboard_button(
    button: gpui::Stateful<gpui::Div>,
    palette: Palette,
    activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    button
        .focusable()
        .tab_stop(true)
        .focus(move |style| style.border_color(rgb(palette.accent)).border_1())
        .on_click(move |_, window, cx| activate(&(), window, cx))
}

#[derive(Clone, Copy)]
struct Palette {
    background: u32,
    text: u32,
    muted: u32,
    accent: u32,
    line: gpui::Rgba,
    hover: gpui::Rgba,
    surface: u32,
    panel: u32,
    error: u32,
    success: u32,
}

impl Palette {
    fn focus_ring(self) -> Vec<gpui::BoxShadow> {
        self.focus_ring_on(self.background)
    }

    fn focus_ring_on(self, background: u32) -> Vec<gpui::BoxShadow> {
        [(self.accent, 6.), (background, 4.)]
            .into_iter()
            .map(|(color, spread)| gpui::BoxShadow {
                color: rgb(color).into(),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(spread),
            })
            .collect()
    }

    fn profile_avatar(self, name: Option<&str>, large: bool) -> gpui::Div {
        let size = if large { 44. } else { 35. };
        div()
            .size(px(size))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(self.line)
            .bg(rgb(self.surface))
            .text_size(px(if large { 20. } else { 15. }))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .when_some(name, |avatar, name| {
                avatar.child(name.chars().next().unwrap_or('A').to_string())
            })
            .when(name.is_none(), |avatar| {
                avatar.child(icon("cloud", if large { 22. } else { 18. }, self.text))
            })
    }

    fn new(light: bool) -> Self {
        if light {
            Self {
                background: 0xfdfcf9,
                text: 0x101114,
                muted: 0x666970,
                accent: 0x0057ff,
                line: gpui::rgba(0x10111428),
                hover: gpui::rgba(0x10111406),
                surface: 0xf3f2ef,
                panel: 0xffffff,
                error: 0xa92920,
                success: 0x067658,
            }
        } else {
            Self {
                background: 0x101114,
                text: 0xf2f1ed,
                muted: 0xa0a3ac,
                accent: 0x527dff,
                line: gpui::rgba(0xffffff20),
                hover: gpui::rgba(0xffffff08),
                surface: 0x191b20,
                panel: 0x191b20,
                error: 0xffad9e,
                success: 0x62c5a3,
            }
        }
    }
    fn label(self, text: impl Into<SharedString>) -> gpui::Div {
        div()
            .text_sm()
            .text_color(rgb(self.muted))
            .child(text.into())
    }

    fn tab(self, id: &'static str, name: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .cursor_pointer()
            .py_2()
            .text_sm()
            .border_b_2()
            .border_color(rgb(if active { self.accent } else { self.background }))
            .text_color(rgb(if active { self.text } else { self.muted }))
            .hover(move |style| style.text_color(rgb(self.text)))
            .child(name)
    }

    fn button(
        self,
        id: impl Into<gpui::ElementId>,
        name: impl Into<SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        self.styled_button(id, name, false)
    }

    fn primary_button(
        self,
        id: &'static str,
        name: impl Into<SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        self.styled_button(id, name, true)
    }

    fn styled_button(
        self,
        id: impl Into<gpui::ElementId>,
        name: impl Into<SharedString>,
        primary: bool,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .h(px(39.))
            .px(px(15.))
            .rounded(px(5.))
            .border_1()
            .border_color(if primary { rgb(self.accent) } else { self.line })
            .text_size(px(12.))
            .font_weight(gpui::FontWeight(550.))
            .when(primary, |button| {
                button
                    .bg(rgb(self.accent))
                    .text_color(rgb(0xffffff))
                    .hover(move |style| {
                        let mut color = rgb(self.accent);
                        color.r = (color.r * 1.1).min(1.);
                        color.g = (color.g * 1.1).min(1.);
                        color.b = (color.b * 1.1).min(1.);
                        style.bg(color).border_color(color)
                    })
            })
            .when(!primary, |button| {
                button.hover(move |style| style.bg(self.hover))
            })
            .child(name.into())
    }

    fn icon_button(self, id: &'static str, glyph: &'static str) -> gpui::Stateful<gpui::Div> {
        self.icon_button_sized(id, glyph, 18.)
    }

    fn icon_button_sized(
        self,
        id: &'static str,
        glyph: &'static str,
        size: f32,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .size(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(self.hover))
            .child(icon(glyph, size, self.muted))
    }

    fn text_button(
        self,
        id: impl Into<gpui::ElementId>,
        name: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(rgb(self.muted))
            .border_1()
            .border_color(gpui::transparent_black())
            .hover(move |style| style.text_color(rgb(self.accent)))
            .child(name)
    }

    fn checkbox_mark(self, checked: Option<bool>, size: f32) -> gpui::Div {
        let light = self.panel == 0xffffff;
        let mark_color = if light { 0xffffff } else { 0x101114 };
        div()
            .size(px(size))
            .flex_shrink_0()
            .rounded(px(2.))
            .border_1()
            .border_color(rgb(if checked == Some(false) {
                if light { 0x767676 } else { 0x858585 }
            } else {
                self.accent
            }))
            .flex()
            .items_center()
            .justify_center()
            .when(checked != Some(false), |mark| mark.bg(rgb(self.accent)))
            .when(checked == Some(false), |mark| {
                mark.bg(rgb(if light { 0xffffff } else { 0x3b3b3b }))
            })
            .when(checked == Some(true), |mark| {
                mark.child(icon("check", size - 3., mark_color))
            })
            .when(checked.is_none(), |mark| {
                mark.child(div().w(px(size / 2.)).h(px(2.)).bg(rgb(mark_color)))
            })
    }

    fn menu_row(
        self,
        id: &'static str,
        glyph: &'static str,
        name: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .w_full()
            .flex()
            .items_center()
            .gap_0()
            .px(px(10.))
            .h(px(42.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.))
            .hover(move |style| style.bg(self.hover))
            .child(icon(glyph, if glyph == "cloud" { 17. } else { 16. }, self.text).mr(px(12.)))
            .child(div().flex_1().child(name))
    }

    fn settings_section(
        self,
        title: impl Into<SharedString>,
        controls: gpui::Div,
        compact: bool,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_shrink_0()
            .gap_0()
            .py(px(18.))
            .border_b_1()
            .border_color(self.line)
            .child(
                div()
                    .w(gpui::relative(if compact { 0.32 } else { 0.34 }))
                    .flex_shrink_0()
                    .text_size(px(if compact { 21. } else { 24. }))
                    .font_weight(gpui::FontWeight(650.))
                    .child(typography::tracked(
                        title,
                        if compact { 21. } else { 24. },
                        1.2,
                        -0.8,
                    )),
            )
            .child(controls.flex_1().min_w_0())
    }

    fn setting_checkbox(
        self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        enabled: bool,
        change: fn(&mut Astral, &mut Context<Astral>),
        cx: &Context<Astral>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(25.))
            .mt(px(18.))
            .pt(px(9.))
            .pb(px(12.))
            .border_b_1()
            .border_color(self.line)
            .text_size(px(12.))
            .child(label)
            .child(self.checkbox_mark(Some(checked), 13.))
            .when(!enabled, |row| row.opacity(0.55))
            .when(enabled, |row| {
                row.focusable()
                    .tab_stop(true)
                    .cursor_pointer()
                    .focus(move |style| style.border_color(rgb(self.accent)))
                    .on_click(cx.listener(move |app, _, _, cx| change(app, cx)))
            })
    }
}

impl Astral {
    fn settings(&self, palette: Palette, compact: bool, cx: &Context<Self>) -> gpui::Div {
        let client = self.preferences.client;
        let root = self.root(client);
        let folder = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p(px(13.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(palette.line)
                    .bg(rgb(palette.surface))
                    .child(icon("folder", 17., palette.text))
                    .child(
                        palette
                            .label(root.map_or_else(
                                || "No game folder selected".into(),
                                |path| path.display().to_string(),
                            ))
                            .flex_1()
                            .min_w_0()
                            .font_family(if cfg!(target_os = "macos") {
                                "Menlo"
                            } else {
                                "Consolas"
                            })
                            .text_size(px(11.))
                            .line_height(px(13.2)),
                    )
                    .child({
                        let change = div()
                            .id("change-wow-folder")
                            .cursor_pointer()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .line_height(px(13.2))
                            .text_color(rgb(palette.muted))
                            .hover(move |style| style.text_color(rgb(palette.text)))
                            .child("Change");
                        if self.fixture
                            || self.addon_batch_running()
                            || self.settings_write_in_progress()
                        {
                            change.opacity(0.45).cursor_default()
                        } else {
                            keyboard_button(
                                change,
                                palette,
                                cx.listener(move |app, _: &(), _, cx| {
                                    app.choose_root(client, cx);
                                }),
                            )
                        }
                    }),
            )
            .when_some(
                self.folder_error
                    .as_ref()
                    .filter(|(game, _)| *game == client),
                |row, (_, error)| {
                    row.child(palette.label(error.clone()).text_color(rgb(palette.error)))
                },
            )
            .child(self.local_account_picker(palette, cx));
        div()
            .border_t_1()
            .border_color(rgb(palette.text))
            .child(palette.settings_section(format!("WoW: {}", client.label()), folder, compact))
            .when(startup::SUPPORTED, |settings| {
                settings.child(palette.settings_section(
                    "Startup",
                    self.startup_settings(palette, cx),
                    compact,
                ))
            })
            .child(palette.settings_section(
                "Addon updates",
                self.daily_settings(palette, cx),
                compact,
            ))
            .child(palette.settings_section(
                "Astral updates",
                self.app_update_settings(palette, cx),
                compact,
            ))
    }

    fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            if self.page == Page::Account {
                self.close_account_overlays();
            }
            self.close_cloud_profile_manager();
            let query = if page == Page::Library {
                self.library_query.clone()
            } else {
                String::new()
            };
            // Discovery is a separately mounted component in Electron: its
            // query/results/filter reset after leaving, unlike Library search.
            if self.page == Page::Discover {
                self.github_search = None;
                self.discovery_source = None;
                if let Some(review) = &mut self.project_review {
                    review.leave_discovery();
                }
            }
            self.query = query.clone();
            self.search.update(cx, |input, cx| input.set(&query, cx));
            self.dismiss_cloud_profile_review(cx);
            self.dismiss_local_profile_review(cx);
            self.dismiss_account_profile_picker(cx);
            self.profile_review_focus.return_to = None;
        }
        self.page = page;
        self.menu = None;
        self.menu_return_focus = None;
        self.selected = None;
        self.quick_find = None;
        self.quick_find_return = None;
        self.root_focus.focus(window);
        if page == Page::Discover {
            self.poll_profile_game_status(cx);
        }
        if page == Page::Backups {
            self.load_daily_history();
            self.load_backups(cx);
        }
        if matches!(page, Page::Settings | Page::Account) {
            self.poll_profile_game_status(cx);
            let client = self.preferences.client;
            let selected = self
                .root(client)
                .and_then(|root| self.selected_local_account(client, root));
            self.load_settings_backups(selected, cx);
        }
        if page == Page::Account {
            self.open_account(cx);
        }
        cx.notify();
    }

    fn sources_page(&self, palette: Palette, compact: bool) -> gpui::Div {
        let logo_color = if self.preferences.light_theme {
            0x000000
        } else {
            0xffffff
        };
        let github_count = self
            .addons
            .iter()
            .filter(|addon| {
                self.definition(addon)
                    .is_some_and(|source| source.repo.is_some())
            })
            .count();
        div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(palette.text))
            .children([
                ("GitHub", "github", "https://github.com", true),
                ("Wago", "wago", "https://addons.wago.io", false),
                ("CurseForge", "curseforge", "https://www.curseforge.com/wow", false),
            ].map(|(name, glyph, url, available)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(if compact { 22. } else { 30. }))
                    .py(px(32.))
                    .border_b_1()
                    .border_color(palette.line)
                    .child(
                        div()
                            .size(px(75.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(palette.surface))
                            .rounded(px(9.))
                            .border_1()
                            .border_color(palette.line)
                            .when(glyph == "wago", |mark| {
                                mark.child(gpui::img(self.wago_icon.clone()).size(px(32.)))
                            })
                            .when(glyph != "wago", |mark| mark.child(icon(glyph, 42., logo_color))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(gpui::FontWeight::BOLD).child(typography::tracked(name, 28., 1.2, -1.)))
                            .when(available, |details| details.child(
                                palette.label(format!("{github_count} installed addon groups use GitHub releases"))
                                    .mt(px(8.)).text_size(px(11.)).line_height(px(13.2)),
                            ))
                            .child(
                                div().id(glyph).mt(px(17.)).flex().items_center().gap(px(7.))
                                    .group(glyph)
                                    .focusable().tab_stop(true)
                                    .rounded(px(4.))
                                    .focus(move |style| style.shadow(vec![gpui::BoxShadow {
                                        color: rgb(palette.accent).into(),
                                        offset: gpui::point(px(0.), px(0.)),
                                        blur_radius: px(0.),
                                        spread_radius: px(2.),
                                    }]))
                                    .cursor_pointer().text_size(px(11.)).line_height(px(13.2)).text_color(rgb(palette.muted))
                                    .hover(move |style| style.text_color(rgb(palette.accent)))
                                    .child(format!("Visit {name}"))
                                    .child(icon("arrow-up-right", 14., palette.muted)
                                        .group_hover(glyph, move |style| style.text_color(rgb(palette.accent))))
                                    .on_click(move |_, _, cx| cx.open_url(url)),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex().items_center().gap(px(7.)).px(px(10.)).py(px(7.))
                            .border_1().border_color(palette.line).rounded_full()
                            .text_size(px(11.))
                            .line_height(px(13.2))
                            .when(available, |badge| badge.child(icon("check", 13., palette.text)).child("Available"))
                            .when(!available, |badge| badge.text_color(rgb(palette.muted))
                                .child(div().size(px(5.)).rounded_full().bg(rgb(0xd6b786)))
                                .child("Unavailable")),
                    )
            }))
    }

    fn toggle_quick_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismiss_quick_find(window, cx) {
            return;
        }
        if self.profile_review_focus.open
            || self.sync_dialog_open()
            || self.release_review.is_some()
            || self.activity.has_confirmation()
            || self.removal.is_some()
            || self.batch.is_some()
            || self.project_review.is_some()
        {
            return;
        }
        self.menu = None;
        self.menu_return_focus = None;
        self.selected = None;
        self.quick_find_return = window.focused(cx);
        self.quick_find = Some(0);
        self.root_focus.focus(window);
        cx.notify();
    }

    fn main_scrollbar(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let metrics = scrollbar::Metrics::new(
            f32::from(window.viewport_size().height),
            f32::from(self.main_scroll.max_offset().height),
            f32::from(self.main_scroll.offset().y),
        )?;
        Some(
            div()
                .id("main-scrollbar")
                .absolute()
                .top_0()
                .right(px(-9.))
                .bottom_0()
                .w(px(9.))
                .bg(rgb(0x111516))
                .occlude()
                .on_mouse_move(
                    cx.listener(|app, event, window, cx| app.drag_scrollbar(event, window, cx)),
                )
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|app, _, _, cx| app.release_scrollbar(cx)),
                )
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|app, _, _, cx| app.release_scrollbar(cx)),
                )
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |app, event: &gpui::MouseDownEvent, _, cx| {
                        let y = f32::from(event.position.y);
                        if metrics.contains_thumb(y) {
                            app.scrollbar_drag = Some((y, metrics.thumb_top));
                        } else {
                            let target =
                                metrics.page_offset(y, f32::from(app.main_scroll.offset().y));
                            app.main_scroll.set_offset(gpui::point(px(0.), px(target)));
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(metrics.thumb_top))
                        .left_0()
                        .w(px(9.))
                        .h(px(metrics.thumb_height))
                        .rounded(px(5.))
                        .border(px(2.))
                        .border_color(rgb(0x111516))
                        .bg(rgb(0x33473b)),
                ),
        )
    }

    fn drag_scrollbar(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pointer_y, thumb_top)) = self.scrollbar_drag else {
            return;
        };
        if let Some(metrics) = scrollbar::Metrics::new(
            f32::from(window.viewport_size().height),
            f32::from(self.main_scroll.max_offset().height),
            f32::from(self.main_scroll.offset().y),
        ) {
            let top = thumb_top + f32::from(event.position.y) - pointer_y;
            self.main_scroll
                .set_offset(gpui::point(px(0.), px(metrics.offset_for_top(top))));
            cx.notify();
        }
    }

    fn release_scrollbar(&mut self, cx: &mut Context<Self>) {
        if self.scrollbar_drag.take().is_some() {
            cx.notify();
        }
    }

    fn dismiss_quick_find(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.quick_find.take().is_none() {
            return false;
        }
        self.quick_find_return
            .take()
            .as_ref()
            .unwrap_or(&self.root_focus)
            .focus(window);
        cx.notify();
        true
    }

    fn quick_find_dialog(
        &self,
        selected: usize,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("quick-find-dialog")
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("quick-find-backdrop")
                    .absolute()
                    .top(px(78.))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .bg(gpui::rgba(0x050b08b3))
                    .occlude()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dismiss_quick_find(window, cx);
                    })),
            )
            .child(
                div()
                    .id("quick-find-panel")
                    .occlude()
                    .w(px(470.))
                    .max_w(gpui::relative(0.90))
                    .p(px(10.))
                    .rounded(px(14.))
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(palette.line)
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::rgba(0x00000044).into(),
                        offset: gpui::point(px(0.), px(20.)),
                        blur_radius: px(35.),
                        spread_radius: px(0.),
                    }])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .pt(px(16.))
                            .pb(px(22.))
                            .px_3()
                            .child(icon("search", 18., 0x88a291))
                            .child(
                                palette
                                    .label("Go to")
                                    .flex_1()
                                    .text_size(px(12.))
                                    .line_height(px(14.4))
                                    .font_weight(gpui::FontWeight(450.))
                                    .text_color(rgb(0x88a291)),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(gpui::rgba(0xffffff14))
                                    .text_size(px(10.))
                                    .line_height(px(12.))
                                    .font_weight(gpui::FontWeight(500.))
                                    .text_color(rgb(0x85908c))
                                    .child("ESC"),
                            ),
                    )
                    .children(
                        QUICK_PAGES
                            .into_iter()
                            .enumerate()
                            .map(|(index, (page, glyph))| {
                                keyboard_button(
                                    div()
                                        .id(page.title())
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .rounded(px(7.))
                                        .text_size(px(12.))
                                        .line_height(px(14.4))
                                        .h(px(44.))
                                        .px(px(14.))
                                        .gap(px(13.))
                                        .hover(move |style| style.bg(palette.hover))
                                        .track_focus(&self.quick_find_rows[index])
                                        .when(
                                            selected == index
                                                && self.quick_find_rows[index].is_focused(window),
                                            |row| row.bg(palette.hover),
                                        )
                                        .child(icon(glyph, 18., palette.text))
                                        .child(page.title())
                                        .child(div().flex_1())
                                        .child(icon("arrow-right", 15., palette.text)),
                                    palette,
                                    cx.listener(move |app, _: &(), window, cx| {
                                        app.navigate(page, window, cx)
                                    }),
                                )
                            }),
                    ),
            )
    }

    fn toggle_menu(&mut self, menu: ToolbarMenu, window: &mut Window, cx: &mut Context<Self>) {
        if menu == ToolbarMenu::Client
            && (self.addon_batch_running() || self.settings_write_in_progress())
        {
            return;
        }
        if self.menu == Some(menu) {
            self.close_toolbar_menu(window, cx);
        } else {
            self.menu = Some(menu);
            self.menu_return_focus = window.focused(cx);
            self.selected = None;
            if menu == ToolbarMenu::Client {
                self.client_menu_focus[self.preferences.client.index()].focus(window);
            }
            cx.notify();
        }
    }

    fn close_toolbar_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(focus) = self.menu_return_focus.take() {
            focus.focus(window);
        }
        cx.notify();
    }

    fn return_to_menu_trigger_on_shift_tab(
        &self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "tab"
            && event.keystroke.modifiers.shift
            && let Some(trigger) = &self.menu_return_focus
        {
            trigger.focus(window);
            cx.stop_propagation();
        }
    }

    fn set_query(&mut self, value: &str, cx: &mut Context<Self>) {
        self.query = value.to_owned();
        if self.page == Page::Library {
            self.library_query = value.to_owned();
        }
        self.search.update(cx, |input, cx| input.set(value, cx));
    }

    fn search_box(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.preferences.light_theme);
        let focused = self.search_focus.is_focused(window);
        let discover = self.page == Page::Discover;
        let placeholder = if discover {
            "Search addons…"
        } else {
            "Find an addon…"
        };
        let input = search_input::field(&self.search, placeholder, palette.muted, cx);
        div()
            .id("addon-search")
            .when(discover, |field| {
                field
                    .flex_1()
                    .max_w(px(600.))
                    .h(px(49.))
                    .bg(rgb(palette.panel))
            })
            .when(!discover, |field| field.w(px(204.)).h(px(39.)))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(if focused {
                rgb(palette.accent)
            } else {
                palette.line
            })
            .text_size(px(12.))
            .when(discover, |field| {
                field
                    .text_size(px(14.))
                    .px(px(14.))
                    .bg(rgb(palette.surface))
            })
            .child(icon("search", 15., palette.muted))
            .child(input)
            .when(!discover && !self.query.is_empty(), |field| {
                field.child(keyboard_button(
                    div().id("clear-search").cursor_pointer().px_1().child(icon(
                        "close",
                        13.,
                        palette.muted,
                    )),
                    palette,
                    cx.listener(|app, _: &(), window, cx| {
                        app.set_query("", cx);
                        app.search_focus.focus(window);
                        cx.notify();
                    }),
                ))
            })
            .on_click(cx.listener(|app, _, window, _| app.search_focus.focus(window)))
            .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" && app.page == Page::Discover {
                    app.search_github(cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn definition(&self, addon: &Addon) -> Option<&Definition> {
        self.catalog.iter().find(|source| {
            source.folder == addon.folder && source.supports(self.preferences.client)
        })
    }

    fn addon_icon(&self, addon: &Addon, size: f32) -> gpui::Div {
        let palette = Palette::new(self.preferences.light_theme);

        let artwork = addon
            .artwork
            .clone()
            .or_else(|| (addon.source == "Astral").then(|| self.brand_icon.clone()));
        self.artwork_tile(
            artwork,
            size,
            self.definition(addon)
                .and_then(|source| source.color.as_deref())
                .unwrap_or("slate"),
            palette,
        )
    }

    fn artwork_tile(
        &self,
        artwork: Option<Arc<gpui::Image>>,
        size: f32,
        color: &str,
        palette: Palette,
    ) -> gpui::Div {
        let tile = div()
            .size(px(size))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette.surface))
            .text_color(rgb(palette.accent))
            .text_xl()
            .font_weight(gpui::FontWeight::BOLD);
        match artwork {
            Some(image) => {
                tile.child(gpui::img(image).size(px(size - if size < 50. { 10. } else { 20. })))
            }
            None => {
                let (foreground, start, end) = match color {
                    "violet" => (0xc8bbdf, 0x3c334b, 0x27232f),
                    "amber" => (0xe5cf9c, 0x4c4229, 0x2d291e),
                    "teal" => (0xa7d4c5, 0x284c43, 0x1d302c),
                    "rose" => (0xd3b5b4, 0x483335, 0x2d2426),
                    "orange" => (0xe3ba91, 0x4b3728, 0x30271f),
                    "blue" => (0xafc3e1, 0x2c4057, 0x202a37),
                    "sky" => (0xb4d7e3, 0x2c424a, 0x222f35),
                    "mint" => (0xafd7b9, 0x32483a, 0x243129),
                    _ => (0xb3bfba, 0x36413b, 0x252e29),
                };
                tile.when(size >= 50., |tile| {
                    tile.bg(gpui::linear_gradient(
                        135.,
                        gpui::linear_color_stop(rgb(start), 0.),
                        gpui::linear_color_stop(rgb(end), 1.),
                    ))
                })
                .child(icon(
                    "package",
                    if size > 90. { 28. } else { 19. },
                    foreground,
                ))
            }
        }
    }

    fn tile(&self, addon: &Addon, compact: bool, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.preferences.light_theme);
        let folder = addon.folder.clone();
        let row_key = folder.clone();
        let definition = self.definition(addon);
        let (status, status_color, status_icon) = match self.update_status(addon) {
            "Update available" => ("Update ready", palette.accent, None),
            "Up to date" | "Ahead of release" => ("Up to date", 0x17ab79, Some("check")),
            "Check failed" => ("Needs attention", palette.error, Some("alert")),
            "Not checked" if definition.is_some_and(|source| source.local == Some(true)) => {
                ("Development", palette.muted, None)
            }
            status => (status, palette.muted, None),
        };
        let release = definition
            .filter(|_| status == "Update ready")
            .map(|source| source.id.clone());
        let ready = release.is_some();
        let artwork = self.addon_icon(addon, if compact { 62. } else { 84. });
        // The reference's three explicit grid rows total 83px. Its 84px
        // artwork can overflow that track without increasing the row height.
        let artwork = div()
            .w(px(if compact { 62. } else { 84. }))
            .h(px(if compact { 70. } else { 83. }))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(artwork.when(compact, |artwork| artwork.h(px(70.))));
        let version = if definition.is_some_and(|source| source.id == "rio") {
            RAIDER_IO_VERSION
                .find(&addon.version)
                .map_or(addon.version.as_str(), |date| date.as_str())
        } else {
            &addon.version
        };
        let action = palette
            .icon_button("addon-row-action", if ready { "download" } else { "more" })
            .w(px(if compact { 18. } else { 22. }))
            .flex_shrink_0();
        let action = if self.scanning
            || self.addon_batch_running()
            || (ready && self.profile_game_running())
        {
            action.opacity(0.45).cursor_default()
        } else {
            keyboard_button(
                action,
                palette,
                cx.listener(move |app, _: &(), window, cx| {
                    if let Some(source) = &release {
                        app.open_release(source, window, cx);
                    } else {
                        app.open_addon_details(row_key.clone(), cx);
                    }
                }),
            )
        };
        div()
            .id(SharedString::from(format!("addon:{folder}")))
            .flex_1()
            .min_w_0()
            .min_h(px(if compact { 112. } else { 120. }))
            .py(px(18.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_t_1()
            .border_color(rgb(palette.text))
            .hover(move |style| style.bg(palette.hover))
            .when(self.selected.as_deref() == Some(&folder), |row| {
                row.bg(palette.hover)
            })
            .child(keyboard_button(
                div()
                    .id(SharedString::from(format!("addon-details:{folder}")))
                    .flex()
                    .items_center()
                    .gap(px(if compact { 13. } else { 17. }))
                    .min_w_0()
                    .flex_1()
                    .cursor_pointer()
                    .child(artwork)
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_size(px(20.))
                            .line_height(px(23.))
                            .font_weight(gpui::FontWeight(730.))
                            .child(typography::wrapped(addon.name.clone(), 20., 1.15, -0.7)),
                    ),
                palette,
                cx.listener(move |app, _: &(), _, cx| {
                    app.open_addon_details(folder.clone(), cx);
                }),
            ))
            .child(
                div()
                    .w(px(if compact { 88. } else { 105. }))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(26.))
                            .gap_1()
                            .child(icon(
                                match addon.source {
                                    "GitHub" => "github",
                                    "Astral" => "drive",
                                    _ => "globe",
                                },
                                if addon.source == "GitHub" { 13. } else { 12. },
                                if addon.source == "GitHub" {
                                    if self.preferences.light_theme {
                                        0x000000
                                    } else {
                                        0xffffff
                                    }
                                } else {
                                    palette.muted
                                },
                            ))
                            .child(palette.label(addon.source).text_size(px(10.))),
                    )
                    .child(
                        div()
                            .h(px(27.))
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .child(div().truncate().child(version.to_owned())),
                    )
                    .child(
                        div()
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(10.))
                            .text_color(rgb(status_color))
                            .when_some(status_icon, |label, glyph| {
                                label.child(icon(glyph, 12., status_color))
                            })
                            .when(ready, |label| {
                                label.child(div().size(px(5.)).rounded_full().bg(rgb(0xd6b786)))
                            })
                            .child(status),
                    ),
            )
            .child(action)
    }

    fn discovery_card(
        &self,
        source: Option<&Definition>,
        project: Option<&discovery::Project>,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let name = source
            .map(|source| source.name.as_str())
            .or_else(|| project.map(|p| p.name.as_str()))
            .unwrap_or_default();
        let description = source
            .and_then(|source| source.description.as_deref())
            .or_else(|| project.and_then(|p| p.description.as_deref()));
        let repo = source
            .and_then(|source| source.repo.as_deref())
            .or_else(|| project.map(|p| p.full_name.as_str()));
        let key = source
            .map(|source| source.id.as_str())
            .or(repo)
            .unwrap_or(name);
        let bundled = source.is_some_and(|source| source.bundled == Some(true));
        let installed = source.and_then(|source| {
            self.addons
                .iter()
                .find(|addon| addon.folder == source.folder)
        });
        let artwork = installed
            .and_then(|addon| addon.artwork.clone())
            .or_else(|| bundled.then(|| self.brand_icon.clone()));
        let busy = self.discovery_search_busy()
            || self.addon_batch_running()
            || self.scanning
            || self.daily_running
            || self.account_work_in_progress()
            || self.settings_write_in_progress();
        let mut actions = div()
            .w_full()
            .mt_auto()
            .pt(px(24.))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(14.));
        if let Some(addon) = installed {
            let name = addon.name.clone();
            actions = actions.child(keyboard_button(
                palette
                    .text_button(
                        SharedString::from(format!("discover-installed:{key}")),
                        "In your library",
                    )
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("check", 13., palette.muted)),
                palette,
                cx.listener(move |app, _: &(), window, cx| {
                    app.navigate(Page::Library, window, cx);
                    app.filter = Filter::All;
                    app.set_query(&name, cx);
                }),
            ));
        }
        if let Some(repo) = repo {
            let repo = repo.to_owned();
            let button = if installed.is_some() {
                palette.text_button(
                    SharedString::from(format!("discover-release:{key}")),
                    "Review release",
                )
            } else {
                palette
                    .button(
                        SharedString::from(format!("discover-release:{key}")),
                        "Review release",
                    )
                    .child(icon("arrow-right", 13., palette.muted))
            };
            actions = actions.child(if busy {
                button.opacity(0.45).cursor_default()
            } else {
                keyboard_button(
                    button,
                    palette,
                    cx.listener(move |app, _: &(), window, cx| {
                        app.review_project(repo.clone(), window, cx);
                    }),
                )
            });
        } else if let Some(source) = source.filter(|_| installed.is_none()) {
            if bundled {
                let id = source.id.clone();
                let button = palette.button(
                    SharedString::from(format!("discover-install:{id}")),
                    "Install",
                );
                actions = actions.child(
                    if busy
                        || self.profile_game_running()
                        || self.root(self.preferences.client).is_none()
                    {
                        button.opacity(0.45).cursor_default()
                    } else {
                        keyboard_button(
                            button,
                            palette,
                            cx.listener(move |app, _: &(), _, cx| {
                                app.install_missing_catalog_addon(&id, cx);
                            }),
                        )
                    },
                );
            } else if let Some(url) = source.source_url.clone() {
                let button = palette
                    .text_button(
                        SharedString::from(format!("discover-source:{key}")),
                        "Download page",
                    )
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("external", 13., palette.muted));
                actions = actions.child(if busy {
                    button.opacity(0.45).cursor_default()
                } else {
                    keyboard_button(button, palette, move |_, _, cx| cx.open_url(&url))
                });
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .min_h(px(215.))
            .flex()
            .flex_col()
            .items_start()
            .py(px(25.))
            .border_t_1()
            .border_color(rgb(palette.text))
            .child(
                self.artwork_tile(
                    artwork,
                    45.,
                    source
                        .and_then(|source| source.color.as_deref())
                        .unwrap_or("slate"),
                    palette,
                ),
            )
            .child(
                div()
                    .mt(px(17.))
                    .mb(px(9.))
                    .text_size(px(25.))
                    .line_height(px(30.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(typography::wrapped(name.to_owned(), 25., 1.2, -1.)),
            )
            .when_some(description, |card, description| {
                card.child(
                    palette
                        .label(description.to_owned())
                        .max_w(px(460.))
                        .text_size(px(12.))
                        .line_height(px(21.6)),
                )
            })
            .when_some(repo, |card, repo| {
                card.child(
                    div()
                        .mt(px(12.))
                        .mb(px(19.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(icon("github", 13., palette.text))
                        .child(palette.label(repo.to_owned()).text_size(px(10.))),
                )
            })
            .child(actions)
    }

    fn open_addon_details(&mut self, folder: String, cx: &mut Context<Self>) {
        self.selected = Some(folder);
        self.load_backups(cx);
        self.poll_profile_game_status(cx);
        cx.notify();
    }

    fn details(&self, addon: &Addon, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.preferences.light_theme);
        let definition = self.definition(addon);
        let folder = addon.folder.clone();
        let managed =
            definition.is_some_and(|source| source.repo.is_some() || source.bundled == Some(true));
        let update = managed && self.update_status(addon) == "Update available";
        let running = self.profile_game_running();
        let busy = self.checking
            || self.scanning
            || self.account_work_in_progress()
            || self.settings_write_in_progress()
            || self.daily_running;
        let can_import = definition.is_some_and(|source| source.source() != "Astral");
        let check = self.checks.get(&addon.folder);
        let item = |name: &'static str, value: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .text_size(px(11.))
                .line_height(px(13.2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(palette.muted))
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(gpui::FontWeight(450.))
                        .text_align(gpui::TextAlign::Right)
                        .child(value),
                )
        };
        let warning = |message: String| {
            palette
                .label(message)
                .text_size(px(11.))
                .line_height(px(13.2))
                .p(px(14.))
                .mb(px(20.))
                .rounded(px(8.))
                .bg(rgb(palette.surface))
                .border_1()
                .border_color(palette.line)
                .text_color(rgb(0xd18b35))
        };
        div()
            .id("details-drawer")
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(410.))
            .max_w(gpui::relative(0.95))
            .flex()
            .flex_col()
            .p(px(35.))
            .bg(rgb(palette.panel))
            .border_l_1()
            .border_color(palette.line)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::rgba(0x00000044).into(),
                offset: gpui::point(px(0.), px(20.)),
                blur_radius: px(35.),
                spread_radius: px(0.),
            }])
            .overflow_y_scroll()
            .occlude()
            .child(keyboard_button(
                palette
                    .icon_button_sized("close-details", "close", 20.)
                    .absolute()
                    .right(px(19.))
                    .top(px(19.)),
                palette,
                cx.listener(|app, _: &(), _, cx| {
                    app.selected = None;
                    cx.notify();
                }),
            ))
            .child(self.addon_icon(addon, 70.).rounded(px(5.)))
            .child(
                div()
                    .mt(px(28.))
                    .text_size(px(34.))
                    .line_height(px(40.8))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(typography::wrapped(addon.name.clone(), 34., 1.2, -1.5)),
            )
            .child(
                palette
                    .label(
                        definition
                            .and_then(|source| source.description.clone())
                            .unwrap_or_default(),
                    )
                    .mt(px(14.))
                    .mb(px(25.))
                    .text_size(px(12.))
                    .line_height(px(22.2)),
            )
            .child(
                div()
                    .mb(px(24.))
                    .border_y_1()
                    .border_color(palette.line)
                    .py(px(21.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(item("Installed version", addon.version.clone()))
                    .child(item("Source", addon.source.into()))
                    .when_some(
                        check.and_then(|result| result.as_ref().ok()),
                        |details, release| {
                            details.child(item("Latest release", release.tag.clone()))
                        },
                    ),
            )
            .when_some(
                check.and_then(|result| result.as_ref().err()),
                |drawer, error| drawer.child(warning(error.clone())),
            )
            .when_some(
                self.activity
                    .addon_error(addon, definition.map(|source| source.id.as_str())),
                |drawer, error| drawer.child(warning(format!("Last attempt failed: {error}"))),
            )
            .when(managed, |drawer| {
                let included = !self.excluded_from_daily(&folder);
                let toggle = div()
                    .id("addon-auto-update")
                    .flex()
                    .items_center()
                    .justify_between()
                    .my(px(18.))
                    .text_size(px(12.))
                    .child("Include in daily updates")
                    .child(palette.checkbox_mark(Some(included), 14.));
                let toggle_folder = folder.clone();
                let drawer = drawer.child(if busy {
                    toggle.opacity(0.55)
                } else {
                    keyboard_button(
                        toggle.cursor_pointer(),
                        palette,
                        cx.listener(move |app, _: &(), _, cx| {
                            app.toggle_daily_exclusion(&toggle_folder, cx);
                        }),
                    )
                });
                let action = palette
                    .primary_button(
                        "addon-release-action",
                        if update {
                            "Update addon"
                        } else {
                            "Check release"
                        },
                    )
                    .w_full()
                    .mb(px(10.))
                    .child(icon("download", 15., 0xffffff));
                let id = definition.map(|source| source.id.clone());
                drawer.child(if busy || (running && update) {
                    action.opacity(0.45).cursor_default()
                } else {
                    keyboard_button(
                        action,
                        palette,
                        cx.listener(move |app, _: &(), window, cx| {
                            if update && let Some(id) = &id {
                                app.open_release(id, window, cx);
                            } else {
                                app.check_updates(None, cx);
                            }
                        }),
                    )
                })
            })
            .when_some(definition.filter(|_| can_import), |drawer, source| {
                let id = source.id.clone();
                let action = palette
                    .button("addon-choose-zip", "Update from ZIP")
                    .w_full()
                    .mb(px(10.))
                    .child(icon("folder", 15., palette.muted));
                drawer.child(if busy || running {
                    action.opacity(0.45).cursor_default()
                } else {
                    keyboard_button(
                        action,
                        palette,
                        cx.listener(move |app, _: &(), window, cx| {
                            app.choose_addon_zip(&id, window, cx);
                        }),
                    )
                })
            })
            .when_some(self.activity.addon_backup(addon), |drawer, backup| {
                let path = backup.path.clone();
                let label = format!(
                    "Restore {}",
                    backup
                        .transaction
                        .previous_version
                        .as_deref()
                        .unwrap_or("previous version")
                );
                let action = palette
                    .button("addon-restore", label)
                    .w_full()
                    .mb(px(10.))
                    .child(icon("history", 15., palette.muted));
                drawer.child(if busy || running {
                    action.opacity(0.45).cursor_default()
                } else {
                    keyboard_button(
                        action,
                        palette,
                        cx.listener(move |app, _: &(), _, cx| {
                            app.activity.confirm_addon_restore(path.clone());
                            cx.notify();
                        }),
                    )
                })
            })
            .when_some(
                definition
                    .filter(|source| source.bundled != Some(true))
                    .and_then(|source| {
                        let url = source.source_url.clone().or_else(|| {
                            source
                                .repo
                                .as_ref()
                                .map(|repo| format!("https://github.com/{repo}"))
                        })?;
                        Some((
                            url,
                            if source.repo.is_some() {
                                "View on GitHub"
                            } else {
                                "Download page"
                            },
                        ))
                    }),
                |drawer, (url, label)| {
                    drawer.child(keyboard_button(
                        palette
                            .button("addon-source", label)
                            .w_full()
                            .mb(px(10.))
                            .child(icon("external", 14., palette.muted)),
                        palette,
                        move |_, _, cx| cx.open_url(&url),
                    ))
                },
            )
            .when(addon.source == "Manual", |drawer| {
                drawer.child(palette.label(if can_import {
                    "Download the addon ZIP, then choose Update from ZIP."
                } else {
                    "Update this addon manually. Changes made outside Astral are not backed up."
                }).mt(px(13.)).mb(px(28.)).text_size(px(10.)).line_height(px(18.)))
            })
            .when(running && (update || can_import), |drawer| {
                drawer.child(warning("Close WoW before updating.".into()))
            })
            .when_some(
                check
                    .and_then(|result| result.as_ref().ok())
                    .filter(|release| !release.notes.is_empty()),
                |drawer, release| {
                    drawer
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("Release notes"),
                        )
                        .child(
                            palette
                                .label(release.notes.clone())
                                .mt(px(11.))
                                .mb(px(11.))
                                .bg(rgb(palette.surface))
                                .text_size(px(11.))
                                .line_height(px(19.8)),
                        )
                },
            )
    }
}

impl Render for Astral {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        application_menu::refresh_window_menu(window);
        self.validate_account_profile_picker(window);
        if self.local_profile_review_open() || self.cloud_profile_review_open() {
            self.profile_review_focus
                .enter(self.profile_review_has_files(), window, cx);
        } else {
            self.profile_review_focus.leave(window, &self.root_focus);
        }
        self.update_cloud_manager_focus(window, cx);
        self.update_sync_dialog_focus(window, cx);
        self.update_addon_restore_focus(window, cx);
        let palette = Palette::new(self.preferences.light_theme);
        let client = self.preferences.client;
        let library = self.page == Page::Library;
        let settings = self.page == Page::Settings;
        let backups = self.page == Page::Backups;
        let width = window.viewport_size().width;
        let compact = width <= px(1050.);
        let inset = if compact { 26. } else { 38. };
        let action_gap = if compact { 10. } else { 15. };
        let account_name = self.account_name();
        let heading_size = (f32::from(width) * 0.045).clamp(36., 58.);
        let two_columns = width > px(780.);
        let gutter = if self.main_overflows { 9. } else { 0. };
        let main_inset = inset + ((f32::from(width) - gutter - 1640.) / 2.).max(0.);
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |app, cx| {
                let overflowing = app.main_scroll.max_offset().height > px(0.);
                if app.main_overflows != overflowing {
                    app.main_overflows = overflowing;
                    cx.notify();
                }
            });
        });
        let update_count = self
            .addons
            .iter()
            .filter(|addon| self.update_status(addon) == "Update available")
            .count();
        let top = div()
            .h(px(78.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(inset + if cfg!(target_os = "macos") { 65. } else { 0. }))
            .pr(px(inset))
            .border_b_1()
            .border_color(palette.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_7()
                    .child(
                        div()
                            .id("astral-brand")
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .mr(px(if compact { 0. } else { 14. }))
                            .cursor_pointer()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.page = Page::Library;
                                app.menu = None;
                                app.selected = None;
                                app.set_query("", cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .relative()
                                    .size(px(35.))
                                    .child(
                                        div()
                                            .absolute()
                                            .right_0()
                                            .top_0()
                                            .size(px(35.))
                                            .rounded_full()
                                            .bg(rgb(palette.accent)),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left(px(-4.))
                                            .top(px(2.))
                                            .size(px(30.))
                                            .rounded_full()
                                            .bg(rgb(palette.background)),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left(px(-8.))
                                            .top(px(2.))
                                            .size(px(30.))
                                            .rounded_full()
                                            .bg(rgb(palette.text)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(27.))
                                    .font_weight(gpui::FontWeight(760.))
                                    .child(typography::tracked("Astral", 27., 1.2, -1.5)),
                            ),
                    )
                    .children(
                        [
                            (Page::Library, "library", "Library"),
                            (Page::Discover, "discover", "Discover"),
                        ]
                        .map(|(page, id, name)| {
                            keyboard_button(
                                palette
                                    .tab(id, name, self.page == page)
                                    .h(px(78.))
                                    .py_0()
                                    .flex()
                                    .items_center()
                                    .border_b(px(3.))
                                    .when(self.page == page, |tab| {
                                        tab.font_weight(gpui::FontWeight::SEMIBOLD)
                                    }),
                                palette,
                                cx.listener(move |app, _: &(), window, cx| {
                                    app.navigate(page, window, cx)
                                }),
                            )
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(action_gap))
                    .child(
                        keyboard_button(
                            palette
                                .button("client-menu", "")
                                .px(px(10.))
                                .font_weight(gpui::FontWeight::NORMAL)
                                .h(px(40.))
                                .w(px(136.))
                                .justify_between()
                                .child(
                                    gpui::img(self.client_icons[client.index()].clone())
                                        .size(px(24.)),
                                )
                                .child(client.label())
                                .child(icon("chevron", 14., palette.muted)),
                            palette,
                            cx.listener(|app, _: &(), window, cx| {
                                app.toggle_menu(ToolbarMenu::Client, window, cx);
                            }),
                        )
                        .on_key_down(cx.listener(
                            |app, event: &gpui::KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "up" | "down")
                                    && app.menu != Some(ToolbarMenu::Client)
                                {
                                    app.toggle_menu(ToolbarMenu::Client, window, cx);
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    )
                    .child(keyboard_button(
                        palette.icon_button("quick-find", "search"),
                        palette,
                        cx.listener(|app, _: &(), window, cx| {
                            app.toggle_quick_find(window, cx);
                        }),
                    ))
                    .child(keyboard_button(
                        palette.icon_button(
                            "appearance",
                            if self.preferences.light_theme {
                                "moon"
                            } else {
                                "sun"
                            },
                        ),
                        palette,
                        cx.listener(|app, _: &(), window, cx| {
                            app.preferences.light_theme = !app.preferences.light_theme;
                            app.save_preferences();
                            if app.menu.is_some() {
                                app.close_toolbar_menu(window, cx);
                            } else {
                                cx.notify();
                            }
                        }),
                    ))
                    .child(
                        keyboard_button(
                            palette.icon_button("tools-menu", "more"),
                            palette,
                            cx.listener(|app, _: &(), window, cx| {
                                app.toggle_menu(ToolbarMenu::Tools, window, cx)
                            }),
                        )
                        .on_key_down(cx.listener(
                            |app, event: &gpui::KeyDownEvent, window, cx| {
                                if app.menu == Some(ToolbarMenu::Tools)
                                    && event.keystroke.key == "tab"
                                    && !event.keystroke.modifiers.shift
                                {
                                    app.tools_first_focus.focus(window);
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    )
                    .child(
                        keyboard_button(
                            div()
                                .id("account-menu")
                                .flex()
                                .items_center()
                                .gap(px(9.))
                                .cursor_pointer()
                                .child(palette.profile_avatar(account_name, false))
                                .child(icon("chevron", 13., palette.muted)),
                            palette,
                            cx.listener(|app, _: &(), window, cx| {
                                app.toggle_menu(ToolbarMenu::Account, window, cx);
                                if app.menu == Some(ToolbarMenu::Account) {
                                    app.poll_profile_game_status(cx);
                                    app.open_account(cx);
                                }
                            }),
                        )
                        .on_key_down(cx.listener(
                            |app, event: &gpui::KeyDownEvent, window, cx| {
                                if app.menu == Some(ToolbarMenu::Account)
                                    && event.keystroke.key == "tab"
                                    && !event.keystroke.modifiers.shift
                                {
                                    app.account_first_focus.focus(window);
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    ),
            );

        let menu = self.menu.map(|kind| {
            let mut panel = div()
                .id("toolbar-menu")
                .absolute()
                .when(kind == ToolbarMenu::Sources, |panel| {
                    panel.top(px(350.)).left(px(47.)).w(px(158.))
                })
                .when(kind == ToolbarMenu::Client, |panel| {
                    panel
                        .top(px(76.))
                        .right(px(inset + 147. + 4. * action_gap))
                        .w(px(186.))
                })
                .when(kind == ToolbarMenu::Tools, |panel| {
                    panel
                        .top(px(72.))
                        .right(px(inset + 57. + action_gap))
                        .w(px(260.))
                })
                .when(kind == ToolbarMenu::Account, |panel| {
                    panel.top(px(74.5)).right(px(inset)).w(px(300.))
                })
                .p(px(9.))
                .flex()
                .flex_col()
                .rounded(px(8.))
                .bg(rgb(palette.panel))
                .border_1()
                .border_color(palette.line)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::rgba(0x00000033).into(),
                    offset: gpui::point(px(0.), px(15.)),
                    blur_radius: px(25.),
                    spread_radius: px(0.),
                }])
                .occlude();
            match kind {
                ToolbarMenu::Client => {
                    panel = panel.on_key_down(cx.listener(
                        |app, event: &gpui::KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "up" | "down" => {
                                    let other = Client::ALL.into_iter().find(|candidate| {
                                        !app.client_menu_focus[candidate.index()].is_focused(window)
                                            && (*candidate == app.preferences.client
                                                || app.root(*candidate).is_some())
                                    });
                                    if let Some(other) = other {
                                        app.client_menu_focus[other.index()].focus(window);
                                    }
                                }
                                "tab" => {
                                    app.close_toolbar_menu(window, cx);
                                    if event.keystroke.modifiers.shift {
                                        window.focus_prev();
                                    } else {
                                        window.focus_next();
                                    }
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        },
                    ));
                    for (index, target) in Client::ALL.into_iter().enumerate() {
                        if target != client && self.root(target).is_none() {
                            continue;
                        }
                        panel = panel.child(keyboard_button(
                            div()
                                .id(("choose-client", index))
                                .track_focus(&self.client_menu_focus[index])
                                .h(px(40.))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .rounded_md()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .hover(move |style| style.bg(palette.hover))
                                .child(gpui::img(self.client_icons[index].clone()).size(px(22.)))
                                .child(div().flex_1().child(target.label()))
                                .when(target == client, |row| {
                                    row.child(icon("check", 16., palette.accent))
                                }),
                            palette,
                            cx.listener(move |app, _: &(), window, cx| {
                                app.close_toolbar_menu(window, cx);
                                if !app.addon_batch_running()
                                    && !app.settings_write_in_progress()
                                    && app.preferences.client != target
                                {
                                    app.change_client(target, cx);
                                }
                                cx.notify();
                            }),
                        ));
                    }
                }
                ToolbarMenu::Sources => {
                    for (value, label, glyph) in [
                        (None, "All sources", "globe"),
                        (Some("GitHub"), "GitHub", "github"),
                        (Some("Astral"), "Astral", "drive"),
                        (Some("Manual"), "Manual sources", "globe"),
                    ] {
                        panel = panel.child(keyboard_button(
                            palette.menu_row(label, glyph, label),
                            palette,
                            cx.listener(move |app, _: &(), window, cx| {
                                app.discovery_source = value;
                                app.close_toolbar_menu(window, cx);
                                cx.notify();
                            }),
                        ));
                    }
                }
                ToolbarMenu::Tools => {
                    for (id, glyph, label, page) in [
                        ("backups", "history", "Activity & backups", Page::Backups),
                        ("sources-page", "globe", "Sources", Page::Sources),
                        ("settings", "gear", "Settings", Page::Settings),
                    ] {
                        panel = panel.child(
                            keyboard_button(
                                palette
                                    .menu_row(id, glyph, label)
                                    .when(page == Page::Backups, |row| {
                                        row.track_focus(&self.tools_first_focus)
                                    }),
                                palette,
                                cx.listener(move |app, _: &(), window, cx| {
                                    app.navigate(page, window, cx)
                                }),
                            )
                            .when(page == Page::Backups, |row| {
                                row.on_key_down(cx.listener(
                                    |app, event: &gpui::KeyDownEvent, window, cx| {
                                        app.return_to_menu_trigger_on_shift_tab(event, window, cx);
                                    },
                                ))
                            }),
                        );
                    }
                    panel = panel.child(keyboard_button(
                        palette.menu_row("choose", "folder", "Choose WoW folder"),
                        palette,
                        cx.listener(move |app, _: &(), window, cx| {
                            app.close_toolbar_menu(window, cx);
                            app.choose_root(client, cx);
                        }),
                    ));
                }
                ToolbarMenu::Account => {
                    let connected = account_name.is_some();
                    panel = panel
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px(px(10.))
                                .pt(px(10.))
                                .pb(px(19.))
                                .mb(px(5.))
                                .border_b_1()
                                .border_color(palette.line)
                                .child(palette.profile_avatar(account_name, true))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .line_height(px(15.6))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(
                                            account_name
                                                .unwrap_or("Your Astral account")
                                                .to_owned(),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(6.))
                                                .mt(px(6.))
                                                .text_size(px(10.))
                                                .line_height(px(12.))
                                                .text_color(rgb(palette.muted))
                                                .child(div().size(px(6.)).rounded_full().bg(rgb(
                                                    if connected {
                                                        0x19c587
                                                    } else {
                                                        palette.muted
                                                    },
                                                )))
                                                .child(if connected {
                                                    "Battle.net connected"
                                                } else {
                                                    "Not connected"
                                                }),
                                        ),
                                ),
                        )
                        .child(
                            keyboard_button(
                                palette
                                    .menu_row("profiles-and-sync", "cloud", "Profiles & sync")
                                    .h(px(43.))
                                    .track_focus(&self.account_first_focus)
                                    .child(div().ml_auto().child(icon(
                                        "chevron-right",
                                        14.,
                                        palette.text,
                                    ))),
                                palette,
                                cx.listener(|app, _: &(), window, cx| {
                                    app.navigate(Page::Account, window, cx)
                                }),
                            )
                            .on_key_down(cx.listener(
                                |app, event: &gpui::KeyDownEvent, window, cx| {
                                    app.return_to_menu_trigger_on_shift_tab(event, window, cx);
                                },
                            )),
                        )
                        .when(connected, |panel| {
                            panel.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .px(px(10.))
                                    .py(px(13.))
                                    .child(icon("refresh", 16., palette.text))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .line_height(px(14.4))
                                            .child("Last sync")
                                            .child(
                                                palette
                                                    .label(self.last_sync_label())
                                                    .mt(px(5.))
                                                    .text_size(px(10.))
                                                    .line_height(px(12.)),
                                            ),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_0()
                                .px(px(10.))
                                .py(px(12.))
                                .text_size(px(11.))
                                .line_height(px(13.2))
                                .text_color(rgb(palette.muted))
                                .child(icon("monitor", 15., palette.muted).mr(px(9.)))
                                .child(div().flex_1().child(if cfg!(target_os = "windows") {
                                    "This Windows PC"
                                } else {
                                    "This Mac"
                                }))
                                .child(div().ml_auto().text_size(px(10.)).child(
                                    if self.profile_game_running() {
                                        "WoW running"
                                    } else {
                                        "WoW closed"
                                    },
                                )),
                        )
                        .when(connected, |panel| {
                            let signout = palette
                                .menu_row("account-menu-signout", "logout", "Sign out")
                                .h(px(43.))
                                .mt(px(5.))
                                .rounded_none()
                                .border_t_1()
                                .border_color(palette.line);
                            panel.child(if self.account_work_in_progress() {
                                signout.opacity(0.45).cursor_default()
                            } else {
                                keyboard_button(
                                    signout,
                                    palette,
                                    cx.listener(|app, _: &(), window, cx| {
                                        app.close_toolbar_menu(window, cx);
                                        app.sign_out(cx);
                                    }),
                                )
                            })
                        });
                }
            }
            panel
        });

        let can_check = !self.checking
            && !self.scanning
            && self.addons.iter().any(|addon| {
                self.definition(addon)
                    .is_some_and(|source| source.repo.is_some() || source.bundled == Some(true))
            });
        let header = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(heading_size))
                            .line_height(px(heading_size * 1.06))
                            .font_weight(gpui::FontWeight(780.))
                            .child(typography::tracked(
                                self.page.title(),
                                heading_size,
                                1.06,
                                -heading_size * 0.05,
                            )),
                    )
                    .when(library, |heading| {
                        heading.child(
                            palette
                                .label(format!("{} addons", self.addons.len()))
                                .mt(px(8.))
                                .text_size(px(13.))
                                .line_height(px(20.8)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .flex_shrink_0()
                    .when(library, |actions| {
                        actions
                            .child(
                                palette
                                    .button(
                                        "updates",
                                        if self.checking {
                                            "Checking…"
                                        } else {
                                            "Check for updates"
                                        },
                                    )
                                    .flex_row_reverse()
                                    .child(icon("refresh", 15., palette.muted))
                                    .when(!can_check, |button| {
                                        button.opacity(0.45).cursor_default()
                                    })
                                    .when(can_check, |button| {
                                        keyboard_button(
                                            button,
                                            palette,
                                            cx.listener(|app, _: &(), _, cx| {
                                                app.check_updates(None, cx)
                                            }),
                                        )
                                    }),
                            )
                            .when(self.batch_update_count() > 0, |actions| {
                                let blocked =
                                    !self.has_batch_updates() || self.profile_game_running();
                                let button = palette
                                    .primary_button(
                                        "update-all",
                                        format!("Update all ({})", self.batch_update_count()),
                                    )
                                    .flex_row_reverse()
                                    .child(icon("download", 15., 0xffffff));
                                actions.child(if blocked {
                                    button.opacity(0.45).cursor_default()
                                } else {
                                    keyboard_button(
                                        button,
                                        palette,
                                        cx.listener(|app, _: &(), _, cx| app.update_all(cx)),
                                    )
                                })
                            })
                            .when(
                                self.batch_update_count() > 0 && self.profile_game_running(),
                                |actions| {
                                    actions.child(
                                        palette.label("Close WoW to update.").text_size(px(11.)),
                                    )
                                },
                            )
                    }),
            );

        let mut list = div().id("collection").flex_shrink_0().flex().flex_col();
        let mut content = div()
            .id("main-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.main_scroll)
            .flex()
            .flex_col()
            .gap(px(if library { 22. } else { 28. }))
            .px(px(main_inset))
            .pt(px(if compact { 35. } else { 42. }))
            .pb(px(30.))
            .child(header.when(library, |header| header.mb(px(6.))))
            .when_some(
                (!settings)
                    .then(|| self.app_update_notice(palette, cx))
                    .flatten(),
                |content, notice| content.child(notice),
            )
            .when_some(self.preferences_error.as_ref(), |content, error| {
                content.child(palette.label(error.clone()).text_color(rgb(palette.error)))
            })
            .when(!settings, |content| {
                content.when_some(self.folder_error.as_ref(), |content, (game, error)| {
                    content.child(
                        palette
                            .label(format!("{}: {error}", game.label()))
                            .text_color(rgb(palette.error)),
                    )
                })
            });
        if settings {
            list = list.child(self.settings(palette, compact, cx));
        } else if self.page == Page::Account {
            list = list.child(self.account_page(palette, compact, window, cx));
        } else if backups {
            list = list.child(self.backups_page(palette, cx));
        } else if self.page == Page::Sources {
            list = list.child(self.sources_page(palette, compact));
        } else if library {
            content = content.when_some(self.batch_result_panel(palette, cx), |content, result| {
                content.child(result)
            });
            let query = self.query.to_lowercase();
            let matches: Vec<_> = self
                .addons
                .iter()
                .filter(|addon| addon.matches_library_query(&query))
                .filter(|addon| match self.filter {
                    Filter::All => true,
                    Filter::Updates => self.update_status(addon) == "Update available",
                    Filter::Manual => {
                        addon.source == "Manual"
                            || self
                                .definition(addon)
                                .is_some_and(|source| source.local == Some(true))
                    }
                })
                .collect();
            content = content.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div().flex().gap_5().children(
                            [
                                (Filter::All, "all", "All addons"),
                                (Filter::Updates, "filter-updates", "Updates"),
                                (Filter::Manual, "manual", "Manual"),
                            ]
                            .map(|(filter, id, name)| {
                                keyboard_button(
                                    palette
                                        .tab(id, name, self.filter == filter)
                                        .text_size(px(12.))
                                        .line_height(px(14.4))
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .when(
                                            filter == Filter::Updates && update_count > 0,
                                            |tab| {
                                                tab.child(
                                                    div()
                                                        .text_color(rgb(palette.accent))
                                                        .child(update_count.to_string()),
                                                )
                                            },
                                        ),
                                    palette,
                                    cx.listener(move |app, _: &(), _, cx| {
                                        app.filter = filter;
                                        app.selected = None;
                                        cx.notify();
                                    }),
                                )
                            }),
                        ),
                    )
                    .child(self.search_box(window, cx)),
            );
            if let Some(error) = &self.error {
                list = list.child(
                    div()
                        .py_2()
                        .text_color(rgb(palette.error))
                        .child(error.clone()),
                );
            }
            if self.scanning {
                list = list.child(palette.label("Scanning addons…"));
            } else if matches.is_empty() {
                let has_addons = !self.addons.is_empty();
                let failed = self.error.is_some();
                let label = if failed {
                    "Retry"
                } else if has_addons {
                    "Clear filters"
                } else {
                    "Find World of Warcraft"
                };
                let action = palette.button("library-empty-action", label).mt(px(20.));
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .py(px(55.))
                        .px(px(20.))
                        .child(icon("package", 28., palette.muted))
                        .child(
                            div()
                                .mt(px(18.))
                                .mb(px(10.))
                                .text_size(px(16.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(if failed {
                                    "Couldn’t load addons"
                                } else if has_addons {
                                    "No addons match that search"
                                } else {
                                    "Choose your WoW folder"
                                }),
                        )
                        .child(if !has_addons && !failed && self.fixture {
                            action.opacity(0.45).cursor_default()
                        } else {
                            keyboard_button(
                                action,
                                palette,
                                cx.listener(move |app, _: &(), _, cx| {
                                    if failed {
                                        app.refresh(cx);
                                    } else if has_addons {
                                        app.filter = Filter::All;
                                        app.set_query("", cx);
                                        cx.notify();
                                    } else {
                                        app.choose_root(client, cx);
                                    }
                                }),
                            )
                        }),
                );
            }
            for row in matches.chunks(if two_columns { 2 } else { 1 }) {
                let mut line = div()
                    .flex()
                    .flex_shrink_0()
                    .gap(px(if compact { 25. } else { 36. }))
                    .children(row.iter().map(|addon| self.tile(addon, compact, cx)));
                if two_columns && row.len() == 1 {
                    line = line.child(div().flex_1());
                }
                list = list.child(line);
            }
        } else {
            if let Some(essentials) = self.forever_essentials(palette, cx) {
                list = list.child(essentials);
            }
            let query = self.query.to_lowercase();
            let known: Vec<_> = self
                .catalog
                .iter()
                .filter(|source| source.supports(client) && source.local != Some(true))
                .filter(|source| {
                    format!(
                        "{} {}",
                        source.name,
                        source.description.as_deref().unwrap_or_default()
                    )
                    .to_lowercase()
                    .contains(&query)
                })
                .collect();
            let known_repos: std::collections::HashSet<_> = known
                .iter()
                .filter_map(|source| source.repo.as_ref().map(|repo| repo.to_ascii_lowercase()))
                .collect();
            let mut results: Vec<_> = known
                .into_iter()
                .map(|source| (Some(source), None))
                .collect();
            results.extend(
                self.discovery_projects()
                    .iter()
                    .filter(|project| {
                        !known_repos.contains(&project.full_name.to_ascii_lowercase())
                    })
                    .map(|project| {
                        let source = self.catalog.iter().find(|source| {
                            source.supports(client)
                                && source.repo.as_ref().is_some_and(|repo| {
                                    repo.eq_ignore_ascii_case(&project.full_name)
                                })
                        });
                        (source, Some(project))
                    }),
            );
            results.retain(|(source, project)| {
                self.discovery_source.is_none_or(|filter| {
                    if let Some(source) = source {
                        source.source() == filter
                    } else {
                        project.is_some() && filter == "GitHub"
                    }
                })
            });
            let searching = self.discovery_search_busy();
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .pt_1()
                            .pb(px(25.))
                            .border_b_1()
                            .border_color(palette.line)
                            .child(self.search_box(window, cx))
                            .child(if searching || self.addon_batch_running() {
                                palette
                                    .primary_button("search-catalog", "Search")
                                    .h(px(41.))
                                    .opacity(0.45)
                                    .cursor_default()
                            } else {
                                keyboard_button(
                                    palette
                                        .primary_button("search-catalog", "Search")
                                        .h(px(41.))
                                        .flex_row_reverse()
                                        .child(icon("search", 15., 0xffffff)),
                                    palette,
                                    cx.listener(|app, _: &(), _, cx| app.search_github(cx)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(9.))
                            .child(keyboard_button(
                                palette
                                    .button(
                                        "discover-source",
                                        match self.discovery_source {
                                            Some("Manual") => "Manual sources",
                                            Some(source) => source,
                                            None => "All sources",
                                        },
                                    )
                                    .h(px(37.))
                                    .min_w(px(129.))
                                    .justify_between()
                                    .bg(rgb(palette.panel))
                                    .child(icon("chevron", 14., palette.muted)),
                                palette,
                                cx.listener(|app, _: &(), window, cx| {
                                    app.toggle_menu(ToolbarMenu::Sources, window, cx);
                                }),
                            ))
                            .child(
                                palette
                                    .label(if searching {
                                        "Searching…".into()
                                    } else {
                                        format!("{} results", results.len())
                                    })
                                    .text_size(px(12.)),
                            ),
                    ),
            );
            if let Some(error) = self.discovery_search_error() {
                list = list.child(
                    palette
                        .label(error.to_owned())
                        .text_size(px(11.))
                        .line_height(px(13.2))
                        .p(px(14.))
                        .mb(px(20.))
                        .rounded(px(8.))
                        .bg(rgb(palette.surface))
                        .border_1()
                        .border_color(palette.line)
                        .text_color(rgb(0xd18b35)),
                );
            }
            if results.is_empty() && !searching {
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .py(px(55.))
                        .child(icon("package", 28., palette.muted))
                        .child(
                            div()
                                .mt(px(18.))
                                .text_size(px(16.))
                                .child("No matching addons"),
                        ),
                );
            }
            for row in results.chunks(if two_columns { 2 } else { 1 }) {
                let mut line =
                    div()
                        .flex()
                        .flex_shrink_0()
                        .gap(px(36.))
                        .children(row.iter().map(|&(source, project)| {
                            self.discovery_card(source, project, palette, cx)
                        }));
                if two_columns && row.len() == 1 {
                    line = line.child(div().flex_1());
                }
                list = list.child(line);
            }
        }

        application_menu::window_actions(div(), cx)
            .h_full()
            .w(width - px(gutter))
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(palette.background))
            .text_color(rgb(palette.text))
            .text_size(px(14.))
            .child(top)
            .child(
                content
                    .when_some(
                        self.install_receipt
                            .as_ref()
                            .filter(|receipt| receipt.client == client),
                        |content, receipt| content.child(self.install_notice(receipt, palette, cx)),
                    )
                    .child(list),
            )
            .track_focus(&self.root_focus)
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .on_mouse_move(
                cx.listener(|app, event, window, cx| app.drag_scrollbar(event, window, cx)),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|app, _, _, cx| app.release_scrollbar(cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|app, _, _, cx| app.release_scrollbar(cx)),
            )
            .on_action(
                cx.listener(|app, _: &FindAddon, window, cx| app.toggle_quick_find(window, cx)),
            )
            .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" && app.cloud_profile_manager_open() {
                    app.cycle_cloud_manager_focus(event.keystroke.modifiers.shift, window, cx);
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "tab" && app.quick_find.is_none() {
                    if app.dismiss_account_profile_picker(cx) {
                        app.profile_review_focus.picker.focus(window);
                    }
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                let Some(selected) = app.quick_find else {
                    return;
                };
                let row_focused = app.quick_find_rows.iter().any(|row| row.is_focused(window));
                let next = match event.keystroke.key.as_str() {
                    "up" | "tab"
                        if event.keystroke.key == "up" || event.keystroke.modifiers.shift =>
                    {
                        if row_focused {
                            selected.checked_sub(1).unwrap_or(QUICK_PAGES.len() - 1)
                        } else {
                            QUICK_PAGES.len() - 1
                        }
                    }
                    "down" | "tab" => {
                        if row_focused {
                            (selected + 1) % QUICK_PAGES.len()
                        } else {
                            0
                        }
                    }
                    _ => return,
                };
                app.quick_find = Some(next);
                app.quick_find_rows[next].focus(window);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &DismissOverlay, window, cx| {
                if app.dismiss_sync_dialog(cx) {
                    return;
                }
                if app.dismiss_quick_find(window, cx) {
                    return;
                }
                if app.profile_review_focus.open {
                    if !app.dismiss_local_profile_review(cx) {
                        app.dismiss_cloud_profile_review(cx);
                    }
                    return;
                }
                if app.menu.is_some() {
                    app.close_toolbar_menu(window, cx);
                    return;
                }
                if app.dismiss_local_account_picker(cx) {
                    return;
                }
                if app.dismiss_account_profile_picker(cx) {
                    app.profile_review_focus.picker.focus(window);
                    return;
                }
                if app.dismiss_account_confirmation(cx) {
                    app.root_focus.focus(window);
                    return;
                }
                if app.dismiss_profile_manager(window, cx) {
                    return;
                }
                if app.dismiss_project_review(window, cx) {
                    return;
                }
                if app.dismiss_removal(cx) {
                    return;
                }
                if app.activity.has_confirmation() {
                    if app.activity.dismiss_confirmation() {
                        cx.notify();
                    }
                    return;
                }
                if app
                    .release_review
                    .as_ref()
                    .is_some_and(review::Review::is_installing)
                {
                    return;
                }
                if app.release_review.take().is_some() || app.selected.take().is_some() {
                    cx.notify();
                }
            }))
            .when_some(self.main_scrollbar(window, cx), |app, scrollbar| {
                app.child(scrollbar)
            })
            .when_some(menu, |app, menu| {
                app.child(
                    div()
                        .id("menu-dismiss")
                        .absolute()
                        .top(px(78.))
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.close_toolbar_menu(window, cx);
                        })),
                )
                .child(menu)
            })
            .when_some(
                self.selected
                    .as_ref()
                    .and_then(|folder| self.addons.iter().find(|addon| &addon.folder == folder)),
                |app, addon| {
                    app.child(
                        div()
                            .id("drawer-backdrop")
                            .absolute()
                            .top(px(78.))
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .bg(gpui::rgba(0x00000044))
                            .occlude()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.selected = None;
                                cx.notify();
                            })),
                    )
                    .child(self.details(addon, cx))
                },
            )
            .when_some(self.quick_find, |app, selected| {
                app.child(self.quick_find_dialog(selected, palette, window, cx))
            })
            .when_some(self.release_review.as_ref(), |app, review| {
                app.child(self.release_dialog(review, palette, cx))
            })
            .when(self.activity.has_confirmation(), |app| {
                app.child(self.restore_dialog(palette, cx))
            })
            .when_some(self.local_profile_review(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(
                self.cloud_profile_review_dialog(palette, cx),
                |app, dialog| app.child(dialog),
            )
            .when_some(
                self.cloud_profile_manager_dialog(palette, window, cx),
                |app, dialog| app.child(dialog),
            )
            .when_some(self.removal.as_ref(), |app, removal| {
                app.child(self.removal_dialog(removal, palette, cx))
            })
            .when_some(
                self.project_review
                    .as_ref()
                    .filter(|review| review.is_visible()),
                |app, review| app.child(self.project_dialog(review, palette, cx)),
            )
            .when_some(self.sync_conflict_dialog(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(self.sync_missing_dialog(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(self.cloud_restore_dialog(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(self.cloud_device_dialog(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(self.cloud_delete_dialog(palette, cx), |app, dialog| {
                app.child(dialog)
            })
            .when_some(self.notification_view(palette, window, cx), |app, toast| {
                app.child(toast)
            })
            .when(self.account_profile_picker_open(), |app| {
                app.child(div().absolute().inset_0().occlude().on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|app, _, window, cx| {
                        app.dismiss_account_profile_picker(cx);
                        app.profile_review_focus.picker.focus(window);
                        cx.stop_propagation();
                    }),
                ))
            })
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mut fixture_client = None;
    let mut fixture_light = None;
    let mut fixture_dialog = None;
    let mut fixture_notice = None;
    let mut fixture_scan = false;
    let mut fixture_signed_in = false;
    let mut fixture_compact = false;
    let mut fixture_wide = false;
    let mut fixture_page = None;
    if args.first().is_some_and(|arg| arg == "--version") {
        println!("Astral Native {}", app_updates::CURRENT);
        return;
    }
    if args.first().is_some_and(|arg| arg == "--licenses") {
        print!("Lucide icons\n{}", include_str!("../assets/icons/LICENSE"));
        return;
    }
    let fixture = if args.first().is_some_and(|arg| arg == "--ui-fixture") {
        let directory = args.get(1).map(PathBuf::from);
        let fixture = directory.and_then(|directory| directory.canonicalize().ok());
        let valid = args.iter().skip(2).all(|flag| match flag.to_str() {
            Some("--retail") if fixture_client.is_none() => {
                fixture_client = Some(Client::Retail);
                true
            }
            Some("--forever") if fixture_client.is_none() => {
                fixture_client = Some(Client::Forever);
                true
            }
            Some(flag @ ("--light" | "--dark")) if fixture_light.is_none() => {
                fixture_light = Some(flag == "--light");
                true
            }
            Some(flag @ ("--sync-conflict" | "--sync-missing" | "--profile-review"))
                if fixture_dialog.is_none() =>
            {
                fixture_dialog = Some(flag.to_owned());
                true
            }
            Some(flag @ ("--notice" | "--notice-error")) if fixture_notice.is_none() => {
                fixture_notice = Some(flag == "--notice-error");
                true
            }
            Some("--compact") if !fixture_compact && !fixture_wide => {
                fixture_compact = true;
                true
            }
            Some("--wide") if !fixture_compact && !fixture_wide => {
                fixture_wide = true;
                true
            }
            Some("--signed-in") if !fixture_signed_in => {
                fixture_signed_in = true;
                true
            }
            Some("--scan") if args.len() == 3 => {
                fixture_scan = true;
                true
            }
            Some(flag) if fixture_page.is_none() => {
                fixture_page = match flag {
                    "--library" => Some(Page::Library),
                    "--discover" => Some(Page::Discover),
                    "--account" => Some(Page::Account),
                    "--activity" => Some(Page::Backups),
                    "--sources" => Some(Page::Sources),
                    "--settings" => Some(Page::Settings),
                    _ => None,
                };
                fixture_page.is_some()
            }
            _ => false,
        });
        if fixture.is_none() || !valid {
            eprintln!(
                "Usage: astral-native --ui-fixture <fixture-directory> [--scan | [--retail | --forever] [--light | --dark] [--compact | --wide] [--library | --discover | --account | --activity | --sources | --settings] [--sync-conflict | --sync-missing | --profile-review] [--signed-in] [--notice | --notice-error]]"
            );
            std::process::exit(2);
        }
        fixture
    } else {
        None
    };
    let roots = if let Some(directory) = &fixture {
        Client::ALL.map(|client| {
            let path = directory.join(if client == Client::Retail {
                "_retail_"
            } else {
                "_forever_"
            });
            let addons = path.join("Interface").join("AddOns");
            if addons.canonicalize().ok().as_ref() != Some(&addons) {
                return None;
            }
            library::validate_root(&path, client).ok().map(|()| path)
        })
    } else {
        Client::ALL.map(Client::detect)
    };
    if fixture.is_some()
        && (roots.iter().all(Option::is_none)
            || fixture_client.is_some_and(|client| roots[client.index()].is_none()))
    {
        eprintln!("The selected client is missing from the fixture.");
        std::process::exit(2);
    }
    let update_ack = args
        .first()
        .filter(|arg| *arg == "--astral-update-ack")
        .and_then(|_| args.get(1))
        .and_then(|arg| arg.to_str())
        .map(str::to_owned);
    let update_recovery = args
        .first()
        .filter(|arg| *arg == "--astral-update-recovery")
        .and_then(|_| args.get(1))
        .and_then(|arg| arg.to_str())
        .map(str::to_owned);
    if args.first().is_some_and(|arg| arg == "--daily-update") {
        if let Some(path) = preferences::path()
            && let Err(error) = daily::run_background(&path, roots)
        {
            eprintln!("Astral daily updates: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--check") {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let id = args
                .get(1)
                .and_then(|arg| arg.to_str())
                .ok_or("Usage: --check <catalog-id>")?;
            let catalog = library::catalog();
            let source = catalog
                .iter()
                .find(|source| source.id == id)
                .ok_or("Unknown catalog addon.")?;
            let client = if source.supports(Client::Retail) {
                Client::Retail
            } else {
                Client::Forever
            };
            let root = roots[client.index()]
                .as_ref()
                .ok_or("WoW folder not found.")?;
            let addons = library::scan(root, client, &catalog)?;
            let addon = addons
                .iter()
                .find(|addon| addon.folder == source.folder)
                .ok_or("Addon not installed.")?;
            let check = releases::check(&releases::client(), source, &addon.version)?;
            println!(
                "{}: installed {}, latest {} ({:?})",
                addon.name, addon.version, check.tag, check.comparison
            );
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--scan") || fixture_scan {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let catalog = library::catalog();
            for (client, root) in Client::ALL.into_iter().zip(roots) {
                if let Some(root) = root {
                    let addons = library::scan_library(&root, client, &catalog)?;
                    println!(
                        "{}",
                        serde_json::json!({ "client": client.label(), "addons": addons })
                    );
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let background = args.iter().any(|arg| arg == "--background");
    if background
        && !preferences::path()
            .as_deref()
            .and_then(|path| Preferences::load(path).ok())
            .is_some_and(|preferences| preferences.start_at_login)
    {
        return;
    }
    let instance = match if fixture.is_some() {
        Ok(Some(startup::Instance::default()))
    } else {
        startup::Instance::open(background)
    } {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            eprintln!("Couldn't check for another Astral window. {error}");
            if background {
                return;
            }
            startup::Instance::default()
        }
    };
    let application = Application::new().with_assets(icons::Icons);
    application.on_reopen(|cx| {
        for window in cx.windows() {
            if let Some(window) = window.downcast::<Astral>() {
                let _ = window.update(cx, |app, window, cx| tray::reopen(app, window, cx));
            }
        }
    });
    application.run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None,
        )]);
        cx.bind_keys([
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-k"
                } else {
                    "ctrl-k"
                },
                FindAddon,
                None,
            ),
            KeyBinding::new("escape", DismissOverlay, None),
        ]);
        application_menu::install(cx);
        let viewport = if fixture_compact {
            size(px(1000.), px(720.))
        } else if fixture_wide {
            size(px(1920.), px(1000.))
        } else {
            size(px(1250.), px(840.))
        };
        let bounds = Bounds::centered(None, viewport, cx);
        let handle = cx
            .open_window(
                WindowOptions {
                    show: !background,
                    focus: !background,
                    titlebar: Some(TitlebarOptions {
                        title: Some("Astral".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: Some(gpui::point(px(11.), px(11.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(980.), px(680.))),
                    ..Default::default()
                },
                |window, cx| {
                    if fixture.is_some() && window.viewport_size() != viewport {
                        window.resize(viewport);
                    }
                    if let Some(token) = update_ack {
                        window.on_next_frame(move |_, _| app_updates::acknowledge(&token));
                    }
                    cx.new(|cx| {
                        let mut app = Astral::new(
                            roots,
                            fixture.as_deref(),
                            fixture_client,
                            fixture_light.unwrap_or(false),
                            cx,
                        );
                        if let Some(page) = fixture_page.or_else(|| {
                            (fixture_dialog.as_deref() == Some("--profile-review"))
                                .then_some(Page::Account)
                        }) {
                            app.navigate(page, window, cx);
                        }
                        if fixture_signed_in {
                            app.open_signed_account_fixture();
                        }
                        if let Some(error) = fixture_notice {
                            app.open_notification_fixture(error);
                        }
                        if let Some(flag) = fixture_dialog.as_deref() {
                            app.open_sync_dialog_fixture(flag);
                        }
                        if let Some(reason) = update_recovery.as_deref()
                            && app.app_updates.show_recovery(reason)
                        {
                            app.navigate(Page::Settings, window, cx);
                        }
                        if !background {
                            app.root_focus.focus(window);
                        }
                        app
                    })
                },
            )
            .expect("open Astral window");
        if fixture.is_none()
            && let Err(error) = application_menu::install_window(handle, cx)
        {
            eprintln!("Couldn't create the Astral application menu. {error}");
        }
        if fixture.is_none()
            && let Err(error) = tray::install(handle, background, cx)
        {
            eprintln!("Couldn't create the Astral tray menu. {error}");
        }
        instance.watch(handle, cx);
        if !background {
            cx.activate(true);
        }
    });
}
