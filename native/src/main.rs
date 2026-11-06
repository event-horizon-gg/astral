#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod artwork;
mod icons;
mod library;
mod releases;

use gpui::{
    App, Application, Bounds, Context, KeyBinding, Menu, MenuItem, PathPromptOptions, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use library::{Addon, Client, Definition};
use std::{cmp::Ordering, collections::HashMap, path::PathBuf, sync::Arc};

gpui::actions!(astral, [Quit, FindAddon, DismissOverlay]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Discover,
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
}

struct Astral {
    client: Client,
    page: Page,
    filter: Filter,
    query: String,
    search_focus: gpui::FocusHandle,
    menu: Option<ToolbarMenu>,
    light_theme: bool,
    root_focus: gpui::FocusHandle,
    brand_icon: Arc<gpui::Image>,
    client_icons: [Arc<gpui::Image>; 2],
    roots: [Option<PathBuf>; 2],
    catalog: Arc<Vec<Definition>>,
    addons: Vec<Addon>,
    error: Option<String>,
    selected: Option<String>,
    scan_id: u64,
    scanning: bool,
    checking: bool,
    checks: HashMap<String, Result<releases::Check, String>>,
}

impl Astral {
    fn new(roots: [Option<PathBuf>; 2], cx: &mut Context<Self>) -> Self {
        let mut app = Self {
            client: Client::Retail,
            page: Page::Library,
            filter: Filter::All,
            query: String::new(),
            search_focus: cx.focus_handle(),
            menu: None,
            light_theme: false,
            root_focus: cx.focus_handle(),
            brand_icon: Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                include_bytes!("../../assets/icon.png").to_vec(),
            )),
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
            roots,
            catalog: Arc::new(library::catalog()),
            addons: Vec::new(),
            error: None,
            selected: None,
            scan_id: 0,
            scanning: false,
            checking: false,
            checks: HashMap::new(),
        };
        app.refresh(cx);
        app
    }

    fn index(&self) -> usize {
        match self.client {
            Client::Retail => 0,
            Client::Forever => 1,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.scan_id += 1;
        self.error = None;
        self.selected = None;
        self.addons.clear();
        self.checks.clear();
        self.scanning = false;
        self.checking = false;
        if let Some(root) = self.roots[self.index()].clone() {
            let client = self.client;
            let catalog = self.catalog.clone();
            let scan_id = self.scan_id;
            self.scanning = true;
            let task = cx
                .background_executor()
                .spawn(async move { library::scan(&root, client, &catalog) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |app, cx| {
                    if app.scan_id != scan_id {
                        return;
                    }
                    app.scanning = false;
                    match result {
                        Ok(addons) => app.addons = addons,
                        Err(error) => app.error = Some(format!("Couldn't read addons. {error}")),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn check_updates(&mut self, cx: &mut Context<Self>) {
        if self.scanning || self.checking {
            return;
        }
        let work: Vec<_> = self
            .addons
            .iter()
            .filter_map(|addon| {
                let index = self.catalog.iter().position(|source| {
                    source.folder == addon.folder
                        && source.repo.is_some()
                        && source.supports(self.client)
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
            for (index, folder, installed) in work {
                let catalog = catalog.clone();
                let agent = agent.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { releases::check(&agent, &catalog[index], &installed) })
                    .await;
                let should_continue = this.update(cx, |app, cx| {
                    if app.scan_id != scan_id {
                        return false;
                    }
                    app.checks.insert(folder, result);
                    cx.notify();
                    true
                });
                if !matches!(should_continue, Ok(true)) {
                    break;
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
            None if addon.source == "GitHub" => "Not checked",
            None => "",
        }
    }

    fn choose_root(&mut self, cx: &mut Context<Self>) {
        let index = self.index();
        let client = self.client;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("Choose the {} installation folder", client.label()).into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(result) = picker.await else {
                return;
            };
            let _ = this.update(cx, |app, cx| match result {
                Ok(Some(paths)) => {
                    if let Some(path) = paths.into_iter().next() {
                        if let Err(error) = library::validate_root(&path, client) {
                            if app.client == client {
                                app.error = Some(error.to_string());
                                app.page = Page::Library;
                                cx.notify();
                            }
                            return;
                        }
                        app.roots[index] = Some(path);
                        if app.client == client {
                            app.page = Page::Library;
                            app.refresh(cx);
                        }
                    }
                }
                Err(error) => {
                    app.error = Some(error.to_string());
                    cx.notify();
                }
                Ok(None) => {}
            });
        })
        .detach();
    }
}

fn icon(path: &'static str, size: f32, color: u32) -> gpui::Svg {
    gpui::svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

#[derive(Clone, Copy)]
struct Palette {
    background: u32,
    text: u32,
    muted: u32,
    accent: u32,
    line: u32,
    rule: u32,
    hover: u32,
    panel: u32,
    error: u32,
    success: u32,
}

impl Palette {
    fn new(light: bool) -> Self {
        if light {
            Self {
                background: 0xfdfcf9,
                text: 0x101114,
                muted: 0x666970,
                accent: 0x0057ff,
                line: 0xd7d6d4,
                rule: 0x77787c,
                hover: 0xf3f2ef,
                panel: 0xffffff,
                error: 0xa92920,
                success: 0x067658,
            }
        } else {
            Self {
                background: 0x101114,
                text: 0xf2f1ed,
                muted: 0xa0a3ac,
                accent: 0x7996ff,
                line: 0x353740,
                rule: 0x999b9f,
                hover: 0x24262c,
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

    fn button(self, id: &'static str, name: &'static str) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .h(px(39.))
            .px_3()
            .rounded(px(5.))
            .border_1()
            .border_color(rgb(self.line))
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .hover(move |style| style.bg(rgb(self.hover)))
            .child(name)
    }

    fn icon_button(self, id: &'static str, glyph: &'static str) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(self.hover)))
            .child(icon(glyph, 18., self.muted))
    }

    fn menu_row(
        self,
        id: &'static str,
        glyph: &'static str,
        name: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(40.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(13.))
            .hover(move |style| style.bg(rgb(self.hover)))
            .child(icon(glyph, 16., self.muted))
            .child(name)
    }
}

impl Astral {
    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Library;
        self.menu = None;
        self.selected = None;
        self.search_focus.focus(window);
        cx.notify();
    }

    fn toggle_menu(&mut self, menu: ToolbarMenu, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.selected = None;
        self.root_focus.focus(window);
        cx.notify();
    }

    fn search_box(&self, window: &Window, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.light_theme);
        let focused = self.search_focus.is_focused(window);
        let value = if self.query.is_empty() {
            "Find an addon…".into()
        } else {
            self.query.clone()
        };
        div()
            .id("addon-search")
            .track_focus(&self.search_focus)
            .w(px(204.))
            .h(px(39.))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused {
                palette.accent
            } else {
                palette.line
            }))
            .text_sm()
            .text_color(rgb(if self.query.is_empty() {
                palette.muted
            } else {
                palette.text
            }))
            .child(icon("search", 15., palette.muted))
            .child(div().flex_1().min_w_0().truncate().child(value))
            .when(!self.query.is_empty(), |field| {
                field.child(
                    div()
                        .id("clear-search")
                        .cursor_pointer()
                        .px_1()
                        .child(icon("close", 14., palette.muted))
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.query.clear();
                            cx.notify();
                        })),
                )
            })
            .on_click(cx.listener(|app, _, window, _| app.search_focus.focus(window)))
            .on_key_down(cx.listener(|app, event: &gpui::KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                let command = key.modifiers.platform || key.modifiers.control;
                let changed = match key.key.as_str() {
                    "backspace" if command => {
                        app.query.clear();
                        true
                    }
                    "backspace" => app.query.pop().is_some(),
                    "escape" => {
                        app.query.clear();
                        window.blur();
                        true
                    }
                    "v" if command => {
                        if let Some(value) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            app.query
                                .extend(value.chars().filter(|ch| !ch.is_control()).take(128));
                            true
                        } else {
                            false
                        }
                    }
                    _ if !command => {
                        if let Some(value) = &key.key_char {
                            app.query
                                .extend(value.chars().filter(|ch| !ch.is_control()));
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if changed {
                    app.selected = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
    }

    fn definition(&self, addon: &Addon) -> Option<&Definition> {
        self.catalog
            .iter()
            .find(|source| source.folder == addon.folder && source.supports(self.client))
    }

    fn addon_icon(&self, addon: &Addon, size: f32) -> gpui::Div {
        let palette = Palette::new(self.light_theme);
        let mark = self
            .definition(addon)
            .and_then(|source| source.mark.clone())
            .unwrap_or_else(|| addon.name.chars().take(2).collect());
        let artwork = addon
            .artwork
            .clone()
            .or_else(|| (addon.source == "Astral").then(|| self.brand_icon.clone()));
        let tile = div()
            .size(px(size))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette.panel))
            .text_color(rgb(palette.accent))
            .text_xl()
            .font_weight(gpui::FontWeight::BOLD);
        match artwork {
            Some(image) => tile.child(gpui::img(image).size(px(size - 20.))),
            None => tile.child(mark),
        }
    }

    fn tile(&self, addon: &Addon, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.light_theme);
        let folder = addon.folder.clone();
        let status = self.update_status(addon);
        let status_color = match status {
            "Update available" => palette.accent,
            "Up to date" => palette.success,
            "Check failed" => palette.error,
            _ => palette.muted,
        };
        div()
            .id(SharedString::from(format!("addon:{folder}")))
            .flex_1()
            .min_w_0()
            .h(px(120.))
            .flex()
            .items_center()
            .gap_3()
            .border_t_1()
            .border_color(rgb(palette.rule))
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel)))
            .child(self.addon_icon(addon, 84.))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(px(19.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(addon.name.clone()),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(addon.source == "GitHub", |source| {
                                source.child(icon("github", 12., palette.text))
                            })
                            .child(palette.label(addon.source)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .truncate()
                            .child(addon.version.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(status_color))
                            .child(status),
                    ),
            )
            .child(div().pr_2().child(icon("more", 15., palette.muted)))
            .on_click(cx.listener(move |app, _, _, cx| {
                app.selected = Some(folder.clone());
                cx.notify();
            }))
    }

    fn details(&self, addon: &Addon, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let palette = Palette::new(self.light_theme);
        let definition = self.definition(addon);
        let repo = definition.and_then(|source| source.repo.clone());
        div()
            .id("details-drawer")
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(390.))
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(palette.panel))
            .border_l_1()
            .border_color(rgb(palette.line))
            .shadow_lg()
            .overflow_y_scroll()
            .occlude()
            .child(
                div().flex().justify_end().child(
                    palette
                        .icon_button("close-details", "close")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.selected = None;
                            cx.notify();
                        })),
                ),
            )
            .child(self.addon_icon(addon, 70.))
            .child(
                div()
                    .text_3xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(addon.name.clone()),
            )
            .when_some(
                definition.and_then(|source| source.description.clone()),
                |drawer, description| drawer.child(palette.label(description)),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .pt_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(palette.label(format!("Installed version  ·  {}", addon.version)))
                    .child(palette.label(format!("Source  ·  {}", addon.source)))
                    .when_some(self.checks.get(&addon.folder), |details, check| {
                        details.child(palette.label(match check {
                            Ok(release) => format!("Latest release  ·  {}", release.tag),
                            Err(error) => error.clone(),
                        }))
                    }),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .pt_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Installed modules"),
                    )
                    .children(
                        addon
                            .modules
                            .iter()
                            .cloned()
                            .map(|module| palette.label(module)),
                    ),
            )
            .when_some(repo, |drawer, repo| {
                drawer.child(
                    palette
                        .button("addon-source", "View on GitHub")
                        .on_click(move |_, _, cx| {
                            cx.open_url(&format!("https://github.com/{repo}"))
                        }),
                )
            })
    }
}

