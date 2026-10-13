//! Secrets in the OS keychain: service "TableSoft", account "<connection id>:<kind>".

use dbcore::model::{ConnectionConfig, Secrets, SshAuth};
use dbcore::{Engine, Error};

const SERVICE: &str = "TableSoft";

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Db,
    Ssh,
    SshPassphrase,
    Jump,
    JumpPassphrase,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Db,
        Kind::Ssh,
        Kind::SshPassphrase,
        Kind::Jump,
        Kind::JumpPassphrase,
    ];

    fn name(self) -> &'static str {
        match self {
            Kind::Db => "db",
            Kind::Ssh => "ssh",
            Kind::SshPassphrase => "ssh-passphrase",
            Kind::Jump => "jump",
            Kind::JumpPassphrase => "jump-passphrase",
        }
    }
}

fn keychain_err(e: keyring::Error) -> Error {
    Error::Other(format!("OS keychain: {e}"))
}

fn entry(id: &str, kind: Kind) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("{id}:{}", kind.name())).map_err(keychain_err)
}

fn read(id: &str, kind: Kind) -> Result<Option<String>> {
    match entry(id, kind)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_err(e)),
    }
}

fn write(id: &str, kind: Kind, value: &str) -> Result<()> {
    entry(id, kind)?.set_password(value).map_err(keychain_err)
}

fn remove(id: &str, kind: Kind) -> Result<()> {
    match entry(id, kind)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(keychain_err(e)),
    }
}

/// Which secrets a connection can use; reading only those keeps the OS from
/// prompting for keychain items that are irrelevant.
fn needed(config: &ConnectionConfig) -> Vec<Kind> {
    let mut kinds = Vec::new();
    if config.engine != Engine::Sqlite {
        kinds.push(Kind::Db);
    }
    if let Some(ssh) = &config.ssh {
        match ssh.auth {
            SshAuth::Password => kinds.push(Kind::Ssh),
            SshAuth::Key { .. } => kinds.push(Kind::SshPassphrase),
            SshAuth::Agent => {}
        }
        if let Some(jump) = &ssh.jump {
            match jump.auth {
                SshAuth::Password => kinds.push(Kind::Jump),
                SshAuth::Key { .. } => kinds.push(Kind::JumpPassphrase),
                SshAuth::Agent => {}
            }
        }
    }
    kinds
}

fn field(secrets: &mut Secrets, kind: Kind) -> &mut Option<String> {
    match kind {
        Kind::Db => &mut secrets.db_password,
        Kind::Ssh => &mut secrets.ssh_password,
        Kind::SshPassphrase => &mut secrets.ssh_passphrase,
        Kind::Jump => &mut secrets.jump_password,
        Kind::JumpPassphrase => &mut secrets.jump_passphrase,
    }
}

/// Reads the secrets `config` can use. An unavailable keychain yields no secret
/// rather than an error, so passwordless connections still work; a wrong or
/// missing password then surfaces as the server's own authentication error.
pub async fn load(config: &ConnectionConfig) -> Secrets {
    if config.id.is_empty() {
        return Secrets::default();
    }
    let id = config.id.clone();
    let kinds = needed(config);
    tokio::task::spawn_blocking(move || {
        let mut secrets = Secrets::default();
        for kind in kinds {
            match read(&id, kind) {
                Ok(value) => *field(&mut secrets, kind) = value,
                Err(e) => eprintln!("[keychain] {e}"),
            }
        }
        secrets
    })
    .await
    .unwrap_or_default()
}

/// Applies a save: a `None` field keeps the stored secret, `Some("")` deletes it,
/// anything else replaces it.
pub async fn store(id: &str, secrets: &Secrets) -> Result<()> {
    let id = id.to_owned();
    let mut secrets = secrets.clone();
    tokio::task::spawn_blocking(move || {
        for kind in Kind::ALL {
            match field(&mut secrets, kind).take() {
                None => {}
                Some(value) if value.is_empty() => remove(&id, kind)?,
                Some(value) => write(&id, kind, &value)?,
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

/// Removes every secret of a connection (best effort).
pub async fn delete_all(id: &str) {
    let id = id.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        for kind in Kind::ALL {
            if let Err(e) = remove(&id, kind) {
                eprintln!("[keychain] {e}");
            }
        }
    })
    .await;
}

/// `provided` fields override the keychain; `Some("")` means "no password".
pub fn merge(stored: Secrets, provided: Option<Secrets>) -> Secrets {
    let Some(provided) = provided else {
        return stored;
    };
    let pick = |new: Option<String>, old: Option<String>| match new {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(v),
        None => old,
    };
    Secrets {
        db_password: pick(provided.db_password, stored.db_password),
        ssh_password: pick(provided.ssh_password, stored.ssh_password),
        ssh_passphrase: pick(provided.ssh_passphrase, stored.ssh_passphrase),
        jump_password: pick(provided.jump_password, stored.jump_password),
        jump_passphrase: pick(provided.jump_passphrase, stored.jump_passphrase),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provided_secrets_override_stored_ones() {
        let stored = Secrets {
            db_password: Some("old".into()),
            ssh_password: Some("ssh".into()),
            ..Default::default()
        };
        let provided = Secrets {
            db_password: Some("new".into()),
            ssh_password: Some(String::new()),
            ssh_passphrase: Some("pp".into()),
            ..Default::default()
        };
        let merged = merge(stored.clone(), Some(provided));
        assert_eq!(merged.db_password.as_deref(), Some("new"));
        assert_eq!(merged.ssh_password, None);
        assert_eq!(merged.ssh_passphrase.as_deref(), Some("pp"));
        assert_eq!(merge(stored.clone(), None), stored);
    }
}
