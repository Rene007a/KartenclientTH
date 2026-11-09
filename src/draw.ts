import type Map from "ol/Map";
import type Feature from "ol/Feature";
import VectorLayer from "ol/layer/Vector";
import VectorSource from "ol/source/Vector";
import Draw, { createBox } from "ol/interaction/Draw";
import type { Type as DrawType } from "ol/geom/Geometry";
import type Geometry from "ol/geom/Geometry";
import Circle from "ol/geom/Circle";
import { fromCircle } from "ol/geom/Polygon";
import { Circle as CircleStyle, Fill, Stroke, Style } from "ol/style";

export type DrawTool = "Point" | "LineString" | "Polygon" | "Box" | "Circle";

const DRAW_COLOR = "#ef6c00";

const drawStyle = new Style({
  stroke: new Stroke({ color: DRAW_COLOR, width: 2, lineDash: [6, 4] }),
  fill: new Fill({ color: "rgba(239,108,0,0.12)" }),
  image: new CircleStyle({
    radius: 6,
    fill: new Fill({ color: DRAW_COLOR }),
    stroke: new Stroke({ color: "#fff", width: 2 }),
  }),
});

/**
 * Verwaltet die gezeichneten Suchgeometrien und die Zeichen-Interaktion.
 */
export class DrawManager {
  readonly source = new VectorSource();
  readonly layer = new VectorLayer({ source: this.source, style: drawStyle, zIndex: 100 });

  private draw: Draw | null = null;
  private tool: DrawTool | null = null;
  private listeners: Array<() => void> = [];

  constructor(private map: Map) {
    map.addLayer(this.layer);
    this.source.on(["addfeature", "removefeature", "clear"], () => this.emit());

    // Esc bricht das Zeichnen ab
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") this.setTool(null);
    });
  }

  /** Aktives Werkzeug (null = Zeichnen aus) */
  get activeTool() {
    return this.tool;
  }

  get count() {
    return this.source.getFeatures().length;
  }

  setTool(tool: DrawTool | null) {
    if (this.draw) {
      this.map.removeInteraction(this.draw);
      this.draw = null;
    }
    // Erneuter Klick auf dasselbe Werkzeug schaltet es aus
    this.tool = tool === this.tool ? null : tool;

    if (this.tool) {
      const type: DrawType = this.tool === "Box" ? "Circle" : this.tool;
      this.draw = new Draw({
        source: this.source,
        type,
        geometryFunction: this.tool === "Box" ? createBox() : undefined,
        style: drawStyle,
      });
      this.map.addInteraction(this.draw);
    }
    this.emit();
  }

  undo() {
    const features = this.source.getFeatures();
    const last = features[features.length - 1];
    if (last) this.source.removeFeature(last);
  }

  clear() {
    this.source.clear();
  }

  /**
   * Geometrien für den räumlichen Filter.
   * Kreise gibt es in GML nicht – sie werden in Polygone (64 Ecken) umgewandelt.
   */
  getSearchGeometries(): Geometry[] {
    return this.source
      .getFeatures()
      .map((f: Feature) => f.getGeometry())
      .filter((g): g is Geometry => !!g)
      .map((g) => (g instanceof Circle ? fromCircle(g, 64) : g));
  }

  /** Wird bei jeder Änderung (Werkzeug, Zeichnungen) aufgerufen */
  onChange(fn: () => void) {
    this.listeners.push(fn);
  }

  private emit() {
    this.listeners.forEach((fn) => fn());
  }
}
