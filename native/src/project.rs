use crate::{Astral, Client, Definition, Palette, installer, packages, releases};
use gpui::{Context, FocusHandle, Window, div, prelude::*, px, rgb};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct ProjectReview {
    request: u64,
    repo: String,
    root: Option<PathBuf>,
    client: Client,
    release: Option<Result<releases::ProjectRelease, String>>,
    abandoned: bool,
    inspected: Option<(Definition, Arc<packages::Inspection>)>,
    inspected_at: Option<Instant>,
    busy: bool,
    selected: Option<usize>,
    choosing_package: bool,
    show_notes: bool,
    previous_focus: Option<FocusHandle>,
    close_focus: FocusHandle,
    dialog_focus: FocusHandle,
}

impl ProjectReview {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.release.as_ref().is_some_and(Result::is_ok)
    }

    pub(crate) fn leave_discovery(&mut self) {
        if self.release.is_none() {
            self.abandoned = true;
        }
    }
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

fn review_current(
    root: Option<&Path>,
    client: Client,
    current_root: Option<&Path>,
    current_client: Client,
    inspected_at: Option<Instant>,
    now: Instant,
) -> bool {
    root.is_some()
        && client == current_client
        && current_root == root
        && inspected_at
            .is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(600))
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    #[test]
    fn package_review_expiry_and_installation_scope_are_checked_before_installing() {
        let root = Path::new("/fixture/_retail_");
        let other = Path::new("/other/_retail_");
        let at = Instant::now();
        let valid = |root_now, client_now, inspected, now| {
            review_current(
                Some(root),
                Client::Retail,
                root_now,
                client_now,
                inspected,
                now,
            )
        };
        assert!(valid(
            Some(root),
            Client::Retail,
            Some(at),
            at + Duration::from_secs(600)
        ));
        assert!(!valid(
            Some(root),
            Client::Retail,
            Some(at),
            at + Duration::from_millis(600001)
        ));
        assert!(!valid(Some(other), Client::Retail, Some(at), at));
        assert!(!valid(Some(root), Client::Forever, Some(at), at));
        assert!(!valid(None, Client::Retail, Some(at), at));
        assert!(!valid(Some(root), Client::Retail, None, at));
        assert!(!review_current(
            None,
            Client::Retail,
            None,
            Client::Retail,
            Some(at),
            at
        ));
        assert!(!review_current(
            None,
            Client::Retail,
            Some(root),
            Client::Retail,
            Some(at),
            at
        ));
    }

    #[test]
    fn reviewing_a_catalog_release_does_not_register_a_duplicate_custom_source() {
        let mut preferences = crate::preferences::Preferences::default();
        let mut catalog = Arc::new(crate::library::catalog());
        let count = catalog.len();
        let source = catalog
            .iter()
            .find(|source| source.id == "keyui")
            .unwrap()
            .clone();
        assert!(!remember_project(
            &mut preferences,
            &mut catalog,
            source.clone()
        ));
        assert!(!remember_project(&mut preferences, &mut catalog, source));
        assert_eq!(catalog.len(), count);
        assert!(preferences.projects.is_empty());
    }

    #[test]
    fn reviewing_a_registered_community_release_updates_one_source_in_place() {
        let mut preferences = crate::preferences::Preferences::default();
        let mut catalog = Arc::new(crate::library::catalog());
        let count = catalog.len();
        let mut source = Definition::github_project(
            "owner/SampleAddon",
            Client::Retail,
            vec!["SampleAddon".into()],
            "SampleAddon".into(),
            "^first.zip$".into(),
        );
        assert!(remember_project(
            &mut preferences,
            &mut catalog,
            source.clone()
        ));
        source.asset = Some("^second.zip$".into());
        source.requires = vec!["ExternalDep".into()];
        assert!(remember_project(
            &mut preferences,
            &mut catalog,
            source.clone()
        ));
        assert_eq!(catalog.len(), count + 1);
        assert_eq!(preferences.projects.len(), 1);
        assert_eq!(preferences.projects[0].asset, source.asset);
        assert_eq!(
            catalog.iter().filter(|item| item.id == source.id).count(),
            1
        );
        assert_eq!(
            catalog
                .iter()
                .find(|item| item.id == source.id)
                .unwrap()
                .requires,
            vec!["ExternalDep"]
        );
    }
}

