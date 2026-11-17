pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
mod mac {
    use std::{env, fs, io, io::Write, path::Path, process::Command};

    const LABEL: &str = "com.joaoseixas.astral-native.daily";

    fn plist(executable: &Path) -> io::Result<String> {
        let path = executable
            .to_str()
            .ok_or_else(|| io::Error::other("Invalid Astral path."))?;
        let escaped = super::xml_text(path);
        Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{escaped}</string><string>--daily-update</string></array>
<key>ProcessType</key><string>Background</string>
<key>RunAtLoad</key><true/>
<key>StartInterval</key><integer>900</integer>
</dict></plist>
"#
        ))
    }

    pub fn configure(enabled: bool) -> io::Result<()> {
        let home =
            env::var_os("HOME").ok_or_else(|| io::Error::other("Home folder unavailable."))?;
        let dir = Path::new(&home).join("Library/LaunchAgents");
        let file = dir.join(format!("{LABEL}.plist"));
        let uid = Command::new("/usr/bin/id").arg("-u").output()?;
        let uid = String::from_utf8_lossy(&uid.stdout);
        let uid = uid.trim();
        if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(io::Error::other(
                "Couldn't identify the macOS login session.",
            ));
        }
        let domain = format!("gui/{uid}");
        let target = format!("{domain}/{LABEL}");
        let loaded = || -> io::Result<bool> {
            Ok(Command::new("/bin/launchctl")
                .args(["print", &target])
                .output()?
                .status
                .success())
        };
        let stop = || -> io::Result<()> {
            if loaded()? {
                let output = Command::new("/bin/launchctl")
                    .args(["bootout", &target])
                    .output()?;
                if !output.status.success() {
                    return Err(io::Error::other("Couldn't stop the background updater."));
                }
            }
            Ok(())
        };
        if !enabled {
            stop()?;
            return match fs::remove_file(file) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
        let content = plist(&env::current_exe()?)?;
        if fs::read_to_string(&file).ok().as_deref() != Some(&content) {
            stop()?;
            fs::create_dir_all(&dir)?;
            let mut staged = tempfile::NamedTempFile::new_in(&dir)?;
            staged.write_all(content.as_bytes())?;
            staged.persist(&file)?;
        }
        if !loaded()? {
            let output = Command::new("/bin/launchctl")
                .args(["bootstrap", &domain])
                .arg(&file)
                .output()?;
            if !output.status.success() {
                return Err(io::Error::other(
                    "Couldn't start the background updater. Try again from Settings.",
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    #[test]
    fn launch_agent_plist_passes_macos_validation() -> io::Result<()> {
        let mut file = tempfile::NamedTempFile::new()?;
        let manifest = plist(Path::new(
            "/Applications/Astral & Friends.app/Contents/MacOS/astral-native",
        ))?;
        assert!(manifest.contains("Astral &amp; Friends.app"));
        file.write_all(manifest.as_bytes())?;
        let output = Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(file.path())
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub use mac::configure;

#[cfg(any(target_os = "windows", test))]
mod windows {
    use std::{io, path::Path};

    const ARGUMENT: &str = "--daily-update";

    fn task_command(executable: &Path) -> io::Result<String> {
        Ok(super::xml_text(
            executable
                .to_str()
                .filter(|path| !path.chars().any(char::is_control))
                .ok_or_else(|| io::Error::other("Invalid Astral path."))?,
        ))
    }

    fn start_boundary() -> io::Result<String> {
        time::OffsetDateTime::from_unix_timestamp(
            time::OffsetDateTime::now_utc().unix_timestamp() + 60,
        )
        .map_err(io::Error::other)?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(io::Error::other)
    }

    fn task_xml(command: &str, sid: &str, start: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <TimeTrigger>
      <Repetition><Interval>PT15M</Interval></Repetition>
      <StartBoundary>{start}</StartBoundary>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="User">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
  </Settings>
  <Actions Context="User">
    <Exec><Command>{command}</Command><Arguments>{ARGUMENT}</Arguments></Exec>
  </Actions>
</Task>
"#
        )
    }

    fn utf16_xml(xml: &str) -> Vec<u8> {
        std::iter::once(0xfeff)
            .chain(xml.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn xml_output(bytes: &[u8]) -> String {
        let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        if bytes.get(1) == Some(&0) {
            let words: Vec<_> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&words)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }

    fn user_sid(stdout: &[u8]) -> io::Result<&str> {
        // whoami /user /fo csv /nh ends the single record with the ASCII SID.
        std::str::from_utf8(
            stdout
                .rsplit(|byte| *byte == b',')
                .next()
                .unwrap_or_default(),
        )
        .ok()
        .map(|sid| sid.trim().trim_matches('"'))
        .filter(|sid| {
            sid.strip_prefix("S-1-").is_some_and(|parts| {
                !parts.is_empty()
                    && parts.split('-').all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
        })
        .ok_or_else(|| io::Error::other("Couldn't identify the Windows user."))
    }

    #[cfg(target_os = "windows")]
    pub fn configure(enabled: bool) -> io::Result<()> {
        use std::{
            env, io::Write, os::windows::process::CommandExt, path::PathBuf, process::Command,
        };

        // Prevent the console tools from opening a window on the desktop.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let system = env::var_os("SystemRoot")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("Windows system folder unavailable."))?
            .join("System32");
        let command = |name: &str| {
            let mut command = Command::new(system.join(name));
            command.creation_flags(CREATE_NO_WINDOW);
            command
        };
        let user = command("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()?;
        if !user.status.success() {
            return Err(io::Error::other("Couldn't identify the Windows user."));
        }
        let sid = user_sid(&user.stdout)?;
        let task_name = format!("AstralNativeDailyUpdates-{sid}");
        let existing = command("schtasks.exe")
            .args(["/Query", "/TN", &task_name, "/XML"])
            .output()?;
        if !enabled {
            if existing.status.success()
                && !command("schtasks.exe")
                    .args(["/Delete", "/TN", &task_name, "/F"])
                    .output()?
                    .status
                    .success()
            {
                return Err(io::Error::other(
                    "Couldn't remove the background update task.",
                ));
            }
            return Ok(());
        }

        let executable = task_command(&env::current_exe()?)?;
        if existing.status.success() {
            let current = xml_output(&existing.stdout);
            if current.contains(&format!("<Command>{executable}</Command>"))
                && current.contains(&format!("<Arguments>{ARGUMENT}</Arguments>"))
                && current.contains("<Interval>PT15M</Interval>")
                && current
                    .contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>")
                && current.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>")
                && !current.contains("<Enabled>false</Enabled>")
            {
                return Ok(());
            }
        }
        let xml = task_xml(&executable, sid, &start_boundary()?);
        let mut definition = tempfile::Builder::new().suffix(".xml").tempfile()?;
        definition.write_all(&utf16_xml(&xml))?;
        definition.flush()?;
        let result = command("schtasks.exe")
            .args(["/Create", "/TN", &task_name, "/F", "/XML"])
            .arg(definition.path())
            .output()?;
        if !result.status.success() {
            return Err(io::Error::other(
                "Windows couldn't register the background update task. Try again from Settings.",
            ));
        }
        Ok(())
    }

    #[test]
    fn windows_task_keeps_the_unicode_executable_separate_from_its_arguments() -> io::Result<()> {
        let executable = r"C:\Users\João\Astral & Friends\astral-native.exe";
        let sid = user_sid(b"\"DESKTOP\\user\",\"S-1-5-21-123-456-789-1001\"\r\n")?;
        let start = start_boundary()?;
        assert!(start.ends_with('Z'));
        let xml = task_xml(&task_command(Path::new(executable))?, sid, &start);
        assert_eq!(xml_output(&utf16_xml(&xml)), xml);
        assert!(xml.contains(
            "<Command>C:\\Users\\João\\Astral &amp; Friends\\astral-native.exe</Command>"
        ));
        assert!(xml.contains("<Arguments>--daily-update</Arguments>"));
        assert!(xml.contains("<UserId>S-1-5-21-123-456-789-1001</UserId>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub use windows::configure;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn configure(_: bool) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}
pub fn legacy_raider_io() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").is_some_and(|home| {
            std::path::Path::new(&home)
                .join("Library/LaunchAgents/com.joaoseixas.astral-raiderio.plist")
                .exists()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
