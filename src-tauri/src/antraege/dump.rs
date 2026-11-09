//! Liest einen MariaDB/MySQL-Dump (mysqldump / mariadb-dump) zeilenweise.
//!
//! Es wird nie die ganze Datei in den Speicher geladen – wichtig bei 1 GB großen Dumps.
//! Unterstützt werden `CREATE TABLE` (für die Spaltennamen) und
//! `INSERT INTO ... VALUES (...),(...);` – egal ob ein Datensatz pro Zeile
//! oder alle Datensätze in einer langen Zeile stehen.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Ein Wert aus einem Datensatz
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    /// Text in Anführungszeichen (bereits "entescaped")
    Text(Vec<u8>),
    /// Hex-Literal wie `0x0000000001...` (z. B. Geometrien)
    Hex(Vec<u8>),
    /// Alles andere: Zahlen usw.
    Raw(String),
}

impl Value {
    /// Wert als Text (None bei NULL)
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Text(b) => Some(String::from_utf8_lossy(b).into_owned()),
            Value::Raw(s) => Some(s.clone()),
            Value::Hex(b) => Some(b.iter().map(|x| format!("{x:02X}")).collect()),
        }
    }

    /// Wert als Bytes (für Geometrien; MariaDB schreibt sie als Hex oder als _binary-Text)
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Hex(b) | Value::Text(b) if !b.is_empty() => Some(b),
            _ => None,
        }
    }
}

/// Spalte aus `CREATE TABLE`
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    /// SQL-Typ in Kleinbuchstaben, z. B. "varchar(14)" oder "geometry"
    pub sql_type: String,
}

impl Column {
    pub fn is_geometry(&self) -> bool {
        let t = self.sql_type.as_str();
        [
            "geometry", "point", "linestring", "polygon", "multipoint",
            "multilinestring", "multipolygon", "geometrycollection",
        ]
        .iter()
        .any(|g| t == *g || t.starts_with(&format!("{g} ")))
    }
}

/// Was beim Lesen passiert – wird an den Callback von [`read_dump`] gemeldet.
pub enum Event<'a> {
    /// Spalten aus `CREATE TABLE` (kommt vor dem ersten Datensatz)
    Columns(&'a [Column]),
    /// Ein Datensatz
    Row(Vec<Value>),
    /// Bisher gelesene Bytes (für Fortschrittsanzeigen)
    Progress(u64),
}

/// Liest den Dump und meldet Spalten, Datensätze und Fortschritt der Tabelle `table`.
///
/// Ein einziger Callback (statt drei) – so darf er ohne Konflikte mit dem
/// Borrow-Checker auf gemeinsame Variablen (z. B. eine Datenbankverbindung) zugreifen.
/// Gibt einen Fehler `E` aus dem Callback unverändert weiter (dafür steht das generische `E`).
pub fn read_dump<E: From<std::io::Error>>(
    path: &Path,
    table: &str,
    mut on_event: impl FnMut(Event) -> Result<(), E>,
) -> Result<(), E> {
    let mut reader = BufReader::with_capacity(1 << 20, File::open(path)?);
    let create_prefix = format!("CREATE TABLE `{table}`");
    let insert_prefix = format!("INSERT INTO `{table}`");

    let mut line: Vec<u8> = Vec::new();
    let mut bytes_read: u64 = 0;
    let mut last_progress: u64 = 0;

    let mut columns: Vec<Column> = Vec::new();
    let mut in_create = false;
    let mut in_insert = false;
    // Rest einer Zeile, deren Datensatz noch nicht vollständig ist
    let mut pending: Vec<u8> = Vec::new();

    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break; // Dateiende
        }
        bytes_read += n as u64;
        if bytes_read - last_progress > 4 << 20 {
            on_event(Event::Progress(bytes_read))?;
            last_progress = bytes_read;
        }

        // Zeilenende entfernen (\n bzw. \r\n)
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }

        if in_create {
            if line.starts_with(b")") {
                in_create = false;
                on_event(Event::Columns(&columns))?;
            } else if let Some(col) = parse_column_line(&line) {
                columns.push(col);
            }
            continue;
        }

        if !in_insert {
            if line.starts_with(create_prefix.as_bytes()) {
                in_create = true;
                columns.clear();
            } else if line.starts_with(insert_prefix.as_bytes()) {
                in_insert = true;
                // Datensätze können direkt hinter "VALUES" in derselben Zeile stehen
                if let Some(pos) = find(&line, b"VALUES") {
                    pending.extend_from_slice(&line[pos + 6..]);
                }
            } else {
                continue;
            }
        } else {
            if !pending.is_empty() {
                pending.push(b'\n'); // Zeilenumbruch gehörte zum Text
            }
            pending.extend_from_slice(&line);
        }

        // Alle vollständigen Datensätze aus `pending` verarbeiten
        let mut pos = 0;
        loop {
            while pos < pending.len() && matches!(pending[pos], b' ' | b',' | b'\t') {
                pos += 1;
            }
            if pos >= pending.len() {
                break;
            }
            match pending[pos] {
                b'(' => match parse_tuple(&pending[pos..]) {
                    Some((values, used)) => {
                        on_event(Event::Row(values))?;
                        pos += used;
                    }
                    None => break, // unvollständig -> nächste Zeile anhängen
                },
                b';' => {
                    in_insert = false;
                    pos = pending.len();
                    break;
                }
                _ => pos += 1, // unerwartetes Zeichen überspringen
            }
        }
        pending.drain(..pos);
        if !in_insert {
            pending.clear();
        }
    }

    on_event(Event::Progress(bytes_read))?;
    Ok(())
}

