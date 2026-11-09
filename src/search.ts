import type Feature from "ol/Feature";
import type Geometry from "ol/geom/Geometry";
import WFS from "ol/format/WFS";
import { intersects, or } from "ol/format/filter";

import { EPSG_CODE } from "./projection";
import type { WfsLayerConfig } from "./services";

export interface SearchResult {
  features: Feature[];
  /** true, wenn die Höchstzahl erreicht wurde und es vermutlich weitere Treffer gibt */
  truncated: boolean;
}

/** Höchstzahl Treffer je Suche */
export const MAX_SEARCH_RESULTS = 1000;

/**
 * Sucht alle Objekte eines WFS-Layers, die mindestens eine der Geometrien schneiden.
 * Schickt eine WFS-2.0-GetFeature-Anfrage per POST mit räumlichem Filter (fes:Intersects).
 */
export async function searchWfs(
  cfg: WfsLayerConfig,
  geometries: Geometry[],
  maxFeatures = MAX_SEARCH_RESULTS,
): Promise<SearchResult> {
  if (geometries.length === 0) throw new Error("Keine Suchgeometrie gezeichnet.");

  const filters = geometries.map((g) => intersects(cfg.geometryName, g, EPSG_CODE));
  // "or" braucht mindestens zwei Bedingungen
  const filter = filters.length === 1 ? filters[0] : or(...filters);

  const [featurePrefix, featureType] = cfg.typeName.split(":");
  const format = new WFS({ version: "2.0.0" });
  const request = format.writeGetFeature({
    srsName: EPSG_CODE,
    featureNS: cfg.featureNS,
    featurePrefix,
    featureTypes: [featureType],
    count: maxFeatures,
    filter,
  });

  // Content-Type "text/plain" statt "text/xml": So gilt die Anfrage als CORS-"simple request"
  // und der Browser schickt keinen Preflight (OPTIONS) voraus. Der TLBG-Geoproxy beantwortet
  // den Preflight ohne Access-Control-Allow-Headers – mit text/xml würde die Anfrage blockiert.
  // Der Server liest den XML-Body trotzdem korrekt.
  let response: Response;
  try {
    response = await fetch(cfg.url, {
      method: "POST",
      headers: { "Content-Type": "text/plain;charset=UTF-8" },
      body: new XMLSerializer().serializeToString(request),
    });
  } catch (err) {
    throw new Error(`Keine Verbindung zum WFS (${cfg.url}): ${err}`);
  }
  const text = await response.text();
  const doc = new DOMParser().parseFromString(text, "application/xml");
  const root = doc.documentElement;

  // OGC-Dienste melden Fehler als ExceptionReport (oft trotzdem mit HTTP 200)
  if (!response.ok || root.localName === "ExceptionReport") {
    const msg = root.textContent?.trim().replace(/\s+/g, " ").slice(0, 300);
    throw new Error(`WFS-Fehler (HTTP ${response.status}): ${msg || "unbekannt"}`);
  }

  const features = format.readFeatures(doc) as Feature[];
  // Achtung: Dieser Dienst meldet bei Begrenzung numberMatched = Anzahl geliefert,
  // daher wird "abgeschnitten" an der Höchstzahl erkannt.
  return { features, truncated: features.length >= maxFeatures };
}
