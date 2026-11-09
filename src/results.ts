// "OlMap", damit es nicht mit der eingebauten JavaScript-Map kollidiert
import type OlMap from "ol/Map";
import type Feature from "ol/Feature";
import VectorLayer from "ol/layer/Vector";
import VectorSource from "ol/source/Vector";
import { Circle as CircleStyle, Fill, Stroke, Style } from "ol/style";

const normalStyle = new Style({
  stroke: new Stroke({ color: "#f9a825", width: 2 }),
  fill: new Fill({ color: "rgba(255,235,59,0.35)" }),
});
const selectedStyle = new Style({
  stroke: new Stroke({ color: "#e65100", width: 4 }),
  fill: new Fill({ color: "rgba(255,152,0,0.45)" }),
  image: new CircleStyle({ radius: 6, fill: new Fill({ color: "#e65100" }) }),
  zIndex: 1,
});

/**
 * Ergebnisliste unter der Karte + Hervorhebung der Treffer in der Karte.
 */
export class ResultsPanel {
  readonly source = new VectorSource();
  readonly layer: VectorLayer;

  private features: Feature[] = [];
  private columns: string[] = [];
  private selected: Feature | null = null;
  private sortCol: string | null = null;
  private sortAsc = true;
  private rowOf = new Map<Feature, HTMLTableRowElement>();
  private layerTitle = "";

  private titleEl: HTMLElement;
  private tableEl: HTMLTableElement;

  constructor(private map: OlMap, private panelEl: HTMLElement) {
    this.layer = new VectorLayer({
      source: this.source,
      style: (f) => (f === this.selected ? selectedStyle : normalStyle),
      zIndex: 90,
    });
    map.addLayer(this.layer);

    this.titleEl = panelEl.querySelector(".results-title")!;
    this.tableEl = panelEl.querySelector("table")!;
    panelEl.querySelector(".results-close")!.addEventListener("click", () => this.clear());
    panelEl.querySelector(".results-zoom")!.addEventListener("click", () => this.zoomToAll());
  }

  show(layerTitle: string, features: Feature[], truncated: boolean) {
    this.layer.set("title", `Suchtreffer – ${layerTitle}`);
    this.layerTitle = layerTitle;
    this.features = features;
    this.selected = null;
    this.sortCol = null;
    this.source.clear();
    this.source.addFeatures(features);

    // Spalten = alle einfachen Attribute (Geometrien sind Objekte und fallen heraus)
    const cols = new Set<string>();
    features.forEach((f) =>
      Object.entries(f.getProperties()).forEach(([k, v]) => {
        if (v !== null && v !== undefined && typeof v !== "object") cols.add(k);
      }),
    );
    this.columns = [...cols];

    let title = `${layerTitle}: ${features.length} Treffer`;
    if (truncated) {
      title += " – Höchstzahl erreicht, evtl. nicht alle Treffer. Suchbereich verkleinern.";
    }
    this.titleEl.textContent = title;

    this.renderTable();
    this.panelEl.hidden = false;
    this.map.updateSize();
    if (features.length) this.zoomToAll();
  }

  clear() {
    this.features = [];
    this.selected = null;
    this.source.clear();
    this.panelEl.hidden = true;
    this.map.updateSize();
  }

  /** Markiert ein Objekt in Tabelle und Karte */
  select(feature: Feature | null, zoom = false) {
    this.selected = feature;
    this.layer.changed();
    this.rowOf.forEach((tr, f) => tr.classList.toggle("selected", f === feature));
    if (feature && !zoom) this.rowOf.get(feature)?.scrollIntoView({ block: "nearest" });
    const geom = feature?.getGeometry();
    if (zoom && geom) {
      this.map.getView().fit(geom.getExtent(), {
        padding: [60, 60, 60, 60],
        maxZoom: 19,
        duration: 300,
      });
    }
  }

  /** Tabelleninhalt in der aktuell angezeigten Sortierung – für den Export */
  getTable() {
    return {
      title: this.layerTitle,
      columns: this.columns,
      rows: this.sortedFeatures().map((f) =>
        this.columns.map((c) => {
          const v = f.get(c);
          return v === undefined || v === null ? "" : String(v);
        }),
      ),
    };
  }

  get isEmpty() {
    return this.features.length === 0;
  }

  /** true, wenn das Objekt zur aktuellen Ergebnisliste gehört */
  contains(feature: unknown) {
    return this.features.includes(feature as Feature);
  }

  private zoomToAll() {
    const extent = this.source.getExtent();
    if (this.source.isEmpty() || !extent) return;
    this.map.getView().fit(extent, {
      padding: [40, 40, 40, 40],
      maxZoom: 18,
      duration: 300,
    });
  }

  private sortedFeatures() {
    const rows = [...this.features];
    if (this.sortCol) {
      const col = this.sortCol;
      const dir = this.sortAsc ? 1 : -1;
      rows.sort(
        (a, b) =>
          dir *
          String(a.get(col) ?? "").localeCompare(String(b.get(col) ?? ""), "de", {
            numeric: true,
          }),
      );
    }
    return rows;
  }

  private renderTable() {
    const thead = document.createElement("thead");
    const headRow = document.createElement("tr");
    for (const col of this.columns) {
      const th = document.createElement("th");
      th.textContent = col + (this.sortCol === col ? (this.sortAsc ? " ▲" : " ▼") : "");
      th.title = "Klicken zum Sortieren";
      th.addEventListener("click", () => {
        this.sortAsc = this.sortCol === col ? !this.sortAsc : true;
        this.sortCol = col;
        this.renderTable();
      });
      headRow.append(th);
    }
    thead.append(headRow);

    const rows = this.sortedFeatures();

    const tbody = document.createElement("tbody");
    this.rowOf.clear();
    for (const f of rows) {
      const tr = document.createElement("tr");
      this.rowOf.set(f, tr);
      if (f === this.selected) tr.classList.add("selected");
      for (const col of this.columns) {
        const td = document.createElement("td");
        const v = f.get(col);
        td.textContent = v === undefined || v === null ? "" : String(v);
        tr.append(td);
      }
      tr.addEventListener("click", () => this.select(f, true));
      tbody.append(tr);
    }

    if (rows.length === 0) {
      const tr = document.createElement("tr");
      tr.innerHTML = "<td><em>Keine Objekte im Suchbereich gefunden.</em></td>";
      tbody.append(tr);
    }

    this.tableEl.replaceChildren(thead, tbody);
  }
}
