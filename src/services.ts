// Zentrale Konfiguration der Geodienste.
// Neue Dienste einfach hier ergänzen – die Oberfläche baut sich daraus auf.
// Alle URLs/Layernamen wurden per GetCapabilities geprüft (Stand 11/2026).

const GEOPROXY = "https://www.geoproxy.geoportal-th.de/geoproxy/services";

export interface BaseLayerConfig {
  id: string;
  title: string;
  kind: "osm" | "wms";
  url?: string;
  layers?: string;
  attribution: string;
}

export interface WmsLayerConfig {
  id: string;
  title: string;
  url: string;
  layers: string;
  attribution: string;
  opacity?: number;
  /** Ab welcher Auflösung (m/Pixel) der Layer gezeichnet wird */
  maxResolution?: number;
}

export interface WfsLayerConfig {
  id: string;
  title: string;
  url: string;
  typeName: string;
  attribution: string;
  /** WFS-Abfragen erst ab dieser Auflösung (m/Pixel) – schützt vor Riesenabfragen */
  maxResolution: number;
  /** Maximale Anzahl Objekte je Abfrage */
  maxFeatures: number;
  stroke: string;
  fill: string;
  /** Attribut, das als Beschriftung verwendet wird */
  labelField?: string;
}

const TLBG = '© <a href="https://geoportal.thueringen.de">GDI-Th / TLBG</a>, dl-de/by-2-0';
const BKG = '© <a href="https://www.bkg.bund.de">BKG</a>, dl-de/by-2-0';

export const BASE_LAYERS: BaseLayerConfig[] = [
  {
    id: "topplus",
    title: "TopPlusOpen (BKG)",
    kind: "wms",
    url: "https://sgx.geodatenzentrum.de/wms_topplus_open",
    layers: "web",
    attribution: BKG,
  },
  {
    id: "topplus_grau",
    title: "TopPlusOpen grau (BKG)",
    kind: "wms",
    url: "https://sgx.geodatenzentrum.de/wms_topplus_open",
    layers: "web_grau",
    attribution: BKG,
  },
  {
    id: "osm",
    title: "OpenStreetMap",
    kind: "osm",
    attribution: "© OpenStreetMap-Mitwirkende",
  },
  {
    id: "dop",
    title: "Luftbild DOP20 Thüringen",
    kind: "wms",
    url: `${GEOPROXY}/DOP`,
    layers: "th_dop",
    attribution: TLBG,
  },
];

export const WMS_LAYERS: WmsLayerConfig[] = [
  {
    id: "dgm_schummerung",
    title: "Geländeschummerung (DGM2)",
    url: `${GEOPROXY}/DGM`,
    layers: "DGM2",
    attribution: TLBG,
    opacity: 0.5,
  },
  {
    id: "alkis_tn",
    title: "ALKIS Tatsächliche Nutzung",
    url: `${GEOPROXY}/adv_alkis_wms_th`,
    layers:
      "adv_alkis_gewaesser,adv_alkis_vegetation,adv_alkis_verkehr,adv_alkis_siedlung",
    attribution: TLBG,
    opacity: 0.7,
    maxResolution: 10,
  },
  {
    id: "alkis_flurstuecke",
    title: "ALKIS Flurstücke",
    url: `${GEOPROXY}/adv_alkis_wms_th`,
    layers: "adv_alkis_flurstuecke",
    attribution: TLBG,
    maxResolution: 5,
  },
  {
    id: "alkis_gebaeude",
    title: "ALKIS Gebäude",
    url: `${GEOPROXY}/adv_alkis_wms_th`,
    layers: "adv_alkis_gebaeude",
    attribution: TLBG,
    maxResolution: 5,
  },
];

// ALKIS vereinfacht (AdV-Profil) – liefert GML 3.2
export const WFS_LAYERS: WfsLayerConfig[] = [
  {
    id: "wfs_gemeinden",
    title: "Verwaltungseinheiten (WFS)",
    url: `${GEOPROXY}/adv_alkis_wfs`,
    typeName: "ave:VerwaltungsEinheit",
    attribution: TLBG,
    maxResolution: 60,
    maxFeatures: 500,
    stroke: "#7b1fa2",
    fill: "rgba(123,31,162,0.05)",
    labelField: "name",
  },
  {
    id: "wfs_flurstuecke",
    title: "Flurstücke (WFS)",
    url: `${GEOPROXY}/adv_alkis_wfs`,
    typeName: "ave:Flurstueck",
    attribution: TLBG,
    maxResolution: 1.5,
    maxFeatures: 2000,
    stroke: "#d32f2f",
    fill: "rgba(211,47,47,0.05)",
  },
  {
    id: "wfs_gebaeude",
    title: "Gebäude/Bauwerke (WFS)",
    url: `${GEOPROXY}/adv_alkis_wfs`,
    typeName: "ave:GebaeudeBauwerk",
    attribution: TLBG,
    maxResolution: 1.5,
    maxFeatures: 2000,
    stroke: "#1565c0",
    fill: "rgba(21,101,192,0.25)",
  },
];
