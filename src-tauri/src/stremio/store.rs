//! On-disk persistence for Stremio addons and debrid service credentials.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::debrid::{DebridKind, DebridService, DebridServiceFile};
use super::{StremioAddon, StremioAddonStatus, StremioResource};

const FILE_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AddonFile {
    #[serde(default = "default_version")]
    pub version: u32,
    pub addons: Vec<StremioAddon>,
}

fn default_version() -> u32 {
    FILE_VERSION
}

/// In-process cache of the on-disk addon file.
pub struct AddonStore {
    pub file: PathBuf,
    pub inner: Mutex<AddonFile>,
}

impl AddonStore {
    pub fn load_or_init() -> Self {
        let path = super::addon_file_path();
        let inner = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<AddonFile>(&s).unwrap_or_default(),
            Err(_) => AddonFile::default(),
        };
        Self {
            file: path,
            inner: Mutex::new(inner),
        }
    }

    pub fn list(&self) -> Vec<StremioAddon> {
        self.inner.lock().unwrap().addons.clone()
    }

    pub fn upsert(&self, addon: StremioAddon) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(existing) = guard.addons.iter_mut().find(|a| a.id == addon.id) {
            *existing = addon;
        } else {
            guard.addons.push(addon);
        }
        Self::write_locked(&self.file, &guard)
    }

    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let mut guard = self.inner.lock().unwrap();
        let before = guard.addons.len();
        guard.addons.retain(|a| a.id != id);
        let changed = guard.addons.len() != before;
        if changed {
            Self::write_locked(&self.file, &guard)?;
        }
        Ok(changed)
    }

    pub fn set_status(&self, id: &str, status: StremioAddonStatus) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(a) = guard.addons.iter_mut().find(|a| a.id == id) {
            a.status = status;
            a.last_checked_at = chrono::Utc::now();
            Self::write_locked(&self.file, &guard)
        } else {
            Ok(())
        }
    }

    pub fn find(&self, id: &str) -> Option<StremioAddon> {
        self.inner
            .lock()
            .unwrap()
            .addons
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }

    fn write_locked(path: &Path, data: &AddonFile) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
        {
            let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// In-process cache of the debrid service file.
pub struct DebridStore {
    file: PathBuf,
    inner: Mutex<DebridServiceFile>,
}

impl DebridStore {
    pub fn load_or_init() -> Self {
        let path = super::debrid_file_path();
        let inner = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<DebridServiceFile>(&s).unwrap_or_else(|_| DebridServiceFile {
                version: FILE_VERSION,
                services: Vec::new(),
            }),
            Err(_) => DebridServiceFile {
                version: FILE_VERSION,
                services: Vec::new(),
            },
        };
        Self {
            file: path,
            inner: Mutex::new(inner),
        }
    }

    pub fn list(&self) -> Vec<DebridService> {
        self.inner.lock().unwrap().services.clone()
    }

    pub fn upsert(&self, service: DebridService) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(existing) = guard.services.iter_mut().find(|s| s.kind == service.kind) {
            *existing = service;
        } else {
            guard.services.push(service);
        }
        Self::write_locked(&self.file, &guard)
    }

    pub fn remove(&self, kind: DebridKind) -> Result<bool, String> {
        let mut guard = self.inner.lock().unwrap();
        let before = guard.services.len();
        guard.services.retain(|s| s.kind != kind);
        let changed = guard.services.len() != before;
        if changed {
            Self::write_locked(&self.file, &guard)?;
        }
        Ok(changed)
    }

    pub fn set_default(&self, kind: DebridKind) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();
        for s in guard.services.iter_mut() {
            s.is_default = s.kind == kind;
        }
        Self::write_locked(&self.file, &guard)
    }

    pub fn default_service(&self) -> Option<DebridService> {
        self.inner
            .lock()
            .unwrap()
            .services
            .iter()
            .find(|s| s.is_default)
            .cloned()
    }

    #[allow(dead_code)]
    pub fn find(&self, kind: DebridKind) -> Option<DebridService> {
        self.inner
            .lock()
            .unwrap()
            .services
            .iter()
            .find(|s| s.kind == kind)
            .cloned()
    }

    fn write_locked(path: &Path, data: &DebridServiceFile) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
        {
            let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Sentinel for `find` callers; placeholder for resources-type filter
/// expansion (currently unused but kept for future test/inspection helpers).
#[allow(dead_code)]
pub fn addon_supports(addon: &StremioAddon, resource: &str) -> bool {
    addon.resources.iter().any(|r| r.name() == resource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stremio::debrid::DebridKind;

    fn tmp_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        let n: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        p.push(format!("slasshy-stremio-test-{}.json", n));
        p
    }

    #[test]
    fn addon_store_round_trip() {
        let path = tmp_path();
        let s = AddonStore {
            file: path.clone(),
            inner: Mutex::new(AddonFile::default()),
        };
        let a = StremioAddon {
            id: "x".to_string(),
            url: "https://x/manifest.json".to_string(),
            name: "X".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            logo: None,
            types: vec!["movie".to_string()],
            resources: vec![StremioResource::Plain("stream".to_string())],
            id_prefixes: vec!["tt".to_string()],
            catalogs: vec![],
            config: None,
            status: StremioAddonStatus::Available,
            installed_at: chrono::Utc::now(),
            last_checked_at: chrono::Utc::now(),
        };
        s.upsert(a.clone()).unwrap();
        let listed = s.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "x");
        assert!(s.remove("x").unwrap());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn debrid_store_round_trip() {
        let path = tmp_path();
        let s = DebridStore {
            file: path.clone(),
            inner: Mutex::new(DebridServiceFile {
                version: FILE_VERSION,
                services: Vec::new(),
            }),
        };
        s.upsert(DebridService {
            kind: DebridKind::RealDebrid,
            username: "alice".to_string(),
            api_key: "secret".to_string(),
            is_default: true,
        })
        .unwrap();
        s.set_default(DebridKind::RealDebrid).unwrap();
        let def = s.default_service().unwrap();
        assert_eq!(def.username, "alice");
        assert!(def.is_default);
        let _ = std::fs::remove_file(&path);
    }
}
