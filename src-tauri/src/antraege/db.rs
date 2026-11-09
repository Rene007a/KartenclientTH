//! SQLite-Datei mit den Anträgen.
//!
//! Aufbau:
//! - `antraege`        – alle Sachdaten (TEXT) + `geom` (WKB, EPSG:25832)
//! - `antraege_rtree`  – räumlicher Index (Bounding Boxes) für schnelle Ausschnittsabfragen
//! - `meta`            – Infos zum Import (Quelle, Datum, Anzahl)

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

pub type DbResult<T> = rusqlite::Result<T>;

/// Datenstand – wird in der Seitenleiste angezeigt
#[derive(Serialize, Clone, Debug)]
pub struct Status {
    pub anzahl: i64,
    pub ohne_geometrie: i64,
    pub quelle: String,
    /// Unix-Zeit in Sekunden
    pub importiert_am: i64,
}

/// Spalten- und Tabellennamen in SQL sicher in Anführungszeichen setzen
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Neue, leere Datenbank für den Import (schnelle Einstellungen, da sie neu aufgebaut wird)
pub fn create(path: &Path, columns: &[String]) -> DbResult<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA page_size = 8192;",
    )?;

    let cols: String = columns.iter().map(|c| format!(", {} TEXT", ident(c))).collect();
    conn.execute_batch(&format!(
        "CREATE TABLE antraege (id INTEGER PRIMARY KEY, geom BLOB NOT NULL{cols});
         CREATE VIRTUAL TABLE antraege_rtree USING rtree(id, minx, maxx, miny, maxy);
         CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);"
    ))?;
    Ok(conn)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> DbResult<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn get_meta(conn: &Connection, key: &str) -> DbResult<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

pub fn open_readonly(path: &Path) -> DbResult<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

/// Prüft, ob eine Datei eine gültige Antragsdatenbank ist, und liefert ihren Stand
pub fn status(conn: &Connection) -> DbResult<Status> {
    let anzahl: i64 = conn.query_row("SELECT count(*) FROM antraege", [], |r| r.get(0))?;
    // Index muss vorhanden sein
    conn.query_row("SELECT count(*) FROM antraege_rtree LIMIT 1", [], |r| r.get::<_, i64>(0))?;
    let num = |k: &str| -> DbResult<i64> {
        Ok(get_meta(conn, k)?.and_then(|v| v.parse().ok()).unwrap_or(0))
    };
    Ok(Status {
        anzahl,
        ohne_geometrie: num("ohne_geometrie")?,
        quelle: get_meta(conn, "quelle")?.unwrap_or_default(),
        importiert_am: num("importiert_am")?,
    })
}

/// Kurzinfo je Antrag für die Kartendarstellung
#[derive(Serialize)]
pub struct AntragKurz {
    pub id: i64,
    pub geom: Vec<u8>,
    pub antragsnummer: Option<String>,
    pub antragsart: Option<String>,
    pub datum_eingang: Option<String>,
}

/// Anträge, deren Bounding Box den Ausschnitt `[minx, miny, maxx, maxy]` berührt
pub fn in_extent(conn: &Connection, e: [f64; 4], limit: u32) -> DbResult<Vec<AntragKurz>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.id, a.geom, a.antragsnummer, a.antragsart, a.datum_eingang
         FROM antraege_rtree r JOIN antraege a ON a.id = r.id
         WHERE r.maxx >= ?1 AND r.minx <= ?3 AND r.maxy >= ?2 AND r.miny <= ?4
         LIMIT ?5",
    )?;
    let rows = stmt.query_map(params![e[0], e[1], e[2], e[3], limit], |r| {
        Ok(AntragKurz {
            id: r.get(0)?,
            geom: r.get(1)?,
            antragsnummer: r.get(2)?,
            antragsart: r.get(3)?,
            datum_eingang: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Alle Sachdaten eines Antrags als (Spalte, Wert)-Paare in Tabellenreihenfolge.
/// Leere Felder werden weggelassen.
pub fn details(conn: &Connection, id: i64) -> DbResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT * FROM antraege WHERE id = ?1")?;
    let names: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
    stmt.query_row([id], |row| {
        let mut out = Vec::new();
        for (i, name) in names.iter().enumerate() {
            if name == "geom" {
                continue;
            }
            use rusqlite::types::ValueRef;
            let text = match row.get_ref(i)? {
                ValueRef::Null => continue,
                ValueRef::Integer(n) => n.to_string(),
                ValueRef::Real(f) => f.to_string(),
                ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                ValueRef::Blob(_) => continue,
            };
            out.push((name.clone(), text));
        }
        Ok(out)
    })
}
