// Einstiegspunkt der Rust-Seite (wird von main.rs aufgerufen).
//
// Die Karte selbst läuft komplett im Frontend (OpenLayers im WebView).
// Rust baut das Fenster auf und stellt "Commands" bereit, die das
// Frontend per `invoke("name", ...)` aufruft.

// Bindet die Datei export.rs als Modul ein
mod export;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Plugin zum Öffnen von Links im Standardbrowser
        .plugin(tauri_plugin_opener::init())
        // Plugin für Datei-Dialoge ("Speichern unter")
        .plugin(tauri_plugin_dialog::init())
        // Alle Commands, die das Frontend aufrufen darf
        .invoke_handler(tauri::generate_handler![
            export::export_table,
            export::save_binary
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der Tauri-Anwendung");
}
