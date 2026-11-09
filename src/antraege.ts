import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Feature from "ol/Feature";
import WKB from "ol/format/WKB";
import VectorLayer from "ol/layer/Vector";
import VectorSource from "ol/source/Vector";
import { Fill, Stroke, Style, Text } from "ol/style";
import type { FeatureLike } from "ol/Feature";
import type Geometry from "ol/geom/Geometry";
import type OlMap from "ol/Map";

// Eigene Antragsdaten aus der lokalen SQLite-Datei (Rust: src-tauri/src/antraege/)

/** Anträge werden erst ab dieser Auflösung (m/Pixel, ca. 1:36.000) geladen */
export const ANTRAEGE_MAX_RESOLUTION = 10;
/** Höchstzahl je Abfrage */
const LIMIT = 3000;
/** Danach wird der Zwischenspeicher geleert, damit der Speicher nicht volläuft */
const CACHE_MAX = 30000;

export interface AntraegeStatus {
  anzahl: number;
  ohne_geometrie: number;
  quelle: string;
  /** Vollständiger Pfad der zuletzt importierten Datei */
  quelle_pfad: string | null;
  /** Unix-Zeit in Sekunden */
  importiert_am: number;
}

export interface ImportProgress {
  phase: "lesen" | "index" | "kopieren";
  done: number;
  total: number;
  rows: number;
}

interface AntragFeature {
  id: number;
  wkb: string;
  antragsnummer: string | null;
  antragsart: string | null;
  datum_eingang: string | null;
}

const COLOR = "#c2185b";
const baseStyle = new Style({
  stroke: new Stroke({ color: COLOR, width: 1.5 }),
  fill: new Fill({ color: "rgba(194,24,91,0.10)" }),
});

function style(feature: FeatureLike, resolution: number) {
  // Antragsnummer erst bei starkem Zoom beschriften
  baseStyle.setText(
    resolution < 0.6
      ? new Text({
          text: String(feature.get("antragsnummer") ?? ""),
          font: "11px sans-serif",
          fill: new Fill({ color: COLOR }),
          stroke: new Stroke({ color: "#fff", width: 3 }),
          overflow: true,
        })
      : null,
  );
  return baseStyle;
}

function base64ToBytes(b64: string) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/**
 * Karten-Layer der Anträge. Lädt bei jeder Kartenbewegung den sichtbaren Ausschnitt nach.
 */
export class AntraegeLayer {
  readonly source = new VectorSource();
  readonly layer = new VectorLayer({
    source: this.source,
    style,
    maxResolution: ANTRAEGE_MAX_RESOLUTION,
    visible: false,
    zIndex: 50,
  });

  private format = new WKB();
  private requestNo = 0;

  /** `onHint` zeigt Hinweise an (z. B. "zu viele Anträge – hineinzoomen") */
  constructor(private map: OlMap, private onHint: (text: string) => void) {
    this.layer.set("title", "Anträge");
    map.addLayer(this.layer);
    map.on("moveend", () => this.reload());
    this.layer.on("change:visible", () => this.reload());
  }

  /** Nach einem Import: alles verwerfen und neu laden */
  refresh() {
    this.source.clear();
    this.reload();
  }

  private async reload() {
    if (!this.layer.getVisible() || !isTauri()) return;
    const view = this.map.getView();
    if ((view.getResolution() ?? Infinity) > ANTRAEGE_MAX_RESOLUTION) {
      this.onHint("Zum Anzeigen hineinzoomen (ab ca. 1:36.000).");
      return;
    }

    const extent = view.calculateExtent(this.map.getSize());
    const myRequest = ++this.requestNo;
    try {
      const result = await invoke<{ features: AntragFeature[]; truncated: boolean }>(
        "antraege_in_extent",
        { extent, limit: LIMIT },
      );
      // Inzwischen weiter verschoben? Dann ist diese Antwort veraltet.
      if (myRequest !== this.requestNo) return;

      if (this.source.getFeatures().length + result.features.length > CACHE_MAX) {
        this.source.clear();
      }
      const fresh = result.features
        .filter((a) => !this.source.getFeatureById(a.id))
        .map((a) => {
          const f = new Feature({
            geometry: this.format.readGeometry(base64ToBytes(a.wkb)) ?? undefined,
            antragsnummer: a.antragsnummer,
            antragsart: a.antragsart,
            datum_eingang: a.datum_eingang,
          });
          f.setId(a.id);
          return f;
        });
      this.source.addFeatures(fresh);
      this.onHint(
        result.truncated
          ? `Mehr als ${LIMIT.toLocaleString("de-DE")} Anträge im Ausschnitt – für alle hineinzoomen.`
          : "",
      );
    } catch (err) {
      console.error(err);
      this.onHint(`Fehler beim Laden: ${err}`);
    }
  }
}