/// "  `name` varchar(14) NOT NULL," -> Column { name, sql_type }
fn parse_column_line(line: &[u8]) -> Option<Column> {
    let s = std::str::from_utf8(line).ok()?.trim_start();
    let rest = s.strip_prefix('`')?;
    let end = rest.find('`')?;
    let name = rest[..end].to_string();
    let sql_type = rest[end + 1..]
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(',')
        .to_lowercase();
    Some(Column { name, sql_type })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Liest einen Datensatz `( ... )` ab Position 0.
/// Gibt die Werte und die Anzahl verbrauchter Bytes zurück – oder `None`, wenn er unvollständig ist.
pub fn parse_tuple(b: &[u8]) -> Option<(Vec<Value>, usize)> {
    debug_assert_eq!(b.first(), Some(&b'('));
    let mut values = Vec::new();
    let mut i = 1;

    loop {
        while *b.get(i)? == b' ' {
            i += 1;
        }

        let value = match b[i] {
            b'\'' => {
                let (text, used) = parse_string(&b[i..])?;
                i += used;
                Value::Text(text)
            }
            b'_' if b[i..].starts_with(b"_binary '") => {
                let (bytes, used) = parse_string(&b[i + 8..])?;
                i += 8 + used;
                Value::Text(bytes)
            }
            b'0' if b.get(i + 1) == Some(&b'x') => {
                let start = i + 2;
                let mut j = start;
                while b.get(j)?.is_ascii_hexdigit() {
                    j += 1;
                }
                i = j;
                Value::Hex(decode_hex(&b[start..j])?)
            }
            _ => {
                let start = i;
                while !matches!(*b.get(i)?, b',' | b')') {
                    i += 1;
                }
                let raw = std::str::from_utf8(&b[start..i]).ok()?.trim();
                if raw.eq_ignore_ascii_case("NULL") {
                    Value::Null
                } else {
                    Value::Raw(raw.to_string())
                }
            }
        };
        values.push(value);

        while *b.get(i)? == b' ' {
            i += 1;
        }
        match b[i] {
            b',' => i += 1,
            b')' => return Some((values, i + 1)),
            _ => return None, // sollte nicht vorkommen
        }
    }
}

/// Liest `'...'` mit MySQL-Escapes (\' \\ \n \r \t \0 \Z und '').
/// Gibt den Inhalt und die verbrauchten Bytes (inkl. Anführungszeichen) zurück.
fn parse_string(b: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    let mut i = 1; // erstes ' überspringen
    loop {
        match *b.get(i)? {
            b'\\' => {
                let c = *b.get(i + 1)?;
                out.push(match c {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'0' => 0,
                    b'Z' => 0x1a,
                    b'b' => 0x08,
                    other => other, // \' \" \\ usw.
                });
                i += 2;
            }
            b'\'' if b.get(i + 1) == Some(&b'\'') => {
                out.push(b'\'');
                i += 2;
            }
            b'\'' => return Some((out, i + 1)),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

fn decode_hex(hex: &[u8]) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    hex.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datensatz_lesen() {
        let row = br"(1,'a\'b','x\\y\nz',NULL,0x0A0B,_binary 'q',3.5,'it''s'),";
        let (v, used) = parse_tuple(row).unwrap();
        assert_eq!(used, row.len() - 1);
        assert_eq!(v[0], Value::Raw("1".into()));
        assert_eq!(v[1], Value::Text(b"a'b".to_vec()));
        assert_eq!(v[2], Value::Text(b"x\\y\nz".to_vec()));
        assert_eq!(v[3], Value::Null);
        assert_eq!(v[4], Value::Hex(vec![0x0A, 0x0B]));
        assert_eq!(v[5], Value::Text(b"q".to_vec()));
        assert_eq!(v[6], Value::Raw("3.5".into()));
        assert_eq!(v[7], Value::Text(b"it's".to_vec()));
    }

    #[test]
    fn unvollstaendig() {
        assert!(parse_tuple(b"(1,'abc").is_none());
        assert!(parse_tuple(b"(1,0x0A").is_none());
        assert!(parse_tuple(b"(1,2").is_none());
    }

    #[test]
    fn spaltendefinition() {
        let c = parse_column_line(b"  `flstgeometrie` geometry DEFAULT NULL,").unwrap();
        assert_eq!(c.name, "flstgeometrie");
        assert!(c.is_geometry());
        let c = parse_column_line(b"  `antragsnummer` varchar(14) NOT NULL,").unwrap();
        assert_eq!(c.sql_type, "varchar(14)");
        assert!(!c.is_geometry());
        assert!(parse_column_line(b"  PRIMARY KEY (`id`),").is_none());
    }

    #[test]
    fn ganzer_dump_beide_varianten() {
        let dir = std::env::temp_dir();
        let dump = "-- Kommentar\n\
            CREATE TABLE `t` (\n  `id` int(11) NOT NULL,\n  `name` text,\n  PRIMARY KEY (`id`)\n) ENGINE=MyISAM;\n\
            INSERT INTO `t` VALUES (1,'a'),(2,'b;c');\n\
            INSERT INTO `t` VALUES\n(3,'d'),\n(4,'e');\n\
            INSERT INTO `andere` VALUES (9,'x');\n";
        let path = dir.join("kartenclient_dump_test.sql");
        std::fs::write(&path, dump).unwrap();

        let mut cols = Vec::new();
        let mut rows = Vec::new();
        read_dump::<std::io::Error>(&path, "t", |event| {
            match event {
                Event::Columns(c) => cols = c.iter().map(|c| c.name.clone()).collect(),
                Event::Row(r) => rows.push(r[1].as_text().unwrap()),
                Event::Progress(_) => {}
            }
            Ok(())
        })
        .unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(cols, ["id", "name"]);
        assert_eq!(rows, ["a", "b;c", "d", "e"]);
    }
}
