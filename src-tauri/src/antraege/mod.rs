//! Eigene Antragsdaten: Import (MariaDB-Dump oder fertige .db) und Abfragen für die Karte.
//!
//! Die Daten liegen in `%APPDATA%\de.kartenclient.th\antraege.db` – getrennt von der EXE.

mod db;
mod dump;
mod php;
mod wkb;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rusqlite::types::Value as SqlValue;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

pub use db::Status;
use dump::{Column, Event, Value};

/// Name der Tabelle im MariaDB-Dump
const TABLE: &str = "antraege";
const DB_FILE: &str = "antraege.db";

/// Verhindert, dass zwei Importe gleichzeitig laufen.
/// Wird in lib.rs mit `.manage(...)` bei Tauri registriert.
#[derive(Default)]
pub struct ImportState(AtomicBool);

/// Fortschritt, der während des Imports an das Frontend gesendet wird
#[derive(Serialize, Clone)]
pub struct Progress {
    pub phase: &'static str,
    pub done: u64,
    pub total: u64,
    pub rows: u64,
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(DB_FILE))
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tauri-Commands
// ---------------------------------------------------------------------------

/// Aktueller Datenstand, oder `None`, wenn noch nichts importiert wurde
#[tauri::command]
pub async fn antraege_status(app: AppHandle) -> Result<Option<Status>, String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let conn = db::open_readonly(&path).map_err(|e| e.to_string())?;
    db::status(&conn).map(Some).map_err(|e| e.to_string())
}

/// Fragt nach einer Datei (.sql oder .db) und importiert sie.
/// `None` = Dialog abgebrochen.
#[tauri::command]
pub async fn import_antraege(
    app: AppHandle,
    state: State<'_, ImportState>,
) -> Result<Option<Status>, String> {
    // swap(true) liefert den alten Wert: war er schon true, läuft bereits ein Import
    if state.0.swap(true, Ordering::SeqCst) {
        return Err("Es läuft bereits ein Import.".into());
    }
    let result = run_import(&app).await;
    state.0.store(false, Ordering::SeqCst);
    result
}

async fn run_import(app: &AppHandle) -> Result<Option<Status>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Antragsdaten importieren")
        .add_filter("Antragsdaten (MariaDB-Dump oder SQLite)", &["sql", "db"])
        .blocking_pick_file();
    let Some(source) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    import_from(app, source).await.map(Some)
}

