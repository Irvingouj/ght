use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::github::Account;
use crate::model::{Pr, PrDetail};

/// What ght last saw for one account, shown at once on the next start while a refresh runs.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub saved_at: DateTime<Utc>,
    pub prs: Vec<Pr>,
    pub details: Vec<CachedDetail>,
}

#[derive(Serialize, Deserialize)]
pub struct CachedDetail {
    pub repo: String,
    pub number: u64,
    /// The PR's `updatedAt` when this detail was fetched; a newer one means it is stale.
    pub updated_at: DateTime<Utc>,
    pub detail: PrDetail,
}

pub struct Store {
    dir: Option<PathBuf>,
}

impl Store {
    /// `GHT_CACHE_DIR` wins so tests never touch the real cache.
    pub fn new() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let dir = var("GHT_CACHE_DIR")
            .or_else(|| var("LOCALAPPDATA").map(|d| d.join("ght")))
            .or_else(|| var("XDG_CACHE_HOME").map(|d| d.join("ght")))
            .or_else(|| var("HOME").map(|d| d.join(".cache").join("ght")));
        Self { dir }
    }

    #[cfg(test)]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    pub fn accounts(&self) -> Option<Vec<Account>> {
        self.read("accounts.json")
    }

    pub fn save_accounts(&self, accounts: &[Account]) {
        self.write("accounts.json", &accounts);
    }

    pub fn snapshot(&self, account: &str) -> Option<Snapshot> {
        self.read(&format!("prs-{account}.json"))
    }

    pub fn save_snapshot(&self, account: &str, snapshot: &Snapshot) {
        self.write(&format!("prs-{account}.json"), snapshot);
    }

    fn read<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let text = std::fs::read_to_string(self.dir.as_ref()?.join(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Best effort: a cache that cannot be written only costs the next start its head start.
    fn write<T: Serialize + ?Sized>(&self, name: &str, value: &T) {
        let Some(dir) = &self.dir else { return };
        let Ok(text) = serde_json::to_string(value) else { return };
        let tmp = dir.join(format!("{name}.tmp"));
        if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn snapshot_and_accounts_round_trip() {
        let dir = std::env::temp_dir().join(format!("ght-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::at(dir.clone());
        assert!(store.snapshot("work").is_none());

        store.save_accounts(&[Account { login: "work".into(), active: true }]);
        let detail = testutil::detail();
        let snap = Snapshot {
            saved_at: Utc::now(),
            prs: testutil::prs(),
            details: vec![CachedDetail { repo: "o/alpha".into(), number: 12, updated_at: Utc::now(), detail }],
        };
        store.save_snapshot("work", &snap);

        assert_eq!(store.accounts().unwrap()[0].login, "work");
        let back = store.snapshot("work").unwrap();
        assert_eq!(back.prs.len(), 4);
        assert_eq!(back.details[0].detail.files[0].lines, testutil::detail().files[0].lines);
        assert!(store.snapshot("home").is_none(), "one file per account");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