// ---------- Rust-Befehle ----------

export async function getStatus(): Promise<AntraegeStatus | null> {
  if (!isTauri()) return null;
  return invoke<AntraegeStatus | null>("antraege_status");
}

/** Ruft einen Import-Befehl auf und leitet die Fortschrittsmeldungen weiter */
async function withProgress<T>(command: string, onProgress: (p: ImportProgress) => void) {
  if (!isTauri()) throw new Error("Der Import funktioniert nur in der Desktop-App.");
  const unlisten = await listen<ImportProgress>("antraege-import-progress", (e) =>
    onProgress(e.payload),
  );
  try {
    return await invoke<T>(command);
  } finally {
    unlisten();
  }
}

/** Öffnet den Dateidialog und importiert. `null` = abgebrochen. */
export function importAntraege(onProgress: (p: ImportProgress) => void) {
  return withProgress<AntraegeStatus | null>("import_antraege", onProgress);
}

/** Importiert erneut aus der zuletzt verwendeten Quelldatei */
export function updateAntraege(onProgress: (p: ImportProgress) => void) {
  return withProgress<AntraegeStatus>("antraege_update", onProgress);
}

export interface UpdateInfo {
  pfad: string;
  /** Änderungszeit der Quelldatei (Unix-Sekunden) */
  geaendert: number;
}

/** Ist die zuletzt importierte Quelldatei inzwischen neuer? `null` = nein */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  return invoke<UpdateInfo | null>("antraege_update_check");
}

/**
 * Räumliche Suche: alle Anträge, die eine der Geometrien schneiden.
 * Die exakte Prüfung macht Rust (geo-Bibliothek); Ergebnis wie bei der WFS-Suche.
 */
export async function searchAntraege(
  geometries: Geometry[],
  limit: number,
): Promise<{ features: Feature[]; truncated: boolean }> {
  if (!isTauri()) throw new Error("Die Antragssuche funktioniert nur in der Desktop-App.");
  // Suchgeometrien als WKB-Hex an Rust (ohne SRID – alles ist EPSG:25832)
  const writer = new WKB({ hex: true, littleEndian: true, ewkb: false });
  const reader = new WKB();
  const result = await invoke<{
    features: { id: number; wkb: string; props: [string, string][] }[];
    truncated: boolean;
  }>("antraege_search", {
    geometries: geometries.map((g) => writer.writeGeometry(g) as string),
    limit,
  });

  const features = result.features.map((hit) => {
    const f = new Feature({ geometry: reader.readGeometry(base64ToBytes(hit.wkb)) ?? undefined });
    hit.props.forEach(([key, value]) => f.set(key, value));
    f.setId(hit.id);
    return f;
  });
  return { features, truncated: result.truncated };
}

/** Alle Sachdaten eines Antrags als [Feld, Wert]-Paare */
export function getDetails(id: number) {
  return invoke<[string, string][]>("antrag_details", { id });
}

export function formatStatus(s: AntraegeStatus) {
  const date = new Date(s.importiert_am * 1000).toLocaleString("de-DE", {
    dateStyle: "medium",
    timeStyle: "short",
  });
  return `${s.anzahl.toLocaleString("de-DE")} Anträge · importiert ${date} aus „${s.quelle}“`;
}
