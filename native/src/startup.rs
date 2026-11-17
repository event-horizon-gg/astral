use crate::{Astral, Palette};
use gpui::{Context, div, prelude::*, rgb};
use std::io;

pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[derive(Default)]
pub struct State {
    changing: bool,
    error: Option<String>,
}

#[cfg(target_os = "macos")]
fn login_agent(bundle: &str) -> String {
    let bundle = crate::schedule::xml_text(bundle);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.joaoseixas.astral-native.login</string>
<key>RunAtLoad</key><true/>
<key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-g</string><string>-a</string><string>{bundle}</string><string>--args</string><string>--background</string></array>
</dict></plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn configure(enabled: bool) -> io::Result<()> {
    use std::{env, fs, io::Write, path::PathBuf};
    let home = env::var_os("HOME").ok_or_else(|| io::Error::other("Home folder unavailable."))?;
    let directory = PathBuf::from(home).join("Library/LaunchAgents");
    let file = directory.join("com.joaoseixas.astral-native.login.plist");
    if !enabled {
        return match fs::remove_file(file) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    let executable = env::current_exe()?;
    let bundle = executable
        .ancestors()
        .nth(3)
        .filter(|bundle| {
            bundle
                .extension()
                .is_some_and(|extension| extension == "app")
                && executable.strip_prefix(bundle).is_ok_and(|relative| {
                    relative == std::path::Path::new("Contents/MacOS/astral-native")
                })
        })
        .and_then(|bundle| bundle.to_str())
        .ok_or_else(|| {
            io::Error::other("Open Astral from its application bundle before enabling startup.")
        })?;
    fs::create_dir_all(&directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(login_agent(bundle).as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(file)?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn windows_command(executable: &str) -> io::Result<String> {
    if executable.is_empty() || executable.contains('"') || executable.chars().any(char::is_control)
    {
        return Err(io::Error::other("Invalid Astral path."));
    }
    Ok(format!("\"{executable}\" --background"))
}

#[cfg(target_os = "windows")]
fn configure(enabled: bool) -> io::Result<()> {
    use std::{env, os::windows::process::CommandExt, path::PathBuf, process::Command};
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Astral Native";
    let system = env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("Windows system folder unavailable."))?;
    let reg = || {
        let mut command = Command::new(PathBuf::from(&system).join("System32/reg.exe"));
        command.creation_flags(0x0800_0000);
        command
    };
    let output = if enabled {
        let executable = env::current_exe()?;
        let value = windows_command(
            executable
                .to_str()
                .ok_or_else(|| io::Error::other("Invalid Astral path."))?,
        )?;
        reg()
            .args(["add", KEY, "/v", NAME, "/t", "REG_SZ", "/d", &value, "/f"])
            .output()?
    } else {
        reg().args(["delete", KEY, "/v", NAME, "/f"]).output()?
    };
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            "Windows couldn't change the Astral startup setting.",
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn configure(_: bool) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

impl Astral {
    fn set_startup(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.startup.changing || self.preferences_path.is_none() {
            return;
        }
        self.startup.changing = true;
        self.startup.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { configure(enabled) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.startup.changing = false;
                match result {
                    Ok(()) => {
                        app.preferences.start_at_login = enabled;
                        app.save_preferences();
                        if app.preferences_error.is_some() {
                            app.preferences.start_at_login = !enabled;
                        }
                    }
                    Err(error) => app.startup.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn startup_settings(&self, palette: Palette, cx: &Context<Self>) -> gpui::Div {
        let enabled = self.preferences.start_at_login;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(palette.setting_checkbox(
                "startup-toggle",
                "Start Astral when I sign in",
                enabled,
                !self.fixture && !self.startup.changing && self.preferences_path.is_some(),
                |app, cx| app.set_startup(!app.preferences.start_at_login, cx),
                cx,
            ))
            .when_some(self.startup.error.as_ref(), |panel, error| {
                panel.child(palette.label(error.clone()).text_color(rgb(palette.error)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_relaunch_requests_the_existing_window() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path();
        let instance = Instance::in_directory(path, true)?.unwrap();
        assert!(Instance::in_directory(path, true)?.is_none());
        assert!(!path.join("show-window").exists());
        assert!(Instance::in_directory(path, false)?.is_none());
        assert!(path.join("show-window").is_file());
        let duplicate = instance.lock.as_ref().unwrap().try_clone()?;
        drop(instance);
        assert!(Instance::in_directory(path, false)?.is_some());
        drop(duplicate);
        Ok(())
    }

    #[test]
    fn windows_startup_quotes_the_executable_and_leaves_the_argument_separate() {
        let path = r"C:\Users\João\Astral Native\astral-native.exe";
        assert_eq!(
            windows_command(path).unwrap(),
            format!("\"{path}\" --background")
        );
        assert!(windows_command("bad\"path").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_login_agent_uses_a_bundle_and_background_argument() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let xml = login_agent("/Applications/Astral & Friends.app");
        assert!(xml.contains("Astral &amp; Friends.app"));
        assert!(xml.contains("<string>--background</string>"));
        file.write_all(xml.as_bytes()).unwrap();
        assert!(
            std::process::Command::new("/usr/bin/plutil")
                .arg("-lint")
                .arg(file.path())
                .status()
                .unwrap()
                .success()
        );
    }
}

#[derive(Default)]
pub struct Instance {
    lock: Option<std::fs::File>,
    show_request: Option<std::path::PathBuf>,
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Release the lock even when a concurrent process launch still holds a copied descriptor.
        if let Some(lock) = &self.lock {
            let _ = lock.unlock();
        }
    }
}

impl Instance {
    pub fn open(background: bool) -> io::Result<Option<Self>> {
        let Some(preferences) = crate::preferences::path() else {
            return Ok(Some(Self::default()));
        };
        let folder = preferences.parent().ok_or(io::ErrorKind::InvalidInput)?;
        Self::in_directory(folder, background)
    }

    fn in_directory(folder: &std::path::Path, background: bool) -> io::Result<Option<Self>> {
        use std::fs;
        fs::create_dir_all(folder)?;
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(folder.join("desktop.lock"))?;
        let show_request = folder.join("show-window");
        match lock.try_lock().map_err(io::Error::from) {
            Ok(()) => Ok(Some(Self {
                lock: Some(lock),
                show_request: Some(show_request),
            })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if !background {
                    fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(show_request)?;
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn watch(self, window: gpui::WindowHandle<Astral>, cx: &mut gpui::App) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let instance = self;
            loop {
                if instance
                    .show_request
                    .as_ref()
                    .is_some_and(|path| std::fs::remove_file(path).is_ok())
                {
                    let result = cx.update(|cx| {
                        let result = window.update(cx, |app, window, cx| {
                            crate::tray::show(window, cx);
                            app.root_focus.focus(window);
                        });
                        if result.is_ok() {
                            cx.activate(true);
                        }
                        result
                    });
                    if !matches!(result, Ok(Ok(()))) {
                        break;
                    }
                }
                executor.timer(std::time::Duration::from_millis(300)).await;
                if !matches!(cx.update(|cx| window.update(cx, |_, _, _| ())), Ok(Ok(()))) {
                    break;
                }
            }
        })
        .detach();
    }
}
