import { invoke, isTauri } from "@tauri-apps/api/core";
import { jsPDF } from "jspdf";
import autoTable from "jspdf-autotable";
import type OlMap from "ol/Map";

import { renderMapImage } from "./map-image";

export interface TableExport {
  /** z. B. "Flurstücke (WFS)" */
  title: string;
  columns: string[];
  rows: string[][];
}

/** Ergebnis eines Exports: gespeicherter Pfad oder null (Dialog abgebrochen) */
type SavedPath = string | null;

function requireTauri() {
  if (!isTauri()) {
    throw new Error("Der Export funktioniert nur in der Desktop-App (npm run tauri dev).");
  }
}

/** Dateiname ohne verbotene Zeichen, mit Datum: "Flurstücke_WFS_2026-11-09" */
function defaultFileName(title: string) {
  const date = new Date().toISOString().slice(0, 10);
  const base = title.replace(/[()]/g, "").trim().replace(/[\\/:*?"<>|\s]+/g, "_");
  return `${base}_${date}`;
}

/** CSV oder Excel – die Datei schreibt Rust (src-tauri/src/export.rs) */
export async function exportTable(format: "csv" | "xlsx", table: TableExport): Promise<SavedPath> {
  requireTauri();
  return invoke<SavedPath>("export_table", {
    format,
    fileName: `${defaultFileName(table.title)}.${format}`,
    sheetName: table.title,
    table: { columns: table.columns, rows: table.rows },
  });
}

/** PDF mit Kartenbild (Seite 1) und Ergebnistabelle (folgende Seiten) */
export async function exportPdf(map: OlMap, table: TableExport): Promise<SavedPath> {
  requireTauri();

  const image = await renderMapImage(map);
  const doc = new jsPDF({ orientation: "landscape", unit: "mm", format: "a4" });
  const pageW = doc.internal.pageSize.getWidth();
  const pageH = doc.internal.pageSize.getHeight();
  const margin = 10;

  // --- Seite 1: Kopf + Karte ---
  const resolution = map.getView().getResolution() ?? 0;
  const scale = Math.round(resolution / 0.00028 / 100) * 100; // OGC-Standardpixel 0,28 mm
  const now = new Date().toLocaleString("de-DE");

  doc.setFontSize(14);
  doc.text(table.title, margin, margin + 4);
  doc.setFontSize(9);
  doc.setTextColor(90);
  doc.text(
    `${table.rows.length} Objekte · Bezugssystem ETRS89 / UTM 32N (EPSG:25832) · ` +
      `Bildschirmmaßstab ca. 1:${scale.toLocaleString("de-DE")} · erstellt ${now}`,
    margin,
    margin + 10,
  );

  // Karte seitenfüllend, Seitenverhältnis beibehalten
  const top = margin + 14;
  const maxW = pageW - 2 * margin;
  const maxH = pageH - top - margin - 8;
  const factor = Math.min(maxW / image.width, maxH / image.height);
  const imgW = image.width * factor;
  const imgH = image.height * factor;
  doc.addImage(image.dataUrl, "JPEG", margin, top, imgW, imgH);
  doc.setDrawColor(180);
  doc.rect(margin, top, imgW, imgH);

  // Quellenvermerk (Pflicht laut Lizenz dl-de/by-2-0)
  const attribution = [...(map.getViewport()?.querySelectorAll(".ol-attribution li") ?? [])]
    .map((li) => li.textContent?.trim())
    .filter(Boolean)
    .join(" · ");
  doc.setFontSize(7);
  doc.text(`Quellen: ${attribution}`, margin, top + imgH + 4, { maxWidth: maxW });

  // --- Folgeseiten: Tabelle ---
  doc.addPage();
  doc.setTextColor(0);
  autoTable(doc, {
    head: [table.columns],
    body: table.rows,
    margin: { top: margin, left: margin, right: margin, bottom: margin + 4 },
    styles: { fontSize: 7, cellPadding: 1, overflow: "linebreak" },
    headStyles: { fillColor: [0, 102, 140] },
    alternateRowStyles: { fillColor: [246, 247, 249] },
    // Zu viele Spalten? Dann rechts auf weiteren Seiten fortsetzen
    horizontalPageBreak: true,
    horizontalPageBreakRepeat: 0,
  });

  // Seitenzahlen
  const pages = doc.getNumberOfPages();
  for (let i = 1; i <= pages; i++) {
    doc.setPage(i);
    doc.setFontSize(7);
    doc.setTextColor(120);
    doc.text(`Seite ${i} von ${pages}`, pageW - margin, pageH - 5, { align: "right" });
  }

  const bytes = new Uint8Array(doc.output("arraybuffer"));
  // Binärdaten direkt (ohne JSON) an Rust; Metadaten als Header (nur ASCII erlaubt)
  return invoke<SavedPath>("save_binary", bytes, {
    headers: {
      "x-file-name": encodeURIComponent(`${defaultFileName(table.title)}.pdf`),
      "x-file-ext": "pdf",
      "x-file-filter": encodeURIComponent("PDF-Dokument"),
    },
  });
}
