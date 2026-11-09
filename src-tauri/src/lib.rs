// Einstiegspunkt der Rust-Seite (wird von main.rs aufgerufen).
//
// Die Karte selbst läuft komplett im Frontend (OpenLayers im WebView).
// Rust baut hier nur das Fenster auf. Später kannst du hier eigene
// "Commands" ergänzen, die das Frontend per `invoke("name")` aufruft,
// z. B. zum Lesen lokaler Shapefiles/GeoPackages.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Plugin zum Öffnen von Links im Standardbrowser
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der Tauri-Anwendung");
}
