use crate::{Astral, Client, Definition, Palette, installer, packages, releases, review::Receipt};
use gpui::{Context, FocusHandle, Window, div, prelude::*, px, rgb};
use std::{path::PathBuf, sync::Arc};

pub struct ProjectReview {
    request: u64,
    repo: String,
    root: PathBuf,
    client: Client,
    release: Option<Result<releases::ProjectRelease, String>>,
    inspected: Option<(Definition, Arc<packages::Inspection>)>,
    error: Option<String>,
    busy: bool,
    selected: Option<usize>,
    choosing_package: bool,
    show_notes: bool,
    previous_focus: Option<FocusHandle>,
    close_focus: FocusHandle,
    dialog_focus: FocusHandle,
}

fn asset_pattern(name: &str, tag: &str) -> String {
    let version = tag.trim_start_matches('v');
    let versioned =
        version.starts_with(|ch: char| ch.is_ascii_digit()) && version.chars().any(|ch| ch == '.');
    if versioned && let Some((before, after)) = name.split_once(version) {
        return format!(
            "^{}[0-9][0-9A-Za-z.+_-]*{}$",
            regex::escape(before),
            regex::escape(after)
        );
    }
    format!("^{}$", regex::escape(name))
}

impl Astral {
    pub fn review_project(&mut self, repo: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        self.release_request += 1;
        let request = self.release_request;
        self.project_review = Some(ProjectReview {
            request,
            repo: repo.clone(),
            root,
            client,
            release: None,
            inspected: None,
            error: None,
            busy: false,
            selected: None,
            choosing_package: false,
            show_notes: false,
            previous_focus: window.focused(cx),
            close_focus: cx.focus_handle().tab_stop(true),
            dialog_focus: cx.focus_handle(),
        });
        if let Some(review) = &self.project_review {
            review.close_focus.focus(window);
        }
        self.poll_profile_game_status(cx);
        let task = cx
            .background_executor()
            .spawn(async move { releases::project_packages(&repo) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if let Some(review) = app
                    .project_review
                    .as_mut()
                    .filter(|review| review.request == request)
                {
                    review.selected = result
                        .as_ref()
                        .ok()
                        .filter(|release| release.packages.len() == 1)
                        .map(|_| 0);
                    review.release = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn inspect_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.project_review.as_mut().filter(|review| !review.busy) else {
            return;
        };
        let Some(Ok(release)) = &review.release else {
            return;
        };
        let Some(asset) = review
            .selected
            .and_then(|index| release.packages.get(index))
            .cloned()
        else {
            return;
        };
        let pattern = asset_pattern(&asset.name, &release.tag);
        let (request, root, client, repo) = (
            review.request,
            review.root.clone(),
            review.client,
            review.repo.clone(),
        );
        review.busy = true;
        review.choosing_package = false;
        review.dialog_focus.focus(window);
        review.inspected = None;
        review.error = None;
        let catalog = self.catalog.clone();
        let task = cx.background_executor().spawn(async move {
            let bytes = releases::download(&asset)?;
            packages::inspect_project(bytes, &root, client, &repo, pattern, &catalog)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if let Some(review) = app
                    .project_review
                    .as_mut()
                    .filter(|review| review.request == request)
                {
                    review.busy = false;
                    match result {
                        Ok((source, package)) => {
                            review.inspected = Some((source, Arc::new(package)))
                        }
                        Err(error) => review.error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn install_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.profile_game_running() {
            return;
        }
        let Some(review) = self.project_review.as_mut().filter(|review| !review.busy) else {
            return;
        };
        let Some(Ok(release)) = &review.release else {
            return;
        };
        let Some((source, package)) = review
            .inspected
            .as_ref()
            .filter(|(_, package)| package.missing.is_empty())
        else {
            return;
        };
        review.busy = true;
        review.dialog_focus.focus(window);
        review.error = None;
        let (request, client, tag) = (review.request, review.client, release.tag.clone());
        let notes = release.notes.clone();
        let (source, package) = (source.clone(), package.clone());
        let task = cx.background_executor().spawn(async move {
            installer::install(&package, &source.name, &tag, &notes).map(|backup| {
                let message = format!("Installed {} {tag}", source.name);
                (
                    source,
                    Receipt {
                        client,
                        message,
                        backup,
                    },
                )
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                let Some(review) = app
                    .project_review
                    .as_mut()
                    .filter(|review| review.request == request)
                else {
                    return;
                };
                match result {
                    Ok((source, receipt)) => {
                        app.preferences.projects.push(source.clone());
                        Arc::make_mut(&mut app.catalog).push(source);
                        app.save_preferences();
                        app.project_review = None;
                        app.github_search = None;
                        app.install_receipt = Some(receipt);
                        app.refresh_after_change(cx);
                    }
                    Err(error) => {
                        review.busy = false;
                        review.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss_project_review(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(review) = &mut self.project_review else {
            return false;
        };
        if review.choosing_package {
            review.choosing_package = false;
            review.dialog_focus.focus(window);
            cx.notify();
        } else if !review.busy {
            review
                .previous_focus
                .as_ref()
                .unwrap_or(&self.root_focus)
                .focus(window);
            self.project_review = None;
            cx.notify();
        }
        true
    }

    pub fn project_dialog(
        &self,
        review: &ProjectReview,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let mut body = div()
            .id("project-release-content")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        let url = format!("https://github.com/{}/releases", review.repo);
        let blocked = review.busy || self.profile_game_running();
        match &review.release {
            None => body = body.child(palette.label("Loading release…")),
            Some(Err(error)) => {
                body = body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
            }
            Some(Ok(release)) => {
                let selected = review
                    .selected
                    .and_then(|index| release.packages.get(index));
                body = body.child(
                    palette
                        .label(format!("{} · {}", review.repo, release.tag))
                        .text_size(px(12.)),
                );
                if let Some((_, package)) = &review.inspected {
                    body = body.child(div().flex().items_center().gap_2()
                        .child(crate::icon("check", 17., palette.accent))
                        .child(palette.label(format!("Compatible with this {} build · {}",
                            review.client.label(), selected.map(|asset| asset.name.as_str()).unwrap_or("ZIP")))))
                        .child(div().id("project-package-folders").max_h(px(230.)).overflow_y_scroll()
                            .children(package.modules.iter().map(|module| {
                                div().flex().flex_col().gap_1().py(px(11.)).border_b_1().border_color(palette.line)
                                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_size(px(12.)).child(module.folder.clone()))
                                    .child(palette.label(format!("{} · {}", module.version.as_deref().unwrap_or("Unknown"),
                                        if module.replaces { "Replaces installed files" } else { "New addon folder" })).text_size(px(11.)))
                            })))
                        .when(!package.missing.is_empty(), |body| body.child(palette.label(format!(
                            "Install required addons first: {}", package.missing.iter().cloned().collect::<Vec<_>>().join(", ")
                        )).text_color(rgb(palette.error))))
                        .when(self.profile_game_running(), |body| body.child(palette.label("Close World of Warcraft before installing.")
                            .text_color(rgb(palette.error))))
                        .child(palette.label("Existing addon files are backed up. Your saved settings stay in place.").text_size(px(11.)));
                    let install = palette
                        .primary_button(
                            "install-community-addon",
                            if review.busy {
                                "Installing…"
                            } else {
                                "Install release"
                            },
                        )
                        .flex_row_reverse()
                        .child(crate::icon("plus", 15., 0xffffff));
                    let alternate =
                        palette.button("choose-different-asset", "Choose another package");
                    body = body.child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(if review.busy {
                                alternate.opacity(0.55).cursor_default()
                            } else {
                                crate::keyboard_button(
                                    alternate,
                                    palette,
                                    cx.listener(|app, _: &(), window, cx| {
                                        if let Some(review) = &mut app.project_review {
                                            review.inspected = None;
                                            review.error = None;
                                            review.dialog_focus.focus(window);
                                            cx.notify();
                                        }
                                    }),
                                )
                            })
                            .child(if blocked || !package.missing.is_empty() {
                                install.opacity(0.55).cursor_default()
                            } else {
                                crate::keyboard_button(
                                    install,
                                    palette,
                                    cx.listener(|app, _: &(), window, cx| {
                                        app.install_project(window, cx)
                                    }),
                                )
                            }),
                    );
                } else {
                    let package = palette
                        .button(
                            "choose-project-package",
                            selected
                                .map(package_label)
                                .unwrap_or_else(|| "Choose an addon ZIP".into()),
                        )
                        .w_full()
                        .justify_between()
                        .child(crate::icon("chevron", 14., palette.muted));
                    body = body
                        .child(palette.label("Release package").text_size(px(12.)))
                        .child(if release.packages.is_empty() || review.busy {
                            package.opacity(0.55).cursor_default()
                        } else {
                            crate::keyboard_button(
                                package,
                                palette,
                                cx.listener(|app, _: &(), _, cx| {
                                    if let Some(review) = &mut app.project_review {
                                        review.choosing_package = !review.choosing_package;
                                        cx.notify();
                                    }
                                }),
                            )
                        });
                    if release.packages.is_empty() {
                        body = body.child(palette.label("The author has not published a packaged ZIP release. Source-code archives cannot be installed as addons.")
                            .text_color(rgb(palette.error)));
                    } else if review.choosing_package {
                        body = body.child(
                            div()
                                .id("project-package-options")
                                .max_h(px(180.))
                                .overflow_y_scroll()
                                .children(release.packages.iter().enumerate().map(
                                    |(index, asset)| {
                                        crate::keyboard_button(
                                            palette
                                                .button(
                                                    ("review-asset", index),
                                                    package_label(asset),
                                                )
                                                .w_full()
                                                .justify_between()
                                                .when(review.selected == Some(index), |row| {
                                                    row.child(crate::icon(
                                                        "check",
                                                        13.,
                                                        palette.accent,
                                                    ))
                                                }),
                                            palette,
                                            cx.listener(move |app, _: &(), window, cx| {
                                                if let Some(review) = &mut app.project_review {
                                                    review.selected = Some(index);
                                                    review.choosing_package = false;
                                                    review.error = None;
                                                    review.dialog_focus.focus(window);
                                                    cx.notify();
                                                }
                                            }),
                                        )
                                    },
                                )),
                        );
                    }
                    let inspect = palette.primary_button(
                        "check-community-package",
                        if review.busy {
                            "Checking package…"
                        } else {
                            "Check compatibility"
                        },
                    );
                    body = body.child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(crate::keyboard_button(
                                palette
                                    .button("community-github", "View project")
                                    .child(crate::icon("external", 13., palette.muted)),
                                palette,
                                move |_, _, cx| cx.open_url(&url),
                            ))
                            .child(if review.busy || selected.is_none() {
                                inspect.opacity(0.55).cursor_default()
                            } else {
                                crate::keyboard_button(
                                    inspect,
                                    palette,
                                    cx.listener(|app, _: &(), window, cx| {
                                        app.inspect_project(window, cx)
                                    }),
                                )
                            }),
                    );
                }
                if !release.notes.is_empty() {
                    body = body
                        .child(div().mt(px(12.)).child(crate::keyboard_button(
                            palette.text_button("community-release-notes", "Release notes"),
                            palette,
                            cx.listener(|app, _: &(), _, cx| {
                                if let Some(review) = &mut app.project_review {
                                    review.show_notes = !review.show_notes;
                                    cx.notify();
                                }
                            }),
                        )))
                        .when(review.show_notes, |body| {
                            body.child(
                                palette
                                    .label(release.notes.clone())
                                    .p_3()
                                    .bg(rgb(palette.surface))
                                    .text_size(px(11.))
                                    .line_height(px(20.)),
                            )
                        });
                }
            }
        }
        if let Some(error) = &review.error {
            body = body.child(palette.label(error.clone()).text_color(rgb(palette.error)));
        }
        let close = palette
            .icon_button("close-community-review", "close")
            .track_focus(&review.close_focus)
            .absolute()
            .top(px(14.))
            .right(px(14.));
        let close = if review.busy {
            close.opacity(0.55).cursor_default()
        } else {
            crate::keyboard_button(
                close,
                palette,
                cx.listener(|app, _: &(), window, cx| {
                    if let Some(review) = &mut app.project_review {
                        review.choosing_package = false;
                    }
                    app.dismiss_project_review(window, cx);
                }),
            )
        };
        let modal_focus = review.dialog_focus.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("project-release-backdrop")
                    .absolute()
                    .top(px(78.))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .bg(gpui::rgba(0x050b08b3))
                    .occlude()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dismiss_project_review(window, cx);
                    })),
            )
            .child(
                div()
                    .id("project-release-dialog")
                    .w(px(460.))
                    .max_w(gpui::relative(0.90))
                    .max_h(gpui::relative(0.85))
                    .p(px(30.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .occlude()
                    .bg(rgb(palette.panel))
                    .rounded(px(14.))
                    .shadow_lg()
                    .border_1()
                    .border_color(palette.line)
                    .track_focus(&review.dialog_focus)
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.key != "tab" {
                            return;
                        }
                        cx.stop_propagation();
                        // GPUI's tab order includes the underlying page. Skip those
                        // stops while the modal is open, in either direction.
                        let initial = window.focused(cx);
                        loop {
                            let prior = window.focused(cx);
                            if event.keystroke.modifiers.shift {
                                window.focus_prev();
                            } else {
                                window.focus_next();
                            }
                            let current = window.focused(cx);
                            if modal_focus.contains_focused(window, cx) {
                                break;
                            }
                            if current == prior || current == initial {
                                modal_focus.focus(window);
                                break;
                            }
                        }
                    })
                    .child(close)
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(
                                review
                                    .repo
                                    .split('/')
                                    .next_back()
                                    .unwrap_or(&review.repo)
                                    .to_owned(),
                            ),
                    )
                    .child(body),
            )
    }
}

fn package_label(asset: &releases::Package) -> String {
    format!("{} · {:.1} MB", asset.name, asset.size as f64 / 1_048_576.)
}
