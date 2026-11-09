import TileLayer from "ol/layer/Tile";
import VectorLayer from "ol/layer/Vector";
import OSM from "ol/source/OSM";
import TileWMS from "ol/source/TileWMS";
import VectorSource from "ol/source/Vector";
import WFS from "ol/format/WFS";
import { bbox as bboxStrategy } from "ol/loadingstrategy";
import { Fill, Stroke, Style, Text } from "ol/style";
import type { FeatureLike } from "ol/Feature";
import type BaseLayer from "ol/layer/Base";

import { EPSG_CODE } from "./projection";
import type {
  BaseLayerConfig,
  WfsLayerConfig,
  WmsLayerConfig,
} from "./services";

const SRS_URN = "urn:ogc:def:crs:EPSG::25832";

function wmsSource(url: string, layers: string, attribution: string) {
  return new TileWMS({
    url,
    params: { LAYERS: layers, FORMAT: "image/png", TRANSPARENT: true, VERSION: "1.3.0" },
    projection: EPSG_CODE,
    attributions: attribution,
    // Nötig, damit die Karte als Bild exportiert werden kann (PDF)
    crossOrigin: "anonymous",
  });
}

export function createBaseLayer(cfg: BaseLayerConfig): BaseLayer {
  const source =
    cfg.kind === "osm"
      ? new OSM({ crossOrigin: "anonymous" }) // liegt in EPSG:3857 vor – OpenLayers projiziert live nach 25832
      : wmsSource(cfg.url!, cfg.layers!, cfg.attribution);
  const layer = new TileLayer({ source, visible: false });
  layer.set("id", cfg.id);
  return layer;
}

export function createWmsLayer(cfg: WmsLayerConfig): BaseLayer {
  const layer = new TileLayer({
    source: wmsSource(cfg.url, cfg.layers, cfg.attribution),
    opacity: cfg.opacity ?? 1,
    maxResolution: cfg.maxResolution,
    visible: false,
  });
  layer.set("id", cfg.id);
  return layer;
}

export function createWfsLayer(cfg: WfsLayerConfig): VectorLayer {
  const source = new VectorSource({
    // WFS 2.0 liefert hier GML 3.2 – der WFS-Parser von OpenLayers liest das direkt
    format: new WFS({ version: "2.0.0" }),
    attributions: cfg.attribution,
    url: (extent) => {
      const params = new URLSearchParams({
        SERVICE: "WFS",
        VERSION: "2.0.0",
        REQUEST: "GetFeature",
        TYPENAMES: cfg.typeName,
        COUNT: String(cfg.maxFeatures),
        SRSNAME: SRS_URN,
        BBOX: `${extent.map((v) => Math.round(v)).join(",")},${SRS_URN}`,
      });
      return `${cfg.url}?${params.toString()}`;
    },
    // Lädt nur Objekte im aktuellen Kartenausschnitt nach
    strategy: bboxStrategy,
  });

  const baseStyle = new Style({
    stroke: new Stroke({ color: cfg.stroke, width: 1.5 }),
    fill: new Fill({ color: cfg.fill }),
  });

  const style = cfg.labelField
    ? (feature: FeatureLike) => {
        baseStyle.setText(
          new Text({
            text: String(feature.get(cfg.labelField!) ?? ""),
            font: "12px sans-serif",
            fill: new Fill({ color: cfg.stroke }),
            stroke: new Stroke({ color: "#fff", width: 3 }),
            overflow: true,
          }),
        );
        return baseStyle;
      }
    : baseStyle;

  const layer = new VectorLayer({
    source,
    style,
    maxResolution: cfg.maxResolution,
    visible: false,
  });
  layer.set("id", cfg.id);
  layer.set("title", cfg.title);
  return layer;
}
