import proj4 from "proj4";
import { register } from "ol/proj/proj4";
import { get as getProjection } from "ol/proj";

// ETRS89 / UTM Zone 32N – amtliches Lagebezugssystem in Thüringen
export const EPSG_CODE = "EPSG:25832";

proj4.defs(
  EPSG_CODE,
  "+proj=utm +zone=32 +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs +type=crs",
);
// OpenLayers mit proj4 bekannt machen (ermöglicht Umprojektion, z. B. von OSM)
register(proj4);

export const projection = getProjection(EPSG_CODE)!;

// Gültigkeitsbereich der Projektion (Deutschland grob, in Metern)
projection.setExtent([280000, 5200000, 920000, 6110000]);

// Ausdehnung Thüringens in EPSG:25832 (etwas großzügig)
export const THURINGIA_EXTENT = [555000, 5560000, 775000, 5730000];

// Erfurt als Startpunkt
export const ERFURT = [642500, 5649000];
