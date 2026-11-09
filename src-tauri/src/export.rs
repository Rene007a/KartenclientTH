//! Export von Suchergebnissen als CSV, Excel (.xlsx) und Speichern von Binärdaten (PDF).
//!
//! Kurz zu Rust-Begriffen, die hier vorkommen:
//! - `Result<T, E>`: Rückgabe ist entweder `Ok(wert)` oder `Err(fehler)`.
//!   Das `?` hinter einem Aufruf heißt: "Bei Fehler sofort mit diesem Fehler zurückgeben".
//! - `Option<T>`: entweder `Some(wert)` oder `None` (z. B. Dialog abgebrochen).
//! - `#[tauri::command]`: macht die Funktion aus dem Frontend per `invoke(...)` aufrufbar.
//! - `serde::Deserialize`: wandelt das JSON aus dem Frontend automatisch in Rust-Structs um.

use std::path::PathBuf;

use rust_xlsxwriter::{Color, Format, Workbook};
use serde::Deserialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Tabelle, wie sie das Frontend schickt: `{ columns: [...], rows: [[...], ...] }`
#[derive(Deserialize)]
pub struct TableData {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

/// Zeigt den Windows-Dialog "Speichern unter". `None` = abgebrochen.
fn ask_save_path(app: &AppHandle, file_name: &str, filter_name: &str, ext: &str) -> Option<PathBuf> {
    app.dialog()
        .file()
        .set_file_name(file_name)
        .add_filter(filter_name, &[ext])
        .blocking_save_file()? // `?` bei Option: None -> sofort None zurückgeben
        .into_path()
        .ok()
}

/// Exportiert eine Tabelle als "csv" oder "xlsx".
/// Gibt den gespeicherten Pfad zurück, oder `None`, wenn der Dialog abgebrochen wurde.
///
/// `async`, damit der Dialog nicht den Hauptthread (die Oberfläche) blockiert.
#[tauri::command]
pub async fn export_table(
    app: AppHandle,
    format: String,
    file_name: String,
    sheet_name: String,
    table: TableData,
) -> Result<Option<String>, String> {
    let (filter, ext) = match format.as_str() {
        "csv" => ("CSV (Semikolon-getrennt)", "csv"),
        "xlsx" => ("Excel-Arbeitsmappe", "xlsx"),
        other => return Err(format!("Unbekanntes Format: {other}")),
    };

    let Some(path) = ask_save_path(&app, &file_name, filter, ext) else {
        return Ok(None); // Benutzer hat abgebrochen
    };

    // Fehler beider Funktionen werden mit `map_err` in Text umgewandelt,
    // damit das Frontend eine lesbare Meldung bekommt.
    match ext {
        "csv" => write_csv(&path, &table).map_err(|e| e.to_string())?,
        _ => write_xlsx(&path, &sheet_name, &table).map_err(|e| e.to_string())?,
    }

    Ok(Some(path.display().to_string()))
}

/// CSV im "Excel-Deutsch"-Format: Semikolon als Trenner, UTF-8 mit BOM (damit Umlaute stimmen).
fn write_csv(path: &PathBuf, table: &TableData) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = std::fs::File::create(path)?;
    std::io::Write::write_all(&mut file, b"\xEF\xBB\xBF")?; // BOM

    let mut writer = csv::WriterBuilder::new().delimiter(b';').from_writer(file);
    writer.write_record(&table.columns)?;
    for row in &table.rows {
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

/// Excel-Datei mit fetter Kopfzeile, fixierter erster Zeile, Autofilter und passenden Spaltenbreiten.
fn write_xlsx(
    path: &PathBuf,
    sheet_name: &str,
    table: &TableData,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name(clean_sheet_name(sheet_name))?;

    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xEEF1F4));

    for (col, name) in table.columns.iter().enumerate() {
        sheet.write_string_with_format(0, col as u16, name, &header)?;
    }

    for (r, row) in table.rows.iter().enumerate() {
        let excel_row = (r + 1) as u32;
        for (col, value) in row.iter().enumerate() {
            // Zahlen als echte Zahlen schreiben, damit man in Excel damit rechnen kann
            match as_number(value) {
                Some(n) => sheet.write_number(excel_row, col as u16, n)?,
                None => sheet.write_string(excel_row, col as u16, value)?,
            };
        }
    }

