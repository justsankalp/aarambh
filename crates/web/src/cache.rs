//! IndexedDB cache: day files as last seen in the repo, the offline post
//! queue, and small bits of sync metadata. Values are JSON strings.

use std::cell::RefCell;
use std::rc::Rc;

use idb::{Database, DatabaseEvent as _, Factory, ObjectStoreParams, TransactionMode};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

const DB_NAME: &str = "aarambh";
const DB_VERSION: u32 = 1;

pub const FILES: &str = "files";
pub const QUEUE: &str = "queue";
pub const META: &str = "meta";
const STORES: [&str; 3] = [FILES, QUEUE, META];

pub const META_REPO: &str = "repo";
pub const META_EVENTS_ETAG: &str = "events_etag";

/// A repo file as last fetched.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CachedFile {
    pub sha: String,
    pub text: String,
}

thread_local! {
    static DB: RefCell<Option<Rc<Database>>> = const { RefCell::new(None) };
}

fn err(e: impl std::fmt::Display) -> String {
    format!("local storage: {e}")
}

async fn db() -> Result<Rc<Database>, String> {
    if let Some(db) = DB.with_borrow(Clone::clone) {
        return Ok(db);
    }
    let factory = Factory::new().map_err(err)?;
    let mut req = factory.open(DB_NAME, Some(DB_VERSION)).map_err(err)?;
    req.on_upgrade_needed(|event| {
        if let Ok(db) = event.database() {
            let have = db.store_names();
            for s in STORES {
                if !have.iter().any(|h| h == s) {
                    let _ = db.create_object_store(s, ObjectStoreParams::new());
                }
            }
        }
    });
    let db = Rc::new(req.await.map_err(err)?);
    DB.with_borrow_mut(|slot| *slot = Some(db.clone()));
    Ok(db)
}

pub async fn get(store: &str, key: &str) -> Result<Option<String>, String> {
    let db = db().await?;
    let tx = db
        .transaction(&[store], TransactionMode::ReadOnly)
        .map_err(err)?;
    let v = tx
        .object_store(store)
        .map_err(err)?
        .get(JsValue::from_str(key))
        .map_err(err)?
        .await
        .map_err(err)?;
    Ok(v.and_then(|v| v.as_string()))
}

pub async fn all(store: &str) -> Result<Vec<String>, String> {
    let db = db().await?;
    let tx = db
        .transaction(&[store], TransactionMode::ReadOnly)
        .map_err(err)?;
    let vals = tx
        .object_store(store)
        .map_err(err)?
        .get_all(None, None)
        .map_err(err)?
        .await
        .map_err(err)?;
    Ok(vals.into_iter().filter_map(|v| v.as_string()).collect())
}

pub async fn put(store: &str, key: &str, value: &str) -> Result<(), String> {
    let db = db().await?;
    let tx = db
        .transaction(&[store], TransactionMode::ReadWrite)
        .map_err(err)?;
    tx.object_store(store)
        .map_err(err)?
        .put(&JsValue::from_str(value), Some(&JsValue::from_str(key)))
        .map_err(err)?
        .await
        .map_err(err)?;
    tx.await.map_err(err)?;
    Ok(())
}

pub async fn delete(store: &str, keys: &[String]) -> Result<(), String> {
    let db = db().await?;
    let tx = db
        .transaction(&[store], TransactionMode::ReadWrite)
        .map_err(err)?;
    let s = tx.object_store(store).map_err(err)?;
    for k in keys {
        s.delete(JsValue::from_str(k))
            .map_err(err)?
            .await
            .map_err(err)?;
    }
    tx.await.map_err(err)?;
    Ok(())
}

pub async fn clear(stores: &[&str]) -> Result<(), String> {
    let db = db().await?;
    let tx = db
        .transaction(stores, TransactionMode::ReadWrite)
        .map_err(err)?;
    for s in stores {
        tx.object_store(s)
            .map_err(err)?
            .clear()
            .map_err(err)?
            .await
            .map_err(err)?;
    }
    tx.await.map_err(err)?;
    Ok(())
}

pub async fn put_file(path: &str, sha: &str, text: &str) -> Result<(), String> {
    let v = serde_json::to_string(&CachedFile {
        sha: sha.into(),
        text: text.into(),
    })
    .map_err(err)?;
    put(FILES, path, &v).await
}

pub async fn get_file(path: &str) -> Result<Option<CachedFile>, String> {
    Ok(get(FILES, path)
        .await?
        .and_then(|s| serde_json::from_str(&s).ok()))
}

pub async fn all_files() -> Result<Vec<CachedFile>, String> {
    Ok(all(FILES)
        .await?
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect())
}
