import "ol/ol.css";
import "./styles.css";

import Map from "ol/Map";
import View from "ol/View";
import { ScaleLine, MousePosition, defaults as defaultControls } from "ol/control";
import type BaseLayer from "ol/layer/Base";
import type VectorLayer from "ol/layer/Vector";

import { projection, THURINGIA_EXTENT, ERFURT } from "./projection";
import { BASE_LAYERS, WMS_LAYERS, WFS_LAYERS } from "./services";
import { createBaseLayer, createWmsLayer, createWfsLayer } from "./layers";
import { DrawManager, type DrawTool } from "./draw";
import { ResultsPanel } from "./results";
import { searchWfs } from "./search";
import { exportPdf, exportTable } from "./export";
import type Feature from "ol/Feature";

// ---------- Layer anlegen ----------
const baseLayers = BASE_LAYERS.map(createBaseLayer);
const wmsLayers = WMS_LAYERS.map(createWmsLayer);
const wfsLayers = WFS_LAYERS.map(createWfsLayer);
baseLayers[0].setVisible(true);

// ---------- Karte ----------
const coordEl = document.getElementById("coords")!;

const map = new Map({
  target: "map",
  layers: [...baseLayers, ...wmsLayers, ...wfsLayers],
  controls: defaultControls().extend([
    new ScaleLine({ units: "metric" }),
    new MousePosition({
      projection,
      target: coordEl,
      className: "mouse-position",
      coordinateFormat: (c) =>
        c ? `EPSG:25832  R ${c[0].toFixed(1)}  H ${c[1].toFixed(1)}` : "",
      placeholder: "EPSG:25832",
    }),
  ]),
  view: new View({
    projection, // Kartenprojektion = EPSG:25832
    center: ERFURT,
    zoom: 9,
    extent: [THURINGIA_EXTENT[0] - 100000, THURINGIA_EXTENT[1] - 100000,
             THURINGIA_EXTENT[2] + 100000, THURINGIA_EXTENT[3] + 100000],
  }),
});

map.getView().fit(THURINGIA_EXTENT, { padding: [20, 20, 20, 20] });

// ---------- Layer-Steuerung (Sidebar) ----------
function addRadio(container: HTMLElement, layer: BaseLayer, title: string) {
  const label = document.createElement("label");
  const input = document.createElement("input");
  input.type = "radio";
  input.name = "base";
  input.checked = layer.getVisible();
  input.addEventListener("change", () =>
    baseLayers.forEach((l) => l.setVisible(l === layer)),
  );
  label.append(input, " ", title);
  container.append(label);
}

function addCheckbox(container: HTMLElement, layer: BaseLayer, title: string, hint?: string) {
  const label = document.createElement("label");
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = layer.getVisible();
  input.addEventListener("change", () => layer.setVisible(input.checked));
  label.append(input, " ", title);
  if (hint) {
    const small = document.createElement("small");
    small.textContent = hint;
    label.append(small);
  }
  container.append(label);
}

const baseEl = document.getElementById("base-layers")!;
BASE_LAYERS.forEach((cfg, i) => addRadio(baseEl, baseLayers[i], cfg.title));

const wmsEl = document.getElementById("wms-layers")!;
WMS_LAYERS.forEach((cfg, i) =>
  addCheckbox(wmsEl, wmsLayers[i], cfg.title,
    cfg.maxResolution ? `sichtbar ab ca. 1:${scaleFor(cfg.maxResolution)}` : undefined),
);

const wfsEl = document.getElementById("wfs-layers")!;
WFS_LAYERS.forEach((cfg, i) =>
  addCheckbox(wfsEl, wfsLayers[i], cfg.title, `lädt ab ca. 1:${scaleFor(cfg.maxResolution)}`),
);

/** Grober Maßstab zu einer Auflösung (0,28 mm Pixelgröße nach OGC) */
function scaleFor(resolution: number) {
  return Math.round(resolution / 0.00028).toLocaleString("de-DE");
}

// ---------- Sachdaten per Klick auf WFS-Objekte ----------
const infoEl = document.getElementById("info")!;

map.on("singleclick", (evt) => {
  // Beim Zeichnen gehört der Klick dem Zeichenwerkzeug
  if (draw.activeTool) return;

  // Klick auf einen Suchtreffer markiert die Tabellenzeile
  const hit = map.forEachFeatureAtPixel(evt.pixel, (f) => f, {
    layerFilter: (l) => l === results.layer,
  });
  if (hit) results.select(hit as Feature);

  const hits: string[] = [];
  map.forEachFeatureAtPixel(
    evt.pixel,
    (feature, layer) => {
      // Geometrien sind Objekte und werden durch den Filter ausgeblendet
      const rows = Object.entries(feature.getProperties())
        .filter(([, v]) => v !== undefined && v !== null && typeof v !== "object")
        .map(([k, v]) => `<tr><th>${escapeHtml(k)}</th><td>${escapeHtml(String(v))}</td></tr>`)
        .join("");
      hits.push(`<h3>${escapeHtml((layer as VectorLayer).get("title"))}</h3><table>${rows}</table>`);
    },
    { layerFilter: (l) => wfsLayers.includes(l as VectorLayer) || l === results.layer },
  );
  infoEl.innerHTML = hits.length
    ? hits.join("")
    : "<em>Kein WFS-Objekt an dieser Stelle.</em>";
});