    if !table.columns.is_empty() {
        sheet.set_freeze_panes(1, 0)?;
        sheet.autofilter(0, 0, table.rows.len() as u32, (table.columns.len() - 1) as u16)?;
    }
    sheet.autofit();

    workbook.save(path)?;
    Ok(())
}

/// Erkennt Zahlen wie "750" oder "12.5" – aber keine Kennzeichen mit führender Null
/// (z. B. "0123"), die sonst ihre Nullen verlieren würden.
fn as_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let leading_zero = s.len() > 1 && s.starts_with('0') && !s.starts_with("0.");
    if s.is_empty() || leading_zero || s.len() > 15 {
        return None;
    }
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Excel erlaubt max. 31 Zeichen und keine der Zeichen []:*?/\ im Blattnamen.
fn clean_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .take(31)
        .collect();
    if cleaned.trim().is_empty() {
        "Ergebnis".to_string()
    } else {
        cleaned
    }
}

/// Speichert Binärdaten (z. B. ein im Frontend erzeugtes PDF).
///
/// Die Bytes kommen als "Raw"-Body (schneller als ein JSON-Zahlenarray),
/// Dateiname und Dateityp stehen in Headern.
#[tauri::command]
pub async fn save_binary(app: AppHandle, request: Request<'_>) -> Result<Option<String>, String> {
    let InvokeBody::Raw(data) = request.body() else {
        return Err("Erwarte Binärdaten".into());
    };

    let header = |name: &str| -> String {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| urlencoding_decode(v))
            .unwrap_or_default()
    };
    let file_name = header("x-file-name");
    let ext = header("x-file-ext");
    let filter = header("x-file-filter");

    let Some(path) = ask_save_path(&app, &file_name, &filter, &ext) else {
        return Ok(None);
    };
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// HTTP-Header dürfen nur ASCII enthalten – das Frontend kodiert Umlaute daher
/// mit `encodeURIComponent`. Hier wird das wieder zurückverwandelt.
fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // "%C3" -> Byte 0xC3 (nur wenn wirklich zwei Hex-Ziffern folgen)
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zahlen_erkennen() {
        assert_eq!(as_number("750"), Some(750.0));
        assert_eq!(as_number("12.5"), Some(12.5));
        assert_eq!(as_number("0.5"), Some(0.5));
        assert_eq!(as_number("0"), Some(0.0));
        assert_eq!(as_number("0123"), None);
        assert_eq!(as_number("16010114100144______"), None);
        assert_eq!(as_number(""), None);
        assert_eq!(as_number("Erfurt"), None);
    }

    #[test]
    fn blattname_bereinigen() {
        assert_eq!(clean_sheet_name("Flurstücke (WFS)"), "Flurstücke (WFS)");
        assert_eq!(clean_sheet_name("a/b:c"), "a_b_c");
        assert_eq!(clean_sheet_name(""), "Ergebnis");
        assert_eq!(clean_sheet_name(&"x".repeat(40)).len(), 31);
    }

    #[test]
    fn dateien_schreiben() {
        let table = TableData {
            columns: vec!["gemarkung".into(), "flaeche".into()],
            rows: vec![
                vec!["Erfurt-Mitte".into(), "750".into()],
                vec!["Semi;kolon \"Test\"".into(), "12.5".into()],
            ],
        };
        let dir = std::env::temp_dir();

        let csv_path = dir.join("kartenclient_test.csv");
        write_csv(&csv_path, &table).unwrap();
        let text = std::fs::read_to_string(&csv_path).unwrap();
        assert!(text.starts_with('\u{FEFF}'));
        assert!(text.contains("gemarkung;flaeche"));
        assert!(text.contains("\"Semi;kolon \"\"Test\"\"\";12.5")); // korrekt maskiert

        let xlsx_path = dir.join("kartenclient_test.xlsx");
        write_xlsx(&xlsx_path, "Flurstücke (WFS)", &table).unwrap();
        let bytes = std::fs::read(&xlsx_path).unwrap();
        assert_eq!(&bytes[0..2], b"PK"); // xlsx ist ein ZIP-Archiv

        let _ = std::fs::remove_file(csv_path);
        let _ = std::fs::remove_file(xlsx_path);
    }

    #[test]
    fn url_dekodieren() {
        assert_eq!(urlencoding_decode("Flurst%C3%BCcke.pdf"), "Flurstücke.pdf");
        assert_eq!(urlencoding_decode("ohne"), "ohne");
        assert_eq!(urlencoding_decode("100%"), "100%");
    }
}
