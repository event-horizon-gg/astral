use crate::{Addon, Astral, Client, Palette, installer, review::Receipt};
use gpui::{Context, Window, div, prelude::*, px, rgb};
use std::path::PathBuf;

pub struct Removal {
    root: PathBuf,
    client: Client,
    addon: Addon,
    running: bool,
    error: Option<String>,
}

impl Astral {
    pub fn confirm_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.addon_batch_running() {
            return;
        }
        let client = self.preferences.client;
        let Some(root) = self.root(client).cloned() else {
            return;
        };
        let Some(addon) = self
            .addons
            .iter()
            .find(|addon| Some(&addon.folder) == self.selected.as_ref())
        else {
            return;
        };
        self.removal = Some(Removal {
            root,
            client,
            addon: addon.clone(),
            running: false,
            error: None,
        });
        window.focus(&self.root_focus);
        cx.notify();
    }

    pub fn dismiss_removal(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(removal) = &self.removal else {
            return false;
        };
        if !removal.running {
            self.removal = None;
            cx.notify();
        }
        true
    }

    fn uninstall(&mut self, cx: &mut Context<Self>) {
        let Some(removal) = self.removal.as_mut().filter(|removal| !removal.running) else {
            return;
        };
        removal.running = true;
        removal.error = None;
        let (root, client, addon) = (removal.root.clone(), removal.client, removal.addon.clone());
        let name = addon.name.clone();
        let catalog = self.catalog.clone();
        let preferences = self.preferences_path.clone();
        let task = cx.background_executor().spawn(async move {
            let result = installer::remove(&root, client, &addon, &catalog);
            crate::activity::record(
                preferences.as_deref(),
                crate::activity::Action {
                    root,
                    client,
                    name: addon.name,
                    retry_source: None,
                    from: Some(addon.version),
                    to: None,
                    verb: "Uninstalled".into(),
                    error: result.as_ref().err().cloned(),
                    time: 0,
                },
            );
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(backup) => {
                        app.removal = None;
                        app.install_receipt = Some(Receipt {
                            client,
                            message: format!("Uninstalled {name}"),
                            backup,
                        });
                        app.refresh_after_change(cx);
                    }
                    Err(error) => {
                        if let Some(removal) = &mut app.removal {
                            removal.running = false;
                            removal.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn removal_dialog(
        &self,
        removal: &Removal,
        palette: Palette,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let actions =
            div()
                .flex()
                .justify_end()
                .gap_2()
                .when(!removal.running, |actions| {
                    actions.child(palette.button("cancel-uninstall", "Cancel").on_click(
                        cx.listener(|app, _, _, cx| {
                            app.dismiss_removal(cx);
                        }),
                    ))
                })
                .child(if removal.running {
                    palette
                        .primary_button("confirm-uninstall", "Uninstalling…")
                        .opacity(0.6)
                        .cursor_default()
                } else {
                    palette
                        .primary_button("confirm-uninstall", "Uninstall addon")
                        .on_click(cx.listener(|app, _, _, cx| app.uninstall(cx)))
                });
        div().absolute().top_0().left_0().size_full().flex().items_center().justify_center()
            .child(div().absolute().top_0().left_0().size_full().bg(gpui::rgba(0x00000088)).occlude())
            .child(div().w(px(480.)).max_w(gpui::relative(0.92)).max_h(gpui::relative(0.92))
                .p_6().flex().flex_col().gap_4().occlude().bg(rgb(palette.panel)).rounded(px(10.))
                .border_1().border_color(palette.line)
                .child(div().text_size(px(25.)).font_weight(gpui::FontWeight::BOLD)
                    .child(format!("Uninstall {}?", removal.addon.name)))
                .child(palette.label("Close WoW to continue. Your settings stay in WoW, and you can restore the addon from Backups."))
                .child(div().id("uninstall-folders").min_h_0().max_h(px(110.)).overflow_y_scroll()
                    .children(removal.addon.modules.iter().map(|folder| palette.label(folder.clone()))))
                .when_some(removal.error.as_ref(), |body, error| body.child(
                    palette.label(error.clone()).text_color(rgb(palette.error))
                ))
                .child(actions))
    }
}