function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

// Ladeanzeige für WFS
let loading = 0;
const statusEl = document.getElementById("status")!;
wfsLayers.forEach((l) => {
  const src = l.getSource()!;
  src.on("featuresloadstart", () => { loading++; update(); });
  src.on(["featuresloadend", "featuresloaderror"], (e) => {
    loading = Math.max(0, loading - 1);
    if (e.type === "featuresloaderror") {
      statusEl.textContent = `WFS-Fehler beim Laden: ${l.get("title")}`;
    } else {
      update();
    }
  });
});
function update() {
  statusEl.textContent = loading > 0 ? `Lade WFS-Daten … (${loading})` : "";
}

// ---------- Zeichnen & Suchen ----------
const draw = new DrawManager(map);
const results = new ResultsPanel(map, document.getElementById("results")!);

const toolButtons = document.querySelectorAll<HTMLButtonElement>("#draw-tools button");
const undoBtn = document.getElementById("draw-undo") as HTMLButtonElement;
const clearBtn = document.getElementById("draw-clear") as HTMLButtonElement;
const searchBtn = document.getElementById("search-run") as HTMLButtonElement;
const searchSelect = document.getElementById("search-layer") as HTMLSelectElement;
const drawStatusEl = document.getElementById("draw-status")!;
const searchErrorEl = document.getElementById("search-error")!;

toolButtons.forEach((btn) =>
  btn.addEventListener("click", () => draw.setTool(btn.dataset.tool as DrawTool)),
);
undoBtn.addEventListener("click", () => draw.undo());
clearBtn.addEventListener("click", () => draw.clear());

WFS_LAYERS.forEach((cfg) => searchSelect.add(new Option(cfg.title, cfg.id)));

const TOOL_HINTS: Record<DrawTool, string> = {
  Point: "Klicken setzt Punkte – jedes getroffene Objekt wird gefunden.",
  LineString: "Klicken setzt Stützpunkte, Doppelklick beendet die Linie.",
  Polygon: "Klicken setzt Ecken, Doppelklick schließt das Polygon.",
  Box: "Klicken, ziehen, erneut klicken.",
  Circle: "Mittelpunkt klicken, Radius ziehen, erneut klicken.",
};

function updateDrawUi() {
  toolButtons.forEach((btn) =>
    btn.classList.toggle("active", btn.dataset.tool === draw.activeTool),
  );
  const n = draw.count;
  undoBtn.disabled = clearBtn.disabled = n === 0;
  searchBtn.disabled = n === 0;
  const parts = [];
  if (draw.activeTool) parts.push(TOOL_HINTS[draw.activeTool] + " Esc beendet.");
  parts.push(n ? `${n} Suchgeometrie${n > 1 ? "n" : ""} gezeichnet.` : "Noch nichts gezeichnet.");
  drawStatusEl.textContent = parts.join(" ");
}
draw.onChange(updateDrawUi);
updateDrawUi();

searchBtn.addEventListener("click", async () => {
  const cfg = WFS_LAYERS.find((c) => c.id === searchSelect.value)!;
  searchBtn.disabled = true;
  searchBtn.textContent = "Suche läuft …";
  searchErrorEl.textContent = "";
  try {
    const { features, truncated } = await searchWfs(cfg, draw.getSearchGeometries());
    results.show(cfg.title, features, truncated);
  } catch (err) {
    console.error(err);
    searchErrorEl.textContent = err instanceof Error ? err.message : String(err);
  } finally {
    searchBtn.textContent = "Objekte im Suchbereich suchen";
    updateDrawUi();
  }
});

// ---------- Export (CSV / Excel / PDF) ----------
const exportMsgEl = document.getElementById("export-msg")!;
const exportButtons = document.querySelectorAll<HTMLButtonElement>("#results [data-export]");

exportButtons.forEach((btn) =>
  btn.addEventListener("click", async () => {
    if (results.isEmpty) return;
    const format = btn.dataset.export as "csv" | "xlsx" | "pdf";
    const table = results.getTable();

    exportButtons.forEach((b) => (b.disabled = true));
    exportMsgEl.textContent = format === "pdf" ? "PDF wird erstellt …" : "";
    try {
      const path =
        format === "pdf" ? await exportPdf(map, table) : await exportTable(format, table);
      exportMsgEl.textContent = path ? `Gespeichert: ${path}` : "";
    } catch (err) {
      console.error(err);
      exportMsgEl.textContent = `Fehler: ${err instanceof Error ? err.message : String(err)}`;
    } finally {
      exportButtons.forEach((b) => (b.disabled = false));
    }
  }),
);
