use crate::{Astral, Palette, bundled, icon, installer, library::Client, packages, releases};
use gpui::{Context, Window, div, prelude::*, px, rgb};
use std::{cmp::Ordering, path::PathBuf, sync::Arc};

pub struct Review {
    request: u64,
    source: usize,
    origin: Origin,
    package: Option<Result<Arc<packages::Inspection>, String>>,
    checking: bool,
    installing: bool,
    install_error: Option<String>,
}

enum Origin {
    GitHub(Option<Result<Arc<releases::Release>, String>>),
    Bundled,
}

enum PackageInput {
    Download(Arc<releases::Release>),
    Bundled,
}

impl Review {
    fn new(request: u64, source: usize, origin: Origin) -> Self {
        Self {
            request,
            source,
            origin,
            package: None,
            checking: false,
            installing: false,
            install_error: None,
        }
    }

    fn release_tag<'a>(
        &'a self,
        package: &'a packages::Inspection,
        folder: &str,
    ) -> Option<&'a str> {
        match &self.origin {
            Origin::GitHub(Some(Ok(release))) => Some(&release.tag),
            Origin::Bundled => package
                .modules
                .iter()
                .find(|module| module.folder == folder)?
                .version
                .as_deref(),
            Origin::GitHub(_) => None,
        }
    }

    pub fn is_installing(&self) -> bool {
        self.installing
    }
}

pub struct Receipt {
    pub client: Client,
    pub message: String,
    pub backup: PathBuf,
}

