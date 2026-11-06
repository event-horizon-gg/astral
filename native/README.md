# Astral Native

Experimental GPUI desktop client on the `rust-gpui` branch. The Electron app remains the reference for behavior. The native client reads the existing WoW installation and uses public release metadata. It has no access to the Electron app's credentials.

Run with Rust and Xcode on macOS:

```sh
cargo run --locked --manifest-path native/Cargo.toml
```

The Library detects the standard Retail and Forever folders. Choose WoW folder can select a custom installation for the current session. Check for updates compares installed addons with packaged releases from their connected GitHub repositories. Selecting a row shows its modules and any release check error.

`cargo run --locked --manifest-path native/Cargo.toml -- --scan` emits the detected inventories as JSON lines. On macOS, `--check keyui` runs one release check against the detected installation.

The addon installer, backups, profile switching, Battle.net sign-in, sync, scheduling, and signed desktop updates still need to be ported. The Cloudflare API and in-game Lua addons are shared compatibility contracts for this migration. Windows packaging and runtime are unverified.
