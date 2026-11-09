// Einstiegspunkt der Rust-Seite (wird von main.rs aufgerufen).
//
// Die Karte selbst läuft komplett im Frontend (OpenLayers im WebView).
// Rust baut das Fenster auf und stellt "Commands" bereit, die das
// Frontend per `invoke("name", ...)` aufruft.

// Bindet die Datei export.rs und den Ordner antraege/ (mod.rs) als Module ein
mod antraege;
mod export;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Plugin zum Öffnen von Links im Standardbrowser
        .plugin(tauri_plugin_opener::init())
        // Plugin für Datei-Dialoge ("Speichern unter")
        .plugin(tauri_plugin_dialog::init())
        // Gemeinsamer Zustand: verhindert parallele Importe
        .manage(antraege::ImportState::default())
        // Alle Commands, die das Frontend aufrufen darf
        .invoke_handler(tauri::generate_handler![
            export::export_table,
            export::save_binary,
            antraege::antraege_status,
            antraege::import_antraege,
            antraege::antraege_in_extent,
            antraege::antrag_details
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der Tauri-Anwendung");
}