impl Astral {
    pub fn open_release(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        let client = self.preferences.client;
        let Some(source) = self.catalog.iter().position(|source| {
            source.id == id
                && (source.bundled == Some(true)
                    || (source.repo.is_some() && source.asset.is_some()))
                && source.supports(client)
        }) else {
            return;
        };
        self.release_request += 1;
        let request = self.release_request;
        let is_bundled = self.catalog[source].bundled == Some(true);
        let origin = if is_bundled {
            Origin::Bundled
        } else {
            Origin::GitHub(None)
        };
        self.release_review = Some(Review::new(request, source, origin));
        self.menu = None;
        self.selected = None;
        window.focus(&self.root_focus);
        if is_bundled {
            self.inspect_release(cx);
            cx.notify();
            return;
        }
        let catalog = self.catalog.clone();
        let root = self.root(client).cloned();
        let previous = self
            .addons
            .iter()
            .find(|addon| addon.folder == catalog[source].folder)
            .map(|addon| addon.version.clone());
        let preferences = self.preferences_path.clone();
        let task = cx.background_executor().spawn(async move {
            let definition = &catalog[source];
            let result = releases::latest(&releases::client(), definition).map(Arc::new);
            if let (Some(root), Some(from), Err(error)) = (root, previous, &result) {
                crate::activity::record(
                    preferences.as_deref(),
                    crate::activity::Action {
                        root,
                        client,
                        name: definition.name.clone(),
                        retry_source: Some(definition.id.clone()),
                        from: Some(from),
                        to: None,
                        verb: "Updated".into(),
                        error: Some(error.clone()),
                        time: 0,
                    },
                );
            }
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if result.is_err() && app.page == crate::Page::Backups {
                    app.load_backups(cx);
                }
                if let Some(review) = app.release_review.as_mut().filter(|r| r.request == request) {
                    review.origin = Origin::GitHub(Some(result));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn inspect_release(&mut self, cx: &mut Context<Self>) {
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let Some(review) = self.release_review.as_mut().filter(|r| !r.checking) else {
            return;
        };
        let input = match &review.origin {
            Origin::GitHub(Some(Ok(release))) => PackageInput::Download(release.clone()),
            Origin::Bundled => PackageInput::Bundled,
            Origin::GitHub(_) => return,
        };
        review.checking = true;
        review.package = None;
        review.install_error = None;
        let request = review.request;
        let source = review.source;
        let catalog = self.catalog.clone();
        let task = cx.background_executor().spawn(async move {
            let bytes = match input {
                PackageInput::Download(release) => releases::download(&release.package)?,
                PackageInput::Bundled => bundled::package(&catalog[source].folder)?.into(),
            };
            packages::inspect(bytes, &root, client, &catalog[source], &catalog).map(Arc::new)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if let Some(review) = app.release_review.as_mut().filter(|r| r.request == request) {
                    review.package = Some(result);
                    review.checking = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn install_release(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.release_review.as_mut().filter(|r| !r.installing) else {
            return;
        };
        let Some(Ok(package)) = &review.package else {
            return;
        };
        let source = &self.catalog[review.source];
        let tag = review
            .release_tag(package, &source.folder)
            .unwrap_or("Local ZIP")
            .to_owned();
        if !package.missing.is_empty()
            || package.installed_version.as_deref().is_some_and(|version| {
                releases::compare(source, version, &tag) == Some(Ordering::Less)
            })
        {
            return;
        }
        review.installing = true;
        review.install_error = None;
        let request = review.request;
        let name = source.name.clone();
        let notes = match &review.origin {
            Origin::GitHub(Some(Ok(release))) => release.notes.clone(),
            _ => String::new(),
        };
        let activity = crate::activity::Action {
            root: package.root.clone(),
            client: package.client,
            name: name.clone(),
            retry_source: Some(source.id.clone()),
            from: package.installed_version.clone(),
            to: Some(tag.clone()),
            verb: if package.installed_version.is_some() {
                "Updated".into()
            } else {
                "Installed".into()
            },
            error: None,
            time: 0,
        };
        let preferences = self.preferences_path.clone();
        let client = package.client;
        let package = package.clone();
        let task = cx.background_executor().spawn(async move {
            let result = installer::install(&package, &name, &tag, &notes, preferences.as_deref());
            crate::activity::record(
                preferences.as_deref(),
                crate::activity::Action {
                    error: result.as_ref().err().cloned(),
                    ..activity
                },
            );
            result.map(|backup| Receipt {
                client,
                message: format!("Installed {name} {tag}"),
                backup,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| match result {
                Ok(receipt) => {
                    app.install_receipt = Some(receipt);
                    app.refresh_after_change(cx);
                }
                Err(error) => {
                    if app.page == crate::Page::Backups {
                        app.load_backups(cx);
                    }
                    if let Some(review) =
                        app.release_review.as_mut().filter(|r| r.request == request)
                    {
                        review.installing = false;
                        review.install_error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn install_notice(
        &self,
        receipt: &Receipt,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let backup = receipt.backup.clone();
        div()
            .flex()
            .flex_shrink_0()
            .justify_between()
            .items_center()
            .gap_3()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(palette.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("check", 15., palette.success))
                    .child(palette.label(receipt.message.clone())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        palette
                            .button("show-install-backup", "Show backup")
                            .on_click(move |_, _, cx| cx.reveal_path(&backup)),
                    )
                    .child(
                        palette
                            .icon_button("dismiss-install-notice", "close")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.install_receipt = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    pub fn release_dialog(
        &self,
        review: &Review,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let source = &self.catalog[review.source];
        let repo = match &review.origin {
            Origin::GitHub(_) => source.repo.clone(),
            Origin::Bundled => None,
        };
        let source_label = repo.clone().unwrap_or_else(|| match review.origin {
            Origin::Bundled => "Astral".into(),
            _ => "Local ZIP".into(),
        });
        let client = self.preferences.client;
        let installed = self
            .addons
            .iter()
            .find(|addon| addon.folder == source.folder);
        let heading = div()
            .flex()
            .justify_between()
            .items_start()
            .gap_4()
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(29.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(source.name.clone()),
                    )
                    .child(palette.label(source_label)),
            )
            .when(!review.installing, |heading| {
                heading.child(
                    palette
                        .icon_button("close-release-review", "close")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.release_review = None;
                            cx.notify();
                        })),
                )
            });
        let mut body = div()
            .id("release-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4();
        let mut actions = div()
            .flex()
            .flex_shrink_0()
            .justify_end()
            .items_center()
            .gap_3()
            .when_some(repo, |actions, repo| {
                actions.child(
                    palette
                        .button("release-project", "View on GitHub")
                        .child(icon("external", 13., palette.muted))
                        .on_click(move |_, _, cx| {
                            cx.open_url(&format!("https://github.com/{repo}"))
                        }),
                )
            });
        match &review.origin {
            Origin::GitHub(None) => body = body.child(palette.label("Loading release…")),
            Origin::GitHub(Some(Err(error))) => {
                body = body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
                let id = source.id.clone();
                actions =
                    actions.child(
                        palette
                            .primary_button("retry-release", "Try again")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.open_release(&id, window, cx)
                            })),
                    );
            }
            Origin::GitHub(Some(Ok(release))) => {
                let mut meta = div()
                    .flex_shrink_0()
                    .p_4()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(palette.line)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(release.tag.clone()),
                    )
                    .child(palette.label(format!(
                        "{} · {:.1} MB",
                        release.package.name,
                        release.package.size as f64 / 1_000_000.
                    )));
                if let Some(date) = release.published.as_deref().and_then(|date| date.get(..10)) {
                    meta = meta.child(palette.label(format!("Published {date}")));
                }
                let installed_version = match &review.package {
                    Some(Ok(package)) => package.installed_version.as_deref(),
                    _ => installed.map(|addon| addon.version.as_str()),
                };
                if let Some(version) = installed_version {
                    meta = meta.child(palette.label(format!("Installed version {version}")));
                }
                body = body.child(meta);
            }
            Origin::Bundled => {}
        }
        if !matches!(review.origin, Origin::GitHub(None | Some(Err(_)))) {
            match &review.package {
                Some(Ok(package)) => {
                    let comparison = package.installed_version.as_deref().and_then(|version| {
                        review
                            .release_tag(package, &source.folder)
                            .and_then(|tag| releases::compare(source, version, tag))
                    });
                    body = body.child(
                        div()
                            .text_color(rgb(palette.success))
                            .text_size(px(13.))
                            .child(format!(
                                "Compatible with {} · Interface {}",
                                client.label(),
                                package.interface
                            )),
                    );
                    for module in &package.modules {
                        body = body.child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .gap_3()
                                .border_b_1()
                                .border_color(palette.line)
                                .pb_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_size(px(13.))
                                        .child(module.folder.clone()),
                                )
                                .child(palette.label({
                                    let change = if module.replaces {
                                        "Replaces installed files"
                                    } else {
                                        "New folder"
                                    };
                                    module.version.as_ref().map_or_else(
                                        || change.to_owned(),
                                        |version| format!("{version} · {change}"),
                                    )
                                })),
                        );
                    }
                    if !package.missing.is_empty() {
                        body = body.child(
                            palette
                                .label(format!(
                                    "Required addons missing: {}",
                                    package
                                        .missing
                                        .iter()
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ))
                                .text_color(rgb(palette.error)),
                        );
                    } else if comparison == Some(Ordering::Less) {
                        body = body.child(
                            palette.label("The installed version is newer than this release."),
                        );
                    } else {
                        body = body.child(palette.label(
                            "Close WoW before installing. Astral backs up replaced folders.",
                        ));
                        let label = match (
                            review.installing,
                            comparison,
                            package.installed_version.is_some(),
                        ) {
                            (true, _, _) => "Installing…",
                            (_, Some(Ordering::Greater), _) => "Update addon",
                            (_, Some(Ordering::Equal), _) => "Reinstall addon",
                            (_, _, true) => "Replace addon",
                            _ => "Install addon",
                        };
                        let button = palette.primary_button("install-release", label);
                        actions = actions.child(if review.installing {
                            button.cursor_default().opacity(0.6)
                        } else {
                            button.on_click(cx.listener(|app, _, _, cx| app.install_release(cx)))
                        });
                    }
                    if let Some(error) = &review.install_error {
                        body =
                            body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
                    }
                }
                Some(Err(error)) => {
                    body = body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
                }
                None if self.root(client).is_none() => {
                    body = body.child(palette.label(format!(
                        "Choose your {} folder in Settings to check this package.",
                        client.label()
                    )));
                }
                None => {}
            }
            if self.root(client).is_some()
                && !review.installing
                && (!matches!(review.package, Some(Ok(_)))
                    || review.install_error.is_some()
                    || matches!(&review.package, Some(Ok(package)) if !package.missing.is_empty()))
            {
                let label = if review.checking {
                    "Checking package…"
                } else if review.package.is_some() {
                    "Check again"
                } else {
                    "Check compatibility"
                };
                let button = palette.primary_button("inspect-release", label);
                actions = actions.child(if review.checking {
                    button.cursor_default().opacity(0.6)
                } else {
                    button.on_click(cx.listener(|app, _, _, cx| app.inspect_release(cx)))
                });
            }
        }
        if let Origin::GitHub(Some(Ok(release))) = &review.origin {
            if !release.notes.trim().is_empty() {
                body = body.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Release notes"),
                        )
                        .child(palette.label(release.notes.clone()).text_size(px(12.))),
                );
            }
        }
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("release-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .bg(gpui::rgba(0x00000088))
                    .occlude()
                    .when(!review.installing, |backdrop| {
                        backdrop.on_click(cx.listener(|app, _, _, cx| {
                            app.release_review = None;
                            cx.notify();
                        }))
                    }),
            )
            .child(
                div()
                    .id("release-review")
                    .occlude()
                    .w(px(575.))
                    .max_w(gpui::relative(0.93))
                    .max_h(gpui::relative(0.85))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .rounded(px(12.))
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(palette.line)
                    .shadow_lg()
                    .child(heading)
                    .child(body)
                    .child(actions),
            )
    }
}
