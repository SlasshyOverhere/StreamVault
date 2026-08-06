//! Stremio addon protocol support.
//!
//! This module fetches and persists Stremio addon manifests, brokers catalog /
//! meta / stream requests against addon servers, and resolves `infoHash`-only
//! streams through user-configured debrid services.

pub mod catalog;
pub mod configure;
pub mod debrid;
pub mod health;
pub mod manifest;
pub mod meta;
pub mod store;
pub mod stream;

#[cfg(test)]
mod flix_test;
#[cfg(test)]
mod integration_test;
#[cfg(test)]
mod store_test;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use debrid::{DebridKind, DebridService};
use store::{AddonStore, DebridStore};

/// Status of an installed Stremio addon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StremioAddonStatus {
    Available,
    Unavailable,
    /// Addon requires user configuration; install is not yet complete.
    ConfigRequired,
}

/// A single Stremio resource entry (catalog, meta, stream, or subtitles).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StremioResource {
    /// Plain string resource: "catalog", "stream", "subtitles".
    Plain(String),
    /// Object resource with type filtering: meta, stream, subtitles.
    /// We accept any extra fields by using a custom deserializer that
    /// pulls only the ones we know about.
    Typed(StremioTypedResource),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StremioTypedResource {
    pub name: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Vec<String>,
}

impl StremioResource {
    pub fn name(&self) -> &str {
        match self {
            StremioResource::Plain(n) => n.as_str(),
            StremioResource::Typed(t) => t.name.as_str(),
        }
    }
}

/// A catalog entry in a Stremio addon manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StremioCatalog {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: Option<String>,
}

/// Persisted Stremio addon record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StremioAddon {
    pub id: String,
    pub url: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    pub types: Vec<String>,
    pub resources: Vec<StremioResource>,
    #[serde(default, rename = "idPrefixes")]
    pub id_prefixes: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<StremioCatalog>,
    /// Opaque configuration string captured from `/configure` flows.
    #[serde(default)]
    pub config: Option<String>,
    pub status: StremioAddonStatus,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub last_checked_at: chrono::DateTime<chrono::Utc>,
}

// ─── Storage paths ─────────────────────────────────────────────────────────

pub(crate) fn addon_file_path() -> PathBuf {
    crate::database::get_app_data_dir().join("stremio_addons.json")
}

pub(crate) fn debrid_file_path() -> PathBuf {
    crate::database::get_app_data_dir().join("debrid_services.json")
}

// ─── Lazy singletons ───────────────────────────────────────────────────────

static ADDON_STORE: OnceLock<AddonStore> = OnceLock::new();
static DEBRID_STORE: OnceLock<DebridStore> = OnceLock::new();

fn addons() -> &'static AddonStore {
    ADDON_STORE.get_or_init(AddonStore::load_or_init)
}

fn debrids() -> &'static DebridStore {
    DEBRID_STORE.get_or_init(DebridStore::load_or_init)
}

// ─── Tauri commands ────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, Serialize)]
pub enum StremioError {
    #[error("{0}")]
    Generic(String),
}

impl From<String> for StremioError {
    fn from(s: String) -> Self {
        StremioError::Generic(s)
    }
}
impl From<&str> for StremioError {
    fn from(s: &str) -> Self {
        StremioError::Generic(s.to_string())
    }
}

#[tauri::command]
pub fn stremio_add_addon(
    url: String,
    config: Option<String>,
) -> Result<StremioAddon, StremioError> {
    let mut addon = configure::resolve(&url, config).map_err(|e| e.to_string())?;
    addon.last_checked_at = chrono::Utc::now();
    addons().upsert(addon.clone())?;
    Ok(addon)
}

#[tauri::command]
pub fn stremio_remove_addon(id: String) -> Result<bool, StremioError> {
    addons().remove(&id).map_err(Into::into)
}

