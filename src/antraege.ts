import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Feature from "ol/Feature";
import WKB from "ol/format/WKB";
import VectorLayer from "ol/layer/Vector";
import VectorSource from "ol/source/Vector";
import { Fill, Stroke, Style, Text } from "ol/style";
import type { FeatureLike } from "ol/Feature";
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

/** Öffnet den Dateidialog und importiert. `null` = abgebrochen. */
export async function importAntraege(
  onProgress: (p: ImportProgress) => void,
): Promise<AntraegeStatus | null> {
  if (!isTauri()) throw new Error("Der Import funktioniert nur in der Desktop-App.");
  const unlisten = await listen<ImportProgress>("antraege-import-progress", (e) =>
    onProgress(e.payload),
  );
  try {
    return await invoke<AntraegeStatus | null>("import_antraege");
  } finally {
    unlisten();
  }
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
