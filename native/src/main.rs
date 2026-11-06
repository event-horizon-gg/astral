#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod artwork;
mod library;
mod releases;

use gpui::{
    App, Application, Bounds, Context, KeyBinding, Menu, MenuItem, PathPromptOptions, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use library::{Addon, Client, Definition};
use std::{cmp::Ordering, collections::HashMap, path::PathBuf, sync::Arc};

gpui::actions!(astral, [Quit]);

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

struct Astral {
    client: Client,
    page: Page,
    filter: Filter,
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

fn label(text: impl Into<SharedString>) -> gpui::Div {
    div().text_sm().text_color(rgb(0x9ca3af)).child(text.into())
}

fn tab(id: &'static str, name: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .py_2()
        .text_sm()
        .border_b_2()
        .border_color(rgb(if active { 0x597eff } else { 0x101114 }))
        .text_color(rgb(if active { 0xf2f3f5 } else { 0x9ca3af }))
        .hover(|style| style.text_color(rgb(0xffffff)))
        .child(name)
}

fn button(id: &'static str, text: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .bg(rgb(if active { 0x315feb } else { 0x202328 }))
        .hover(|style| style.bg(rgb(if active { 0x426cf1 } else { 0x2d3139 })))
        .child(text)
}

impl Astral {
    fn definition(&self, addon: &Addon) -> Option<&Definition> {
        self.catalog
            .iter()
            .find(|source| source.folder == addon.folder && source.supports(self.client))
    }

    fn addon_icon(&self, addon: &Addon, size: f32) -> gpui::Div {
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
            .bg(rgb(0x191b20))
            .text_color(rgb(0xaebfff))
            .text_xl()
            .font_weight(gpui::FontWeight::BOLD);
        match artwork {
            Some(image) => tile.child(gpui::img(image).size(px(size - 20.))),
            None => tile.child(mark),
        }
    }

    fn tile(&self, addon: &Addon, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let folder = addon.folder.clone();
        let status = self.update_status(addon);
        let status_color = match status {
            "Update available" => 0x96adff,
            "Up to date" => 0x62c5a3,
            "Check failed" => 0xffad9e,
            _ => 0x9ca3af,
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
            .border_color(rgb(0x999b9f))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x191b20)))
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
                    .child(label(addon.source))
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
            .child(div().pr_2().text_color(rgb(0x9ca3af)).child("⋯"))
            .on_click(cx.listener(move |app, _, _, cx| {
                app.selected = Some(folder.clone());
                cx.notify();
            }))
    }

    fn details(&self, addon: &Addon, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
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
            .bg(rgb(0x191b20))
            .border_l_1()
            .border_color(rgb(0x343741))
            .shadow_lg()
            .overflow_y_scroll()
            .occlude()
            .child(div().flex().justify_end().child(
                button("close-details", "Close", false).on_click(cx.listener(|app, _, _, cx| {
                    app.selected = None;
                    cx.notify();
                })),
            ))
            .child(self.addon_icon(addon, 70.))
            .child(
                div()
                    .text_3xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(addon.name.clone()),
            )
            .when_some(
                definition.and_then(|source| source.description.clone()),
                |drawer, description| drawer.child(label(description)),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(0x343741))
                    .pt_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label(format!("Installed version  ·  {}", addon.version)))
                    .child(label(format!("Source  ·  {}", addon.source)))
                    .when_some(self.checks.get(&addon.folder), |details, check| {
                        details.child(label(match check {
                            Ok(release) => format!("Latest release  ·  {}", release.tag),
                            Err(error) => error.clone(),
                        }))
                    }),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(0x343741))
                    .pt_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Installed modules"),
                    )
                    .children(addon.modules.iter().cloned().map(label)),
            )
            .when_some(repo, |drawer, repo| {
                drawer.child(
                    button("addon-source", "View on GitHub", false).on_click(move |_, _, cx| {
                        cx.open_url(&format!("https://github.com/{repo}"))
                    }),
                )
            })
    }
}