/// Importiert eine bestimmte Datei und meldet den Fortschritt an das Frontend
async fn import_from(app: &AppHandle, source: PathBuf) -> Result<Status, String> {
    let target = db_path(app)?;

    // Die eigentliche Arbeit dauert lange -> in einem eigenen Thread, damit die App reagiert
    let app2 = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        import_file(&source, &target, |p| {
            let _ = app2.emit("antraege-import-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())??; // erstes ? = Thread-Fehler, zweites ? = Import-Fehler

    Ok(status)
}

// ---------- Neuere Daten verfügbar? ----------

#[derive(Serialize)]
pub struct UpdateInfo {
    /// Pfad der Quelldatei, z. B. `\\server\gis\antraege.db`
    pfad: String,
    /// Änderungszeit der Quelldatei (Unix-Sekunden)
    geaendert: i64,
}

/// Änderungszeit einer Datei in Unix-Sekunden (`None`, wenn nicht erreichbar)
fn file_modified(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// Prüft, ob die zuletzt importierte Quelldatei inzwischen neuer ist.
/// Ein nicht erreichbares Netzlaufwerk ist kein Fehler – dann gibt es einfach kein Update.
#[tauri::command]
pub async fn antraege_update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    // Netzlaufwerke können langsam antworten -> eigener Thread
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_readonly(&path).ok()?;
        let source = db::source_info(&conn).ok()??;
        drop(conn);
        let modified = file_modified(Path::new(&source.path))?;
        (modified > source.modified).then_some(UpdateInfo { pfad: source.path, geaendert: modified })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Importiert erneut aus der zuletzt verwendeten Quelldatei (ohne Dateidialog)
#[tauri::command]
pub async fn antraege_update(app: AppHandle, state: State<'_, ImportState>) -> Result<Status, String> {
    if state.0.swap(true, Ordering::SeqCst) {
        return Err("Es läuft bereits ein Import.".into());
    }
    let result = run_update(&app).await;
    state.0.store(false, Ordering::SeqCst);
    result
}

async fn run_update(app: &AppHandle) -> Result<Status, String> {
    let path = db_path(app)?;
    let source = {
        let conn = db::open_readonly(&path).map_err(|e| e.to_string())?;
        db::source_info(&conn).map_err(|e| e.to_string())?
    }
    .ok_or("Es ist keine Quelldatei bekannt. Bitte über „Daten importieren …“ einlesen.")?;
    import_from(app, PathBuf::from(source.path)).await
}

// ---------- Räumliche Suche ----------

#[derive(Serialize)]
pub struct SearchHit {
    id: i64,
    /// Geometrie als WKB, Base64-kodiert
    wkb: String,
    /// Alle Sachdaten als (Feld, Wert)-Paare
    props: Vec<(String, String)>,
}

#[derive(Serialize)]
pub struct SearchResult {
    features: Vec<SearchHit>,
    /// true, wenn es mehr als `limit` Treffer gibt
    truncated: bool,
}

/// Alle Anträge, die mindestens eine der Suchgeometrien schneiden.
/// `geometries`: WKB als Hex-Text (so liefert OpenLayers sie), EPSG:25832.
#[tauri::command]
pub async fn antraege_search(
    app: AppHandle,
    geometries: Vec<String>,
    limit: u32,
) -> Result<SearchResult, String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Err("Es sind noch keine Antragsdaten importiert.".into());
    }
    tauri::async_runtime::spawn_blocking(move || search(&path, &geometries, limit as usize))
        .await
        .map_err(|e| e.to_string())?
}

fn search(path: &Path, geometries: &[String], limit: usize) -> Result<SearchResult, String> {
    use geo::Intersects;

    // Suchgeometrien einlesen: (geo-Geometrie, Bounding Box)
    let searches = geometries
        .iter()
        .map(|hex| {
            let bytes = dump::decode_hex(hex.as_bytes()).ok_or("Suchgeometrie ist kein gültiges WKB.")?;
            let geom = wkb::to_geo(&bytes).ok_or("Suchgeometrie konnte nicht gelesen werden.")?;
            let bbox = wkb::bbox(&bytes).ok_or("Suchgeometrie ist leer.")?;
            Ok((geom, bbox))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if searches.is_empty() {
        return Err("Keine Suchgeometrie gezeichnet.".into());
    }

    let conn = db::open_readonly(path).map_err(|e| e.to_string())?;
    let mut found: Vec<(i64, Vec<u8>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut truncated = false;

    for (search_geom, bbox) in &searches {
        // 1. Schnell: Kandidaten über den R-Tree (nur Bounding Boxes)
        // 2. Genau: echte Prüfung "schneidet" mit der geo-Bibliothek
        db::for_each_candidate(&conn, *bbox, |id, geom| {
            if seen.contains(&id) {
                return true;
            }
            let hit = wkb::to_geo(geom).is_some_and(|g| g.intersects(search_geom));
            if hit {
                seen.insert(id);
                found.push((id, geom.to_vec()));
                if found.len() > limit {
                    found.pop();
                    truncated = true;
                    return false; // abbrechen
                }
            }
            true
        })
        .map_err(|e| e.to_string())?;
        if truncated {
            break;
        }
    }

    let b64 = base64::engine::general_purpose::STANDARD;
    let features = found
        .into_iter()
        .map(|(id, geom)| {
            let props = db::details(&conn, id).map_err(|e| e.to_string())?;
            Ok(SearchHit { id, wkb: b64.encode(geom), props })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(SearchResult { features, truncated })
}

/// Ergebnis einer Ausschnittsabfrage
#[derive(Serialize)]
pub struct ExtentResult {
    features: Vec<AntragFeature>,
    /// true, wenn das Limit erreicht wurde
    truncated: bool,
}

#[derive(Serialize)]
pub struct AntragFeature {
    id: i64,
    /// Geometrie als WKB, Base64-kodiert (kompakter als ein JSON-Zahlenarray)
    wkb: String,
    antragsnummer: Option<String>,
    antragsart: Option<String>,
    datum_eingang: Option<String>,
}

/// Anträge im Kartenausschnitt `[minx, miny, maxx, maxy]` (EPSG:25832)
#[tauri::command]
pub async fn antraege_in_extent(
    app: AppHandle,
    extent: [f64; 4],
    limit: u32,
) -> Result<ExtentResult, String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Ok(ExtentResult { features: vec![], truncated: false });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_readonly(&path).map_err(|e| e.to_string())?;
        let rows = db::in_extent(&conn, extent, limit).map_err(|e| e.to_string())?;
        let truncated = rows.len() >= limit as usize;
        let b64 = base64::engine::general_purpose::STANDARD;
        let features = rows
            .into_iter()
            .map(|r| AntragFeature {
                id: r.id,
                wkb: b64.encode(&r.geom),
                antragsnummer: r.antragsnummer,
                antragsart: r.antragsart,
                datum_eingang: r.datum_eingang,
            })
            .collect();
        Ok(ExtentResult { features, truncated })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Alle Sachdaten eines Antrags (für die Objektinfo)
#[tauri::command]
pub async fn antrag_details(app: AppHandle, id: i64) -> Result<Vec<(String, String)>, String> {
    let path = db_path(&app)?;
    let conn = db::open_readonly(&path).map_err(|e| e.to_string())?;
    db::details(&conn, id).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Importiert `source` (.sql oder .db) nach `target`.
/// Es wird erst in eine temporäre Datei geschrieben und diese am Ende umbenannt –
/// so bleibt bei einem Fehler der alte Datenstand erhalten.
pub fn import_file(
    source: &Path,
    target: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<Status, String> {
    let tmp = target.with_extension("db.tmp");
    let _ = std::fs::remove_file(&tmp);

    let is_sql = source
        .extension()
        .map(|e| e.eq_ignore_ascii_case("sql"))
        .unwrap_or(false);

    let result = if is_sql {
        convert_dump(source, &tmp, &mut progress)
    } else {
        copy_db(source, &tmp, &mut progress)
    };
    // Quelle merken – für die spätere Prüfung "Neuere Daten verfügbar"
    let result = result.and_then(|()| remember_source(source, &tmp));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // Prüfen, dann die alte Datei ersetzen
    let status = {
        let conn = db::open_readonly(&tmp).map_err(|e| e.to_string())?;
        db::status(&conn).map_err(|e| format!("Die Datei ist keine gültige Antragsdatenbank: {e}"))?
    }; // <- Verbindung wird hier geschlossen (Ende des Blocks), sonst klappt das Umbenennen nicht

    replace_file(&tmp, target)?;
    Ok(status)
}

fn remember_source(source: &Path, tmp: &Path) -> Result<(), String> {
    let full_path = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
    let modified = file_modified(source).unwrap_or(0);
    let conn = rusqlite::Connection::open(tmp).map_err(|e| e.to_string())?;
    db::set_meta(&conn, "quelle_pfad", &full_path.display().to_string()).map_err(|e| e.to_string())?;
    db::set_meta(&conn, "quelle_geaendert", &modified.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Windows erlaubt das Ersetzen nicht, solange die Datei gerade gelesen wird – kurz erneut versuchen
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    let mut last_err = None;
    for _ in 0..10 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }
    Err(format!("Datenbank konnte nicht ersetzt werden: {}", last_err.unwrap()))
}

fn copy_db(source: &Path, tmp: &Path, progress: &mut impl FnMut(Progress)) -> Result<(), String> {
    let total = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    progress(Progress { phase: "kopieren", done: 0, total, rows: 0 });
    std::fs::copy(source, tmp).map_err(|e| e.to_string())?;

    let conn = rusqlite::Connection::open(tmp).map_err(|e| e.to_string())?;
    db::status(&conn).map_err(|e| format!("Die Datei ist keine gültige Antragsdatenbank: {e}"))?;
    db::set_meta(&conn, "importiert_am", &now_unix().to_string()).map_err(|e| e.to_string())?;
    progress(Progress { phase: "kopieren", done: total, total, rows: 0 });
    Ok(())
}

/// Wie eine Spalte aus dem Dump in die SQLite-Tabelle übernommen wird
enum Field {
    /// 1:1 als Text (PHP-serialisierte Werte werden lesbar gemacht)
    Plain(usize),
    /// `gmkgmd` -> gemarkung, gemarkung_nr, gemeinde, gemeinde_nr, kb
    GmkGmd(usize),
}

const GMKGMD_COLUMNS: [(&str, &str); 5] = [
    ("gmk", "gemarkung"),
    ("gmkNR", "gemarkung_nr"),
    ("gmd", "gemeinde"),
    ("gmdNR", "gemeinde_nr"),
    ("kb", "kb"),
];

struct Mapping {
    id: Option<usize>,
    geom: usize,
    fields: Vec<Field>,
    target_columns: Vec<String>,
}

impl Mapping {
    fn new(columns: &[Column]) -> Result<Self, String> {
        let geom = columns
            .iter()
            .position(Column::is_geometry)
            .ok_or("Im Dump wurde keine Geometriespalte gefunden.")?;
        let id = columns.iter().position(|c| c.name == "id");

        let mut fields = Vec::new();
        let mut target_columns = Vec::new();
        for (i, c) in columns.iter().enumerate() {
            if Some(i) == id || i == geom || c.name == "geom" {
                continue;
            }
            if c.name == "gmkgmd" {
                fields.push(Field::GmkGmd(i));
                target_columns.extend(GMKGMD_COLUMNS.iter().map(|(_, t)| t.to_string()));
            } else {
                fields.push(Field::Plain(i));
                target_columns.push(c.name.clone());
            }
        }
        Ok(Mapping { id, geom, fields, target_columns })
    }

    /// Datensatz umwandeln. `None` = keine (gültige) Geometrie -> wird übersprungen
    fn convert(&self, values: &[Value], fallback_id: i64) -> Option<(i64, Vec<u8>, [f64; 4], Vec<SqlValue>)> {
        let raw_geom = values.get(self.geom)?.as_bytes()?;
        let wkb = wkb::strip_mariadb_srid(raw_geom)?;
        let bbox = wkb::bbox(wkb)?;

        let id = self
            .id
            .and_then(|i| values.get(i)?.as_text()?.parse().ok())
            .unwrap_or(fallback_id);

        let mut out = Vec::with_capacity(self.target_columns.len());
        for field in &self.fields {
            match field {
                Field::Plain(i) => out.push(to_sql_text(values.get(*i))),
                Field::GmkGmd(i) => {
                    let parsed = match values.get(*i) {
                        Some(Value::Text(b)) => php::unserialize(b),
                        _ => None,
                    };
                    for (key, _) in GMKGMD_COLUMNS {
                        let text = parsed.as_ref().and_then(|p| p.get(key)).map(|v| v.to_text());
                        out.push(non_empty(text));
                    }
                }
            }
        }
        Some((id, wkb.to_vec(), bbox, out))
    }
}

fn non_empty(s: Option<String>) -> SqlValue {
    match s {
        Some(s) if !s.trim().is_empty() => SqlValue::Text(s),
        _ => SqlValue::Null,
    }
}

/// Text aus dem Dump; PHP-serialisierte Arrays (`a:...`) werden lesbar gemacht
fn to_sql_text(v: Option<&Value>) -> SqlValue {
    match v {
        Some(Value::Text(b)) if b.starts_with(b"a:") => match php::unserialize(b) {
            Some(p) => non_empty(Some(p.to_text())),
            None => non_empty(Some(String::from_utf8_lossy(b).into_owned())),
        },
        Some(v) => non_empty(v.as_text()),
        None => SqlValue::Null,
    }
}

/// Fehlertyp für den Dump-Import.
/// Die `From`-Implementierungen erlauben es, mit `?` sowohl Dateifehler (`io::Error`)
/// als auch eigene Fehlermeldungen (`String`) einheitlich weiterzugeben.
struct ImportError(String);

impl From<std::io::Error> for ImportError {
    fn from(e: std::io::Error) -> Self {
        ImportError(format!("Fehler beim Lesen der Datei: {e}"))
    }
}

impl From<String> for ImportError {
    fn from(s: String) -> Self {
        ImportError(s)
    }
}

impl From<&str> for ImportError {
    fn from(s: &str) -> Self {
        ImportError(s.to_string())
    }
}

/// Liest den MariaDB-Dump und schreibt alle Datensätze mit Geometrie in eine neue SQLite-Datei
pub fn convert_dump(source: &Path, tmp: &Path, progress: &mut impl FnMut(Progress)) -> Result<(), String> {
    let total = std::fs::metadata(source).map_err(|e| e.to_string())?.len();
    let source_name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    // Zustand, den der Callback unten verändert
    let mut conn: Option<rusqlite::Connection> = None;
    let mut mapping: Option<Mapping> = None;
    let mut imported: u64 = 0;
    let mut skipped: u64 = 0;
    let mut seen: i64 = 0;

    dump::read_dump::<ImportError>(source, TABLE, |event| {
        match event {
            Event::Columns(columns) => {
                let m = Mapping::new(columns)?;
                let c = db::create(tmp, &m.target_columns).map_err(|e| e.to_string())?;
                c.execute_batch("BEGIN").map_err(|e| e.to_string())?;
                conn = Some(c);
                mapping = Some(m);
            }
            Event::Row(values) => {
                let (Some(c), Some(m)) = (conn.as_ref(), mapping.as_ref()) else {
                    return Err("Datensätze vor der Tabellendefinition gefunden.".into());
                };
                seen += 1;
                let Some((id, geom, bbox, fields)) = m.convert(&values, seen) else {
                    skipped += 1;
                    return Ok(());
                };
                insert(c, m, id, geom, bbox, fields).map_err(|e| format!("Antrag {id}: {e}"))?;
                imported += 1;
            }
            Event::Progress(done) => progress(Progress { phase: "lesen", done, total, rows: imported }),
        }
        Ok(())
    })
    .map_err(|e| e.0)?;

    let conn = conn.ok_or(format!("Tabelle `{TABLE}` wurde im Dump nicht gefunden."))?;
    progress(Progress { phase: "index", done: total, total, rows: imported });
    let meta = |k: &str, v: String| db::set_meta(&conn, k, &v).map_err(|e| e.to_string());
    meta("quelle", source_name)?;
    meta("importiert_am", now_unix().to_string())?;
    meta("ohne_geometrie", skipped.to_string())?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_antragsnummer ON antraege(antragsnummer);
         COMMIT;",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn insert(
    conn: &rusqlite::Connection,
    m: &Mapping,
    id: i64,
    geom: Vec<u8>,
    bbox: [f64; 4],
    fields: Vec<SqlValue>,
) -> rusqlite::Result<()> {
    // prepare_cached: Die SQL-Anweisung wird nur einmal vorbereitet und dann wiederverwendet
    let cols: String = m.target_columns.iter().map(|c| format!(", {}", db::ident(c))).collect();
    let marks: String = (0..m.target_columns.len()).map(|i| format!(", ?{}", i + 3)).collect();
    let sql = format!("INSERT INTO antraege (id, geom{cols}) VALUES (?1, ?2{marks})");

    let mut params: Vec<SqlValue> = Vec::with_capacity(fields.len() + 2);
    params.push(SqlValue::Integer(id));
    params.push(SqlValue::Blob(geom));
    params.extend(fields);
    conn.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(params))?;

    conn.prepare_cached(
        "INSERT INTO antraege_rtree (id, minx, maxx, miny, maxy) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(rusqlite::params![id, bbox[0], bbox[2], bbox[1], bbox[3]])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kleiner Dump mit 3 Datensätzen, davon 1 ohne Geometrie
    #[test]
    fn mini_dump_importieren() {
        let dir = std::env::temp_dir().join("kartenclient_import_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // MariaDB-Geometrie: 4 Byte SRID 0 + WKB-Punkt (642000, 5649000)
        let mut g = vec![0u8, 0, 0, 0, 1, 1, 0, 0, 0];
        g.extend(642000f64.to_le_bytes());
        g.extend(5649000f64.to_le_bytes());
        let hex: String = g.iter().map(|b| format!("{b:02X}")).collect();

        let gmk = r#"a:5:{s:2:\"kb\";i:51;s:3:\"gmk\";s:12:\"Erfurt-Mitte\";s:5:\"gmkNR\";s:4:\"0101\";s:3:\"gmd\";s:6:\"Erfurt\";s:5:\"gmdNR\";s:8:\"16051000\";}"#;
        let flst = r#"a:2:{i:0;s:3:\"1/2\";i:1;s:3:\"3/4\";}"#;
        let sql = format!(
            "CREATE TABLE `antraege` (\n  `id` int(11) NOT NULL,\n  `antragsnummer` varchar(14),\n  `gmkgmd` text,\n  `flurstuecke` mediumtext,\n  `flstgeometrie` geometry DEFAULT NULL,\n  `antragsart` varchar(255),\n  `datum_eingang` date\n) ENGINE=MyISAM;\n\
             INSERT INTO `antraege` VALUES\n\
             (1,'A-1','{gmk}','{flst}',0x{hex},'Zerlegung','2020-01-02'),\n\
             (2,'A-2','{gmk}','{flst}',NULL,'Zerlegung','2020-01-03'),\n\
             (3,'A-3','{gmk}','',0x{hex},'Gebäudeeinmessung',NULL);\n"
        );
        let src = dir.join("test.sql");
        std::fs::write(&src, sql).unwrap();

        let target = dir.join("antraege.db");
        let status = import_file(&src, &target, |_| {}).unwrap();
        assert_eq!(status.anzahl, 2);
        assert_eq!(status.ohne_geometrie, 1);
        assert_eq!(status.quelle, "test.sql");

        let conn = db::open_readonly(&target).unwrap();
        let hits = db::in_extent(&conn, [641000.0, 5648000.0, 643000.0, 5650000.0], 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(db::in_extent(&conn, [0.0, 0.0, 1.0, 1.0], 10).unwrap().is_empty());

        // Räumliche Suche mit einem Rechteck als WKB-Hex (wie es OpenLayers schickt)
        let rect_hex = |x0: f64, y0: f64, x1: f64, y1: f64| -> String {
            let mut w = vec![1u8];
            w.extend(3u32.to_le_bytes());
            w.extend(1u32.to_le_bytes());
            w.extend(5u32.to_le_bytes());
            for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)] {
                w.extend(x.to_le_bytes());
                w.extend(y.to_le_bytes());
            }
            w.iter().map(|b| format!("{b:02X}")).collect()
        };
        let um_punkt = rect_hex(641990.0, 5648990.0, 642010.0, 5649010.0);
        let r = search(&target, &[um_punkt.clone()], 10).unwrap();
        assert_eq!(r.features.len(), 2);
        assert!(!r.truncated);
        assert!(r.features[0].props.iter().any(|(k, v)| k == "antragsnummer" && v == "A-1"));
        let r = search(&target, &[um_punkt], 1).unwrap();
        assert_eq!(r.features.len(), 1);
        assert!(r.truncated);
        // Bounding Box trifft, Geometrie aber nicht: Rechteck knapp neben dem Punkt
        let daneben = rect_hex(642001.0, 5649001.0, 642010.0, 5649010.0);
        assert!(search(&target, &[daneben], 10).unwrap().features.is_empty());

        // Quelle für "Neuere Daten verfügbar" gemerkt
        let src_info = db::source_info(&conn).unwrap().unwrap();
        assert!(src_info.path.ends_with("test.sql"));
        assert_eq!(Some(src_info.modified), file_modified(&src));

        let d = db::details(&conn, 1).unwrap();
        let get = |k: &str| d.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("gemarkung"), Some("Erfurt-Mitte"));
        assert_eq!(get("gemeinde_nr"), Some("16051000"));
        assert_eq!(get("flurstuecke"), Some("1/2, 3/4"));
        assert_eq!(get("antragsart"), Some("Zerlegung"));
        // Leere Felder fehlen
        assert_eq!(db::details(&conn, 3).unwrap().iter().find(|(n, _)| n == "flurstuecke"), None);
        drop(conn);

        // Erneuter Import ersetzt die Datei (auch als .db-Kopie)
        let copy = dir.join("kopie.db");
        std::fs::copy(&target, &copy).unwrap();
        let status = import_file(&copy, &target, |_| {}).unwrap();
        assert_eq!(status.anzahl, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Import des echten Dumps – nur manuell:
    /// `cargo test --release -- --ignored echter_dump --nocapture`
    #[test]
    #[ignore]
    fn echter_dump() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../daten/andori_antraege.sql");
        let target = std::env::temp_dir().join("kartenclient_echt.db");
        let start = std::time::Instant::now();
        let mut last = 0;
        let status = import_file(&src, &target, |p| {
            let pct = p.done * 100 / p.total.max(1);
            if pct >= last + 10 {
                println!("{:>3} % {} ({} Anträge)", pct, p.phase, p.rows);
                last = pct;
            }
        })
        .unwrap();
        let size = std::fs::metadata(&target).unwrap().len();
        println!(
            "{} Anträge, {} ohne Geometrie übersprungen, {:.0} MB, {:.1} s",
            status.anzahl,
            status.ohne_geometrie,
            size as f64 / 1e6,
            start.elapsed().as_secs_f64()
        );

        let conn = db::open_readonly(&target).unwrap();
        let t = std::time::Instant::now();
        let hits = db::in_extent(&conn, [640000.0, 5647000.0, 645000.0, 5651000.0], 3000).unwrap();
        println!("Ausschnitt Erfurt 5x4 km: {} Anträge in {:?}", hits.len(), t.elapsed());
        let felder: Vec<String> = db::details(&conn, hits[0].id).unwrap().into_iter().map(|(k, _)| k).collect();
        println!("Felder: {}", felder.join(", "));
        drop(conn);

        // Suchzeiten: Rechtecke (WKB-Hex wie aus OpenLayers) und ein Punkt
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| -> String {
            let mut w = vec![1u8];
            w.extend(3u32.to_le_bytes());
            w.extend(1u32.to_le_bytes());
            w.extend(5u32.to_le_bytes());
            for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)] {
                w.extend(x.to_le_bytes());
                w.extend(y.to_le_bytes());
            }
            w.iter().map(|b| format!("{b:02X}")).collect()
        };
        let mut punkt = vec![1u8, 1, 0, 0, 0];
        punkt.extend(642300f64.to_le_bytes());
        punkt.extend(5649200f64.to_le_bytes());
        let punkt: String = punkt.iter().map(|b| format!("{b:02X}")).collect();

        for (name, geom) in [
            ("Rechteck 200 m", rect(642200.0, 5649100.0, 642400.0, 5649300.0)),
            ("Rechteck 2 km", rect(641000.0, 5648000.0, 643000.0, 5650000.0)),
            ("Punkt", punkt),
        ] {
            let t = std::time::Instant::now();
            let r = search(&target, &[geom], 1000).unwrap();
            println!("Suche {name}: {} Treffer (abgeschnitten: {}) in {:?}", r.features.len(), r.truncated, t.elapsed());
        }

        // Testdatenbank enthält echte (personenbezogene) Daten -> wieder löschen
        std::fs::remove_file(&target).unwrap();
    }
}
