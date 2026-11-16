use super::*;
use crate::{
    Client,
    settings_backup::{Entry, Profile},
    sync_format::{Bundle, Encoded, LibraryAddon},
};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn snapshot(value: &str, catalog: &[crate::Definition]) -> Encoded {
    Bundle {
        schema: 1,
        files: vec![Entry::new("bindings-cache.wtf".into(), value.as_bytes())],
        addon_selection: None,
        library: None,
        input: None,
    }
    .encode(catalog)
    .unwrap()
}

fn read_snapshot(remote: RemoteSnapshot, expected: &Encoded, catalog: &[crate::Definition]) {
    assert_eq!(remote.hash.as_deref(), Some(expected.hash.as_str()));
    let decoded = Bundle::decode(
        remote.payload.as_deref().unwrap(),
        remote.hash.as_deref(),
        catalog,
    )
    .unwrap();
    assert_eq!(decoded.normalized_hash().unwrap(), expected.hash);
}

#[test]
fn native_client_exchanges_scoped_settings_and_backups_with_the_worker_over_http()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(node) = std::env::var_os("ASTRAL_TEST_ELECTRON_CODEC") else {
        return Ok(());
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/worker-server.mjs");
    let mut worker = Worker(
        Command::new(node)
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let mut line = String::new();
    BufReader::new(worker.0.stdout.take().unwrap()).read_line(&mut line)?;
    let started: Value = serde_json::from_str(&line)?;
    let service =
        Service::for_loopback(started["origin"].as_str().ok_or("Missing fixture origin")?);
    let session = |letter: char| Session {
        token: letter.to_string().repeat(43),
        name: "FixturePlayer#1234".into(),
    };
    let (first, second, outsider) = (session('a'), session('b'), session('c'));
    let catalog = crate::library::catalog();
    let original = snapshot("BIND 1 ACTIONBUTTON1\n", &catalog);
    let changed = snapshot("BIND 2 ACTIONBUTTON2\n", &catalog);

    for client in Client::ALL {
        let initial = service.profiles(&first, client)?;
        assert!(initial.iter().any(|p| p.id == "regular"));
        assert!(initial.iter().any(|p| p.id == "handheld"));
        assert_eq!(
            service
                .current_snapshot(&first, client, Profile::Regular)?
                .revision,
            0
        );
        assert_eq!(
            service
                .upload_snapshot(&first, client, Profile::Regular, 0, &original)?
                .revision,
            1
        );
        read_snapshot(
            service.current_snapshot(&second, client, Profile::Regular)?,
            &original,
            &catalog,
        );

        let addons = [LibraryAddon {
            folder: "HandyNotes".into(),
            name: "HandyNotes".into(),
        }];
        assert_eq!(
            service.save_library(&first, client, 0, &addons)?.revision,
            1
        );
        assert!(
            service.shared_library(&second, client)?.addons.as_deref() == Some(addons.as_slice())
        );
        assert_eq!(
            service
                .save_library(&second, client, 0, &addons)
                .err()
                .unwrap()
                .status,
            Some(409)
        );

        assert_eq!(
            service
                .upload_snapshot(&second, client, Profile::Regular, 1, &changed)?
                .revision,
            2
        );
        assert_eq!(
            service
                .upload_snapshot(&first, client, Profile::Regular, 1, &original)
                .err()
                .unwrap()
                .status,
            Some(409)
        );
        let history = service.overview(&first, client, Profile::Regular)?;
        assert_eq!(history.history.len(), 2);
        assert_eq!(history.devices.len(), 2);
        let previous = history
            .history
            .iter()
            .find(|point| point.revision == 1)
            .unwrap();
        let old = service.historical_snapshot(&second, client, Profile::Regular, &previous.id)?;
        assert_eq!(old.hash.as_deref(), Some(original.hash.as_str()));
        assert_eq!(
            service
                .promote_snapshot(
                    &first,
                    client,
                    Profile::Regular,
                    &previous.id,
                    2,
                    &changed.hash
                )
                .err()
                .unwrap()
                .status,
            Some(409)
        );
        assert_eq!(
            service
                .current_snapshot(&first, client, Profile::Regular)?
                .revision,
            2
        );
        assert_eq!(
            service
                .promote_snapshot(
                    &first,
                    client,
                    Profile::Regular,
                    &previous.id,
                    2,
                    &original.hash
                )?
                .revision,
            3
        );
        read_snapshot(
            service.current_snapshot(&second, client, Profile::Regular)?,
            &original,
            &catalog,
        );
        assert_eq!(
            service
                .promote_snapshot(
                    &first,
                    client,
                    Profile::Regular,
                    &previous.id,
                    2,
                    &original.hash
                )
                .err()
                .unwrap()
                .status,
            Some(409)
        );

        assert_eq!(
            service
                .current_snapshot(&second, client, Profile::Handheld)?
                .revision,
            0
        );
        assert_eq!(
            service
                .upload_snapshot(&second, client, Profile::Handheld, 0, &changed)?
                .revision,
            1
        );
        read_snapshot(
            service.current_snapshot(&first, client, Profile::Handheld)?,
            &changed,
            &catalog,
        );
        assert_eq!(
            service
                .current_snapshot(&first, client, Profile::Regular)?
                .revision,
            3
        );

        let custom = service
            .save_profile(&first, client, None, "Raid setup")?
            .profile()?;
        assert_eq!(
            service
                .save_profile(&second, client, Some(custom), "Dungeon setup")?
                .name,
            "Dungeon setup"
        );
        assert_eq!(
            service
                .upload_snapshot(&first, client, custom, 0, &original)?
                .revision,
            1
        );
        read_snapshot(
            service.current_snapshot(&second, client, custom)?,
            &original,
            &catalog,
        );
        assert_eq!(
            service
                .delete_profile(&outsider, client, custom)
                .err()
                .unwrap()
                .status,
            Some(404)
        );
        service.delete_profile(&first, client, custom)?;
        assert!(
            !service
                .profiles(&second, client)?
                .iter()
                .any(|p| p.id == custom.cloud_id())
        );
        assert_eq!(
            service
                .current_snapshot(&outsider, client, Profile::Regular)?
                .revision,
            0
        );
        assert!(service.shared_library(&outsider, client)?.addons.is_none());
        service.upload_snapshot(&outsider, client, Profile::Regular, 0, &changed)?;
        service.save_library(&outsider, client, 0, &addons)?;

        let mut revision = 3;
        for index in 0..6 {
            let value = snapshot(&format!("BIND {} ACTIONBUTTON1\n", index + 3), &catalog);
            revision = service
                .upload_snapshot(&first, client, Profile::Regular, revision, &value)?
                .revision;
        }
        let retained = service.overview(&second, client, Profile::Regular)?;
        assert_eq!(retained.history.len(), 5);
        assert_eq!(retained.history.iter().filter(|p| p.current).count(), 1);
        assert!(retained.history.iter().all(|p| p.revision >= revision - 4));
        assert_eq!(
            service
                .historical_snapshot(&first, client, Profile::Regular, &previous.id)
                .err()
                .unwrap()
                .status,
            Some(404)
        );
    }
    service.revoke_device(&first, &"b".repeat(43))?;
    assert_eq!(
        service
            .profiles(&second, Client::Retail)
            .err()
            .unwrap()
            .status,
        Some(401)
    );
    service.delete_account_data(&first)?;
    assert_eq!(
        service
            .profiles(&first, Client::Retail)
            .err()
            .unwrap()
            .status,
        Some(401)
    );
    for client in Client::ALL {
        let untouched = service.current_snapshot(&outsider, client, Profile::Regular)?;
        assert_eq!(untouched.revision, 1);
        read_snapshot(untouched, &changed, &catalog);
        assert_eq!(service.shared_library(&outsider, client)?.revision, 1);
    }
    Ok(())
}