#[tauri::command]
pub fn stremio_list_addons() -> Result<Vec<StremioAddon>, StremioError> {
    let path = addon_file_path();
    eprintln!("[stremio_list_addons] file path: {:?}", path);
    eprintln!("[stremio_list_addons] file exists: {}", path.exists());
    if let Ok(contents) = std::fs::read_to_string(&path) {
        eprintln!("[stremio_list_addons] file length: {}", contents.len());
    } else {
        eprintln!("[stremio_list_addons] could not read file");
    }
    let mut list = addons().list();
    eprintln!("[stremio_list_addons] store has {} addons", list.len());
    for a in list.iter_mut() {
        eprintln!("[stremio_list_addons] addon: id={}", a.id);
        if chrono::Utc::now()
            .signed_duration_since(a.last_checked_at)
            .num_seconds()
            > 60
        {
            let s = health::probe(&a.url);
            a.status = s;
            a.last_checked_at = chrono::Utc::now();
            if let Err(e) = addons().set_status(&a.id, s) {
                eprintln!("[stremio_list_addons] set_status failed: {}", e);
            }
        }
    }
    eprintln!(
        "[stremio_list_addons] returning {} addons to frontend",
        list.len()
    );
    Ok(list)
}

#[tauri::command]
pub fn stremio_fetch_catalog(
    addon_id: String,
    kind: String,
    catalog_id: String,
    extra: Option<HashMap<String, String>>,
) -> Result<catalog::CatalogResponse, StremioError> {
    let addon = addons()
        .find(&addon_id)
        .ok_or_else(|| StremioError::Generic("addon not found".to_string()))?;
    catalog::fetch(
        &addon,
        &kind,
        &catalog_id,
        extra.as_ref().unwrap_or(&HashMap::new()),
    )
    .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub fn stremio_fetch_meta(
    addon_id: String,
    kind: String,
    id: String,
) -> Result<meta::MetaResponse, StremioError> {
    let addon = addons()
        .find(&addon_id)
        .ok_or_else(|| StremioError::Generic("addon not found".to_string()))?;
    meta::fetch(&addon, &kind, &id).map_err(|e| e.to_string().into())
}

#[tauri::command]
pub fn stremio_fetch_streams(
    addon_id: String,
    kind: String,
    id: String,
) -> Result<stream::StreamsResponse, StremioError> {
    let addon = addons()
        .find(&addon_id)
        .ok_or_else(|| StremioError::Generic("addon not found".to_string()))?;
    stream::fetch(&addon, &kind, &id).map_err(|e| e.to_string().into())
}

#[tauri::command]
pub fn stremio_resolve_stream(stream: debrid::StremioStreamLike) -> Result<String, StremioError> {
    let default = debrids().default_service();
    debrid::resolve_stream(default.as_ref(), &stream).map_err(|e| e.to_string().into())
}

#[tauri::command]
pub fn debrid_add_service(kind: String, api_key: String) -> Result<DebridService, StremioError> {
    let kind_enum = parse_kind(&kind)?;
    let username = debrid::validate_key(kind_enum, &api_key).map_err(|e| e.to_string())?;
    let service = DebridService {
        kind: kind_enum,
        username,
        api_key,
        is_default: debrids().list().is_empty(),
    };
    debrids().upsert(service.clone())?;
    Ok(service)
}

#[tauri::command]
pub fn debrid_remove_service(kind: String) -> Result<bool, StremioError> {
    let kind_enum = parse_kind(&kind)?;
    debrids().remove(kind_enum).map_err(Into::into)
}

#[tauri::command]
pub fn debrid_list_services() -> Result<Vec<DebridService>, StremioError> {
    Ok(debrids().list())
}

#[tauri::command]
pub fn debrid_set_default(kind: String) -> Result<(), StremioError> {
    let kind_enum = parse_kind(&kind)?;
    debrids().set_default(kind_enum).map_err(Into::into)
}

fn parse_kind(s: &str) -> Result<DebridKind, StremioError> {
    match s {
        "real_debrid" => Ok(DebridKind::RealDebrid),
        "all_debrid" => Ok(DebridKind::AllDebrid),
        "premiumize" => Ok(DebridKind::Premiumize),
        "torbox" => Ok(DebridKind::TorBox),
        "offcloud" => Ok(DebridKind::Offcloud),
        "easydebrid" => Ok(DebridKind::EasyDebrid),
        "linksnappy" => Ok(DebridKind::LinkSnappy),
        "mega_debrid" => Ok(DebridKind::MegaDebrid),
        other => Err(StremioError::Generic(format!(
            "unknown debrid kind: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_known() {
        assert_eq!(parse_kind("real_debrid").unwrap(), DebridKind::RealDebrid);
        assert_eq!(parse_kind("all_debrid").unwrap(), DebridKind::AllDebrid);
        assert_eq!(parse_kind("premiumize").unwrap(), DebridKind::Premiumize);
    }

    #[test]
    fn parse_kind_unknown() {
        assert!(parse_kind("nope").is_err());
    }
}