impl Render for Astral {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client;
        let library = self.page == Page::Library;
        let two_columns = window.viewport_size().width >= px(960.);
        let top = div()
            .h(px(66.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px_8()
            .border_b_1()
            .border_color(rgb(0x2a2c32))
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
                                            .bg(rgb(0x4e78ff)),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .size(px(34.))
                                            .rounded_full()
                                            .bg(rgb(0xf2f3f5)),
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
                            tab(id, name, self.page == page).on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.page = page;
                                    app.selected = None;
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
            )
            .child(
                div().flex().items_center().gap_2().children(
                    [(Client::Retail, "retail"), (Client::Forever, "forever")]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (target, id))| {
                            let icon = self.client_icons[index].clone();
                            button(id, target.label(), client == target)
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(gpui::img(icon).size(px(22.)))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    if app.client != target {
                                        app.client = target;
                                        app.filter = Filter::All;
                                        app.refresh(cx);
                                    }
                                }))
                        }),
                ),
            );

        let can_check = !self.checking
            && !self.scanning
            && self.addons.iter().any(|addon| addon.source == "GitHub");
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
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(50.))
                            .line_height(px(56.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(if library { "Library" } else { "Discover" }),
                    )
                    .child(label(if library {
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .when(library, |actions| {
                        actions.child(
                            button(
                                "updates",
                                if self.checking {
                                    "Checking…"
                                } else {
                                    "Check for updates"
                                },
                                can_check,
                            )
                            .when(can_check, |button| {
                                button.on_click(cx.listener(|app, _, _, cx| app.check_updates(cx)))
                            }),
                        )
                    })
                    .child(
                        button("refresh", "Rescan", false)
                            .on_click(cx.listener(|app, _, _, cx| app.refresh(cx))),
                    )
                    .child(
                        button("choose", "WoW folder", false)
                            .on_click(cx.listener(|app, _, _, cx| app.choose_root(cx))),
                    ),
            );

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
            .px_8()
            .py_7()
            .child(header);
        if library {
            let matches: Vec<_> = self
                .addons
                .iter()
                .filter(|addon| match self.filter {
                    Filter::All => true,
                    Filter::Updates => self.update_status(addon) == "Update available",
                    Filter::Manual => addon.source == "Manual",
                })
                .collect();
            content = content.child(
                div().flex().gap_2().children(
                    [
                        (Filter::All, "all", "All addons"),
                        (Filter::Updates, "filter-updates", "Updates"),
                        (Filter::Manual, "manual", "Manual"),
                    ]
                    .map(|(filter, id, name)| {
                        tab(id, name, self.filter == filter).on_click(cx.listener(
                            move |app, _, _, cx| {
                                app.filter = filter;
                                app.selected = None;
                                cx.notify();
                            },
                        ))
                    }),
                ),
            );
            if let Some(error) = &self.error {
                list = list.child(div().py_2().text_color(rgb(0xffad9e)).child(error.clone()));
            }
            if self.scanning {
                list = list.child(label("Scanning addons…"));
            } else if matches.is_empty() {
                list = list.child(label(match self.filter {
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
                    .border_color(rgb(0x383b44))
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
                            .child(label(source.description.clone().unwrap_or_default())),
                    );
                if let Some(repo) = source.repo.clone() {
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("repo:{}", source.folder)))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(0x96adff))
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
                    row = row.child(label(if installed {
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
            .bg(rgb(0x101114))
            .text_color(rgb(0xf2f3f5))
            .child(top)
            .child(content.child(list))
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
    Application::new().run(move |cx: &mut App| {
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
        cx.set_menus(vec![Menu {
            name: "Astral Native".into(),
            items: vec![MenuItem::action("Quit Astral Native", Quit)],
        }]);
        let bounds = Bounds::centered(None, size(px(1080.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Astral Native".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Astral::new(roots, cx)),
        )
        .expect("open Astral window");
        cx.activate(true);
    });
}