fn remember_project(
    preferences: &mut crate::preferences::Preferences,
    catalog: &mut Arc<Vec<Definition>>,
    source: Definition,
) -> bool {
    if let Some(saved) = preferences
        .projects
        .iter_mut()
        .find(|item| item.id == source.id)
    {
        *saved = source.clone();
        let catalog = Arc::make_mut(catalog);
        if let Some(saved) = catalog.iter_mut().find(|item| item.id == source.id) {
            *saved = source;
        } else {
            catalog.push(source);
        }
        true
    } else if !catalog.iter().any(|item| item.id == source.id) {
        preferences.projects.push(source.clone());
        Arc::make_mut(catalog).push(source);
        true
    } else {
        false
    }
}

impl Astral {
    pub fn review_project(&mut self, repo: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.addon_controls_busy() {
            return;
        }
        let client = self.preferences.client;
        let root = self.root(client).cloned();
        self.release_request += 1;
        let request = self.release_request;
        self.project_review = Some(ProjectReview {
            request,
            repo: repo.clone(),
            root,
            client,
            release: None,
            abandoned: false,
            inspected: None,
            inspected_at: None,
            busy: true,
            selected: None,
            choosing_package: false,
            show_notes: false,
            previous_focus: window.focused(cx),
            close_focus: cx.focus_handle().tab_stop(true),
            dialog_focus: cx.focus_handle(),
        });
        self.poll_profile_game_status(cx);
        let task = cx
            .background_executor()
            .spawn(async move { releases::project_packages(&repo) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |app, window, cx| {
                let Some(review) = app
                    .project_review
                    .as_mut()
                    .filter(|review| review.request == request)
                else {
                    return;
                };
                match result {
                    Ok(release) if !review.abandoned => {
                        review.busy = false;
                        review.selected = (release.packages.len() == 1).then_some(0);
                        review.release = Some(Ok(release));
                        review.close_focus.focus(window);
                    }
                    Ok(_) => {
                        app.project_review = None;
                    }
                    Err(error) => {
                        app.project_review = None;
                        app.show_notification(error, true, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn inspect_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.project_review.as_mut().filter(|review| !review.busy) else {
            return;
        };
        let Some(root) = review.root.clone() else {
            self.show_notification(
                "Choose your WoW folder in Settings before checking compatibility.",
                true,
                cx,
            );
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
        let (request, root, client, repo) =
            (review.request, root, review.client, review.repo.clone());
        review.busy = true;
        review.choosing_package = false;
        review.dialog_focus.focus(window);
        review.inspected = None;
        review.inspected_at = None;
        let catalog = self.catalog.clone();
        let task = cx.background_executor().spawn(async move {
            // Electron re-fetches latest before inspection, matching the selected
            // GitHub asset ID rather than silently trusting an old download URL.
            let fresh = releases::project_packages(&repo)?;
            let selected = fresh.package_index(asset.id)?;
            let asset = &fresh.packages[selected];
            let pattern = asset_pattern(&asset.name, &fresh.tag);
            let bytes = releases::download(asset)?;
            let (source, package) =
                packages::inspect_project(bytes, &root, client, &repo, pattern, &catalog)?;
            Ok::<_, String>((fresh, selected, source, package))
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
                        Ok((release, selected, source, package)) => {
                            review.release = Some(Ok(release));
                            review.selected = Some(selected);
                            review.inspected = Some((source, Arc::new(package)));
                            review.inspected_at = Some(Instant::now());
                        }
                        Err(error) => app.show_notification(error, true, cx),
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
        let current_client = self.preferences.client;
        let current_root = self.root(current_client).cloned();
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
        if !review_current(
            review.root.as_deref(),
            review.client,
            current_root.as_deref(),
            current_client,
            review.inspected_at,
            Instant::now(),
        ) {
            self.show_notification(
                "This package review expired or belongs to another game client. Review it again.",
                true,
                cx,
            );
            return;
        }
        review.busy = true;
        review.dialog_focus.focus(window);
        let (request, tag) = (review.request, release.tag.clone());
        let notes = release.notes.clone();
        let (source, package) = (source.clone(), package.clone());
        let task = cx.background_executor().spawn(async move {
            installer::install(&package, &source.name, &tag, &notes).map(|_| source)
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
                    Ok(source) => {
                        if remember_project(&mut app.preferences, &mut app.catalog, source) {
                            app.save_preferences();
                        }
                        app.project_review = None;
                        app.github_search = None;
                        app.refresh_after_change(cx);
                        app.show_notification(
                            "Addon installed. Previous files are backed up in Activity.",
                            false,
                            cx,
                        );
                    }
                    Err(error) => {
                        review.busy = false;
                        app.show_notification(error, true, cx);
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
        if !review.is_visible() {
            return false;
        }
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
        let mut body = div().flex().flex_col().flex_shrink_0();
        let url = format!("https://github.com/{}/releases", review.repo);
        let blocked = review.busy || self.profile_game_running();
        let warning = |text: String| {
            palette
                .label(text)
                .text_size(px(11.))
                .line_height(px(18.7))
                .p(px(14.))
                .mb(px(20.))
                .rounded(px(8.))
                .border_1()
                .border_color(palette.line)
                .bg(rgb(palette.surface))
                .text_color(rgb(0xd18b35))
        };
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
                        .text_size(px(12.))
                        .text_color(rgb(0x8da898))
                        .line_height(px(22.8))
                        .mt(px(13.))
                        .mb(px(25.)),
                );
                if let Some((_, package)) = &review.inspected {
                    body = body.child(div().flex().items_center().gap(px(9.))
                        .mt(px(16.)).p(px(14.)).rounded(px(8.))
                        .border_1().border_color(palette.line).bg(rgb(palette.surface))
                        .text_size(px(11.)).line_height(px(18.7))
                        .child(crate::icon("shield-check", 17., palette.muted))
                        .child(palette.label(format!("Compatible with this {} build · {}",
                            review.client.label(), selected.map(|asset| asset.name.as_str()).unwrap_or("ZIP"))).text_size(px(11.))))
                        .child(div().id("project-package-folders").mt(px(20.)).max_h(px(230.)).overflow_y_scroll()
                            .children(package.modules.iter().map(|module| {
                                div().flex().flex_col().gap(px(6.)).py(px(11.)).border_b_1().border_color(gpui::rgba(0xbac7ff15))
                                    .child(div().font_weight(gpui::FontWeight::BOLD).text_size(px(12.)).child(module.name.clone()))
                                    .child(palette.label(format!("{} · {}", module.version.as_deref().unwrap_or("Unknown"),
                                        if module.replaces { "Replaces installed files" } else { "New addon folder" })).text_size(px(11.)).text_color(rgb(0x8d9ab7)))
                            })))
                        .when(!package.missing.is_empty(), |body| body.child(warning(format!(
                            "Install required addons first: {}", package.missing.iter().cloned().collect::<Vec<_>>().join(", ")
                        ))))
                        .when(self.profile_game_running(), |body| body.child(warning("Close World of Warcraft before installing.".into())))
                        .child(palette.label("Existing addon files are backed up. Your saved settings stay in place.")
                            .text_size(px(11.)).line_height(px(19.8)).mt(px(13.)).mb(px(25.)));
                    let install = palette
                        .primary_button("install-community-addon", "Install release")
                        .flex_row_reverse()
                        .child(crate::icon("plus", 15., 0xffffff));
                    let alternate =
                        palette.button("choose-different-asset", "Choose another package");
                    body = body.child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(9.))
                            .child(if review.busy {
                                alternate.opacity(0.45).cursor_default()
                            } else {
                                crate::keyboard_button(
                                    alternate,
                                    palette,
                                    cx.listener(|app, _: &(), window, cx| {
                                        if let Some(review) = &mut app.project_review {
                                            review.inspected = None;
                                            review.inspected_at = None;
                                            review.dialog_focus.focus(window);
                                            cx.notify();
                                        }
                                    }),
                                )
                            })
                            .child(if blocked || !package.missing.is_empty() {
                                install.opacity(0.45).cursor_default()
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
                        .h_auto()
                        .p(px(9.))
                        .font_weight(gpui::FontWeight::NORMAL)
                        .bg(rgb(palette.surface))
                        .text_color(rgb(palette.text))
                        .justify_between()
                        .child(crate::icon("chevron", 14., palette.muted));
                    let selector = div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .mt(px(18.))
                        .py(px(17.))
                        .border_b_1()
                        .border_color(palette.line)
                        .child(
                            palette
                                .label("Release package")
                                .text_size(px(12.))
                                .text_color(rgb(palette.text)),
                        )
                        .child(if release.packages.is_empty() || review.busy {
                            package.opacity(0.45).cursor_default()
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
                    body = body.child(selector);
                    if release.packages.is_empty() {
                        body = body.child(warning("The author has not published a packaged ZIP release. Source-code archives cannot be installed as addons.".into()));
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
                            .gap(px(9.))
                            .child({
                                let view = palette
                                    .button("community-github", "View project")
                                    .child(crate::icon("external", 13., palette.muted));
                                if review.busy {
                                    view.opacity(0.45).cursor_default()
                                } else {
                                    crate::keyboard_button(view, palette, move |_, _, cx| {
                                        cx.open_url(&url)
                                    })
                                }
                            })
                            .child(if review.busy || selected.is_none() {
                                inspect.opacity(0.45).cursor_default()
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
                        .child(
                            div().mt(px(23.)).child(crate::keyboard_button(
                                palette
                                    .text_button("community-release-notes", "Release notes")
                                    .flex()
                                    .flex_row_reverse()
                                    .items_center()
                                    .gap(px(3.))
                                    .py(px(5.))
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.accent))
                                    .child(crate::icon(
                                        if review.show_notes {
                                            "chevron"
                                        } else {
                                            "chevron-right"
                                        },
                                        12.,
                                        palette.accent,
                                    )),
                                palette,
                                cx.listener(|app, _: &(), _, cx| {
                                    if let Some(review) = &mut app.project_review {
                                        review.show_notes = !review.show_notes;
                                        cx.notify();
                                    }
                                }),
                            )),
                        )
                        .when(review.show_notes, |body| {
                            body.child(
                                palette
                                    .label(release.notes.clone())
                                    .my(px(11.))
                                    .bg(rgb(palette.surface))
                                    .text_size(px(11.))
                                    .line_height(px(19.8)),
                            )
                        });
                }
            }
        }
        let close = palette
            .icon_button("close-community-review", "close")
            .track_focus(&review.close_focus)
            .absolute()
            .top(px(14.))
            .right(px(14.));
        let close = if review.busy {
            close.opacity(0.45).cursor_default()
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
                    .top_0()
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
                    .relative()
                    .overflow_y_scroll()
                    .w(px(460.))
                    .max_w(gpui::relative(0.90))
                    .max_h(gpui::relative(0.85))
                    .p(px(30.))
                    .flex()
                    .flex_col()
                    .occlude()
                    .bg(rgb(palette.panel))
                    .rounded(px(14.))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::rgba(0x00000044).into(),
                        offset: gpui::point(px(0.), px(20.)),
                        blur_radius: px(35.),
                        spread_radius: px(0.),
                    }])
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
                            .flex_shrink_0()
                            .mt(px(20.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(crate::typography::wrapped(
                                review
                                    .repo
                                    .split('/')
                                    .next_back()
                                    .unwrap_or(&review.repo)
                                    .to_owned(),
                                24.,
                                1.2,
                                -0.7,
                            )),
                    )
                    .child(body),
            )
    }
}

fn package_label(asset: &releases::Package) -> String {
    format!("{} · {:.1} MB", asset.name, asset.size as f64 / 1_048_576.)
}
