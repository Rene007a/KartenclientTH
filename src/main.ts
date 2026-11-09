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
    { layerFilter: (l) => wfsLayers.includes(l as VectorLayer) },
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