impl Render for Astral {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::new(self.light_theme);
        let client = self.client;
        let library = self.page == Page::Library;
        let two_columns = window.viewport_size().width >= px(960.);
        let top = div()
            .h(px(76.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(if cfg!(target_os = "macos") { 94. } else { 38. }))
            .pr(px(38.))
            .border_b_1()
            .border_color(rgb(palette.line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .relative()
                                    .w(px(43.))
                                    .h(px(34.))
                                    .child(
                                        div()
                                            .absolute()
                                            .right_0()
                                            .top_0()
                                            .size(px(34.))
                                            .rounded_full()
                                            .bg(rgb(palette.accent)),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .size(px(34.))
                                            .rounded_full()
                                            .bg(rgb(palette.text)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Astral"),
                            ),
                    )
                    .children(
                        [
                            (Page::Library, "library", "Library"),
                            (Page::Discover, "discover", "Discover"),
                        ]
                        .map(|(page, id, name)| {
                            palette
                                .tab(id, name, self.page == page)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.page = page;
                                    app.selected = None;
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        palette
                            .button("client-menu", "")
                            .w(px(136.))
                            .justify_between()
                            .child(gpui::img(self.client_icons[self.index()].clone()).size(px(24.)))
                            .child(client.label())
                            .child(icon("chevron", 14., palette.muted))
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.toggle_menu(ToolbarMenu::Client, window, cx);
                            })),
                    )
                    .child(
                        palette
                            .icon_button("quick-find", "search")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.focus_search(window, cx);
                            })),
                    )
                    .child(
                        palette
                            .icon_button(
                                "appearance",
                                if self.light_theme { "moon" } else { "sun" },
                            )
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.light_theme = !app.light_theme;
                                app.menu = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        palette
                            .icon_button("tools-menu", "more")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.toggle_menu(ToolbarMenu::Tools, window, cx);
                            })),
                    ),
            );

        let menu = self.menu.map(|kind| {
            let mut panel = div()
                .id("toolbar-menu")
                .absolute()
                .top(px(67.))
                .right(px(if kind == ToolbarMenu::Client {
                    170.
                } else {
                    38.
                }))
                .w(px(if kind == ToolbarMenu::Client {
                    186.
                } else {
                    260.
                }))
                .p_2()
                .flex()
                .flex_col()
                .rounded(px(8.))
                .bg(rgb(palette.panel))
                .border_1()
                .border_color(rgb(palette.line))
                .shadow_lg()
                .occlude();
            match kind {
                ToolbarMenu::Client => {
                    for (index, target) in [Client::Retail, Client::Forever].into_iter().enumerate()
                    {
                        panel = panel.child(
                            div()
                                .id(("choose-client", index))
                                .h(px(40.))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .rounded_md()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .hover(move |style| style.bg(rgb(palette.hover)))
                                .child(gpui::img(self.client_icons[index].clone()).size(px(22.)))
                                .child(div().flex_1().child(target.label()))
                                .when(target == client, |row| {
                                    row.child(icon("check", 16., palette.accent))
                                })
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.menu = None;
                                    if app.client != target {
                                        app.client = target;
                                        app.filter = Filter::All;
                                        app.query.clear();
                                        app.refresh(cx);
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                }
                ToolbarMenu::Tools => {
                    panel = panel
                        .child(
                            palette
                                .menu_row("refresh", "refresh", "Rescan addons")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.menu = None;
                                    app.refresh(cx);
                                })),
                        )
                        .child(
                            palette
                                .menu_row("choose", "folder", "Choose WoW folder")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.menu = None;
                                    cx.notify();
                                    app.choose_root(cx);
                                })),
                        );
                }
            }
            panel
        });

        let can_check = !self.checking
            && !self.scanning
            && self.addons.iter().any(|addon| addon.source == "GitHub");
        let header =
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(56.))
                                .line_height(px(62.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(if library { "Library" } else { "Discover" }),
                        )
                        .child(palette.label(if library {
                            format!("{} addons", self.addons.len())
                        } else {
                            format!(
                                "{} addons for {}",
                                self.catalog
                                    .iter()
                                    .filter(|source| source.supports(client))
                                    .count(),
                                client.label()
                            )
                        })),
                )
                .child(div().flex().items_center().gap_2().flex_shrink_0().when(
                    library,
                    |actions| {
                        actions.child(
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
                                .when(!can_check, |button| button.opacity(0.55).cursor_default())
                                .when(can_check, |button| {
                                    button.on_click(
                                        cx.listener(|app, _, _, cx| app.check_updates(cx)),
                                    )
                                }),
                        )
                    },
                ));

        let mut list = div()
            .id("collection")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(38.))
            .pt(px(40.))
            .pb_4()
            .child(header);
        if library {
            let query = self.query.trim().to_lowercase();
            let matches: Vec<_> = self
                .addons
                .iter()
                .filter(|addon| {
                    addon.name.to_lowercase().contains(&query)
                        || addon.folder.to_lowercase().contains(&query)
                })
                .filter(|addon| match self.filter {
                    Filter::All => true,
                    Filter::Updates => self.update_status(addon) == "Update available",
                    Filter::Manual => addon.source == "Manual",
                })
                .collect();
            content = content.child(
                div()
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
                                palette
                                    .tab(id, name, self.filter == filter)
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.filter = filter;
                                        app.selected = None;
                                        cx.notify();
                                    }))
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
                list = list.child(palette.label(match self.filter {
                    _ if !query.is_empty() => "No addons match your search.",
                    Filter::Updates if self.checks.is_empty() => {
                        "Check for updates to see available releases."
                    }
                    Filter::Updates => "No updates found.",
                    Filter::Manual => "No manually managed addons.",
                    Filter::All if self.roots[self.index()].is_none() => {
                        "Choose your WoW folder to load the library."
                    }
                    Filter::All => "No addons found.",
                }));
            }
            for row in matches.chunks(if two_columns { 2 } else { 1 }) {
                let mut line = div()
                    .flex()
                    .flex_shrink_0()
                    .gap_8()
                    .children(row.iter().map(|addon| self.tile(addon, cx)));
                if two_columns && row.len() == 1 {
                    line = line.child(div().flex_1());
                }
                list = list.child(line);
            }
        } else {
            for source in self.catalog.iter().filter(|source| source.supports(client)) {
                let installed = self
                    .addons
                    .iter()
                    .any(|addon| addon.folder == source.folder);
                let mut row = div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .py_4()
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(source.name.clone()),
                            )
                            .child(palette.label(source.description.clone().unwrap_or_default())),
                    );
                if let Some(repo) = source.repo.clone() {
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("repo:{}", source.folder)))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(palette.accent))
                            .child(if installed {
                                "Installed · GitHub"
                            } else {
                                "View on GitHub"
                            })
                            .on_click(move |_, _, cx| {
                                cx.open_url(&format!("https://github.com/{repo}"))
                            }),
                    );
                } else {
                    row = row.child(palette.label(if installed {
                        "Installed"
                    } else {
                        source.source()
                    }));
                }
                list = list.child(row);
            }
        }

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(palette.background))
            .text_color(rgb(palette.text))
            .child(top)
            .child(content.child(list))
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|app, _: &FindAddon, window, cx| app.focus_search(window, cx)))
            .on_action(cx.listener(|app, _: &DismissOverlay, window, cx| {
                if app.menu.take().is_some() || app.selected.take().is_some() {
                    cx.notify();
                } else if app.search_focus.is_focused(window) {
                    app.query.clear();
                    app.root_focus.focus(window);
                    cx.notify();
                }
            }))
            .when_some(menu, |app, menu| {
                app.child(
                    div()
                        .id("menu-dismiss")
                        .absolute()
                        .top(px(59.))
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.menu = None;
                            cx.notify();
                        })),
                )
                .child(menu)
            })
            .when_some(
                self.selected
                    .as_ref()
                    .and_then(|folder| self.addons.iter().find(|addon| &addon.folder == folder)),
                |app, addon| app.child(self.details(addon, cx)),
            )
    }
}

fn main() {
    let roots = [Client::Retail.detect(), Client::Forever.detect()];
    let args: Vec<_> = std::env::args_os().skip(1).collect();
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
            let root = roots[if client == Client::Retail { 0 } else { 1 }]
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
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--scan")) {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let catalog = library::catalog();
            for (client, root) in [Client::Retail, Client::Forever].into_iter().zip(roots) {
                if let Some(root) = root {
                    let addons = library::scan(&root, client, &catalog)?;
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
    Application::new()
        .with_assets(icons::Icons)
        .run(move |cx: &mut App| {
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
            cx.set_menus(vec![Menu {
                name: "Astral Native".into(),
                items: vec![MenuItem::action("Quit Astral Native", Quit)],
            }]);
            let bounds = Bounds::centered(None, size(px(1240.), px(800.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Astral Native".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: Some(gpui::point(px(18.), px(29.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.), px(480.))),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let app = Astral::new(roots, cx);
                        app.root_focus.focus(window);
                        app
                    })
                },
            )
            .expect("open Astral window");
            cx.activate(true);
        });
}
