import type OlMap from "ol/Map";

export interface MapImage {
  dataUrl: string;
  width: number;
  height: number;
}

/**
 * Setzt alle Layer-Canvases der Karte zu einem Bild zusammen
 * (nach dem OpenLayers-Beispiel "export-pdf").
 * Wartet, bis alle Kacheln geladen sind ("rendercomplete").
 */
export function renderMapImage(map: OlMap, timeoutMs = 20000): Promise<MapImage> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("Zeitüberschreitung beim Laden der Karte")),
      timeoutMs,
    );

    map.once("rendercomplete", () => {
      clearTimeout(timer);
      try {
        resolve(composeCanvas(map));
      } catch (err) {
        // SecurityError, falls ein Kartendienst kein CORS erlaubt
        reject(new Error(`Kartenbild konnte nicht erzeugt werden: ${err}`));
      }
    });
    map.renderSync();
  });
}

function composeCanvas(map: OlMap): MapImage {
  const [width, height] = map.getSize()!;
  const ratio = window.devicePixelRatio || 1;

  const out = document.createElement("canvas");
  out.width = Math.round(width * ratio);
  out.height = Math.round(height * ratio);
  const ctx = out.getContext("2d")!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, out.width, out.height);

  const viewport = map.getViewport();
  if (!viewport) throw new Error("Karte ist nicht angezeigt");
  const canvases = viewport.querySelectorAll<HTMLCanvasElement>(
    ".ol-layer canvas, canvas.ol-layer",
  );

  canvases.forEach((canvas) => {
    if (canvas.width === 0) return;
    const parent = canvas.parentElement as HTMLElement;
    const opacity = parent.style.opacity || canvas.style.opacity;
    ctx.globalAlpha = opacity === "" ? 1 : Number(opacity);

    // Jede Layer-Canvas trägt ihre Lage als CSS-Transformation
    const match = canvas.style.transform.match(/^matrix\(([^(]*)\)$/);
    const matrix = match
      ? match[1].split(",").map(Number)
      : [
          parseFloat(canvas.style.width) / canvas.width, 0, 0,
          parseFloat(canvas.style.height) / canvas.height, 0, 0,
        ];
    // Auf die Bildauflösung (devicePixelRatio) hochskalieren
    ctx.setTransform(
      matrix[0] * ratio, matrix[1] * ratio, matrix[2] * ratio,
      matrix[3] * ratio, matrix[4] * ratio, matrix[5] * ratio,
    );

    if (parent.style.backgroundColor) {
      ctx.fillStyle = parent.style.backgroundColor;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
    }
    ctx.drawImage(canvas, 0, 0);
  });

  ctx.globalAlpha = 1;
  ctx.setTransform(1, 0, 0, 1, 0, 0);

  return { dataUrl: out.toDataURL("image/jpeg", 0.92), width: out.width, height: out.height };
}
