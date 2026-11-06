#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod library;
mod releases;

use gpui::{
    App, Application, Bounds, Context, PathPromptOptions, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use library::{Addon, Client, Definition};
use std::{cmp::Ordering, collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Discover,
}

struct Astral {
    client: Client,
    page: Page,
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
                app.checking = false;
                cx.notify();
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
            None if addon.source == "GitHub" => "Unchecked",
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
                        app.roots[index] = Some(path);
                        if app.client == client {
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

impl Render for Astral {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client;
        let root = self.roots[self.index()].as_ref();
        let selected = self
            .selected
            .as_ref()
            .and_then(|folder| self.addons.iter().find(|a| &a.folder == folder));
        let title = match self.page {
            Page::Library => "Library",
            Page::Discover => "Discover",
        };
        let top = div()
            .flex()
            .items_center()
            .justify_between()
            .px_8()
            .py_4()
            .border_b_1()
            .border_color(rgb(0x292c31))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Astral"),
                    )
                    .child(
                        button("library", "Library", self.page == Page::Library).on_click(
                            cx.listener(|app, _, _, cx| {
                                app.page = Page::Library;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("discover", "Discover", self.page == Page::Discover).on_click(
                            cx.listener(|app, _, _, cx| {
                                app.page = Page::Discover;
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("retail", "Retail", client == Client::Retail).on_click(cx.listener(
                            |app, _, _, cx| {
                                if app.client != Client::Retail {
                                    app.client = Client::Retail;
                                    app.refresh(cx);
                                }
                            },
                        )),
                    )
                    .child(
                        button("forever", "Forever", client == Client::Forever).on_click(
                            cx.listener(|app, _, _, cx| {
                                if app.client != Client::Forever {
                                    app.client = Client::Forever;
                                    app.refresh(cx);
                                }
                            }),
                        ),
                    ),
            );

        let can_check = !self.checking
            && !self.scanning
            && self.addons.iter().any(|addon| addon.source == "GitHub");
        let header = div()
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
                            .text_3xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(title),
                    )
                    .child(label(
                        root.map(|path| path.display().to_string())
                            .unwrap_or_else(|| "WoW folder not selected".into()),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_shrink_0()
                    .when(self.page == Page::Library, |header| {
                        header.child(
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
                        button("choose", "Choose WoW folder", false)
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
        if self.page == Page::Library {
            if let Some(error) = &self.error {
                list = list.child(div().py_3().text_color(rgb(0xffad9e)).child(error.clone()));
            } else if self.scanning {
                list = list.child(label("Scanning addons…"));
            } else if self.addons.is_empty() {
                list = list.child(label(if root.is_some() {
                    "No addons found."
                } else {
                    "Choose your installation folder to load the library."
                }));
            } else {
                list = list.child(label(format!("{} addon groups", self.addons.len())));
            }
            for addon in &self.addons {
                let folder = addon.folder.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("addon:{folder}")))
                        .flex()
                        .items_center()
                        .gap_4()
                        .py_3()
                        .px_2()
                        .border_b_1()
                        .border_color(rgb(0x292c31))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x1b1e24)))
                        .child(
                            div()
                                .flex_1()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(addon.name.clone()),
                        )
                        .child(div().w(px(140.)).child(label(addon.version.clone())))
                        .child(div().w(px(76.)).child(label(addon.source)))
                        .child(div().w(px(140.)).child(label(self.update_status(addon))))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.selected = Some(folder.clone());
                            cx.notify();
                        })),
                );
            }
        } else {
            for entry in self.catalog.iter().filter(|entry| entry.supports(client)) {
                let repo = entry.repo.clone();
                let mut row = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(0x292c31))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(entry.name.clone())
                            .child(label(entry.description.clone().unwrap_or_default())),
                    );
                if let Some(repo) = repo {
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("repo:{}", entry.folder)))
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(0x96adff))
                            .child("View on GitHub")
                            .on_click(move |_, _, cx| {
                                cx.open_url(&format!("https://github.com/{repo}"))
                            }),
                    );
                } else {
                    row = row.child(label(entry.source()));
                }
                list = list.child(row);
            }
        }

        let mut content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_5()
            .px_8()
            .py_6()
            .child(header)
            .child(list);
        if self.page == Page::Library {
            if let Some(addon) = selected {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .rounded_md()
                        .bg(rgb(0x1b1e24))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(addon.name.clone())
                                .child(button("close-details", "Close", false).on_click(
                                    cx.listener(|app, _, _, cx| {
                                        app.selected = None;
                                        cx.notify();
                                    }),
                                )),
                        )
                        .child(label(format!("{} · {}", addon.version, addon.source)))
                        .child(label(addon.modules.join(", ")))
                        .when_some(self.checks.get(&addon.folder), |details, check| {
                            details.child(label(match check {
                                Ok(release) => format!("Latest release · {}", release.tag),
                                Err(error) => error.clone(),
                            }))
                        }),
                );
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x101114))
            .text_color(rgb(0xf2f3f5))
            .child(top)
            .child(content)
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
