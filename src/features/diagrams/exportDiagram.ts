// SOT: diagram-export, diagram-png-raster, export-theme-colours, diagram-image-save
import type { ConnectionColor } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { pickDiagramSavePath } from "@/lib/native";
import { CONNECTION_COLOR_ORDER } from "@/lib/connectionGroups";
import { buildSvg, type ExportModel, type ExportTheme, type SvgImage, type SvgOptions } from "./exportSvg";

// WHAT:  Turns a diagram model into a PNG or SVG file the user picked a place for.
// WHY:   The image is drawn from the model (exportSvg.ts), not captured from the
//        canvas, so it needs no screenshot library. Colours come from the theme on
//        screen, resolved to plain rgb because an SVG used as an image cannot see
//        the page's CSS variables and not every engine paints `oklch()` there.
// HOW:   1. ask for the path (cancelling stops here)  2. draw the SVG  3. for PNG,
//        paint it on a canvas and encode  4. hand the bytes to Rust, which writes
//        the file. The webview never touches the file system.
// WHERE: src-tauri/src/services/diagrams.rs (the write), src/lib/native.ts (the dialog)

export type ExportFormat = "png" | "svg";

/// PNG is drawn at up to this many device pixels per diagram pixel…
const PNG_SCALE = 2;
/// …but never past the longest side a canvas can be relied on to hold.
const MAX_CANVAS_SIDE = 16384;

type Rgba = readonly [number, number, number, number];

// WHAT:  Reads one CSS colour as rgba by painting it on a 1×1 canvas.
// WHY:   `getComputedStyle` hands back `oklch(...)` for the app's tokens; the
//        canvas is the one place every engine reduces any CSS colour to bytes.
function toRgba(css: string): Rgba | null {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return null;
  // Two different sentinels: a colour the engine cannot parse leaves the fill as it was,
  // so a value that paints the same bytes under both is one it understood.
  const paint = (sentinel: string): Rgba | null => {
    context.fillStyle = sentinel;
    context.fillStyle = css;
    context.clearRect(0, 0, 1, 1);
    context.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
    if (r === undefined || g === undefined || b === undefined || a === undefined) return null;
    return [r, g, b, a / 255];
  };
  const first = paint("#010203");
  const second = paint("#fdfcfb");
  if (first === null || second === null) return null;
  const same = first.every((channel, i) => channel === second[i]);
  return same ? first : null;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function over(top: Rgba, ground: Rgba): string {
  const a = top[3];
  const mix = (t: number, g: number) => Math.round(t * a + g * (1 - a));
  return `rgb(${mix(top[0], ground[0])},${mix(top[1], ground[1])},${mix(top[2], ground[2])})`;
}

// WHAT:  The on-screen theme as colours an exported SVG can use. Translucent tokens
//        (surfaces, borders) are flattened over the opaque ground.
export function resolveExportTheme(dark: boolean): ExportTheme {
  const base: Rgba = dark ? [11, 11, 15, 1] : [255, 255, 255, 1];
  const ground = toRgba(token("--background")) ?? base;
  const groundOpaque: Rgba = [ground[0], ground[1], ground[2], 1];
  const background = over(ground, base);
  const solid = (name: string, fallback: Rgba): string => over(toRgba(token(name)) ?? fallback, groundOpaque);
  const palette = Object.fromEntries(
    CONNECTION_COLOR_ORDER.map((c) => [c, solid(`--color-conn-${c}`, [100, 116, 139, 1])]),
  );
  return {
    background,
    surface: solid("--surface", dark ? [24, 24, 30, 1] : [248, 248, 250, 1]),
    border: solid("--border", dark ? [50, 50, 58, 1] : [215, 215, 222, 1]),
    separator: solid("--separator", dark ? [38, 38, 46, 1] : [230, 230, 236, 1]),
    text: solid("--foreground", dark ? [240, 240, 242, 1] : [24, 24, 27, 1]),
    muted: solid("--muted", [140, 140, 150, 1]),
    accent: solid("--accent", [59, 130, 246, 1]),
    accentInk: solid("--accent-foreground", [255, 255, 255, 1]),
    warning: solid("--warning", [245, 158, 11, 1]),
    inkDark: solid("--diagram-ink-dark", [24, 24, 27, 1]),
    inkLight: solid("--diagram-ink-light", [252, 252, 252, 1]),
    palette: paletteOf(palette),
  };
}

// WHAT:  Narrows the resolved colour map back to a record over every palette key.
function paletteOf(resolved: Record<string, string>): Record<ConnectionColor, string> {
  const pick = (key: ConnectionColor): string => resolved[key] ?? "rgb(100,116,139)";
  return {
    blue: pick("blue"),
    teal: pick("teal"),
    green: pick("green"),
    amber: pick("amber"),
    orange: pick("orange"),
    red: pick("red"),
    pink: pick("pink"),
    violet: pick("violet"),
    gray: pick("gray"),
  };
}

function loadImage(svg: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      resolve(image);
    };
    image.onerror = () => {
      reject(new Error("The diagram could not be drawn as an image."));
    };
    image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  });
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== "string") {
        reject(new Error("The image could not be read."));
        return;
      }
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.onerror = () => {
      reject(new Error("The image could not be read."));
    };
    reader.readAsDataURL(blob);
  });
}

// WHAT:  Rasterises an SVG to PNG, base64-encoded.
async function renderPng(image: SvgImage): Promise<string> {
  const scale = Math.min(PNG_SCALE, MAX_CANVAS_SIDE / image.width, MAX_CANVAS_SIDE / image.height);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(image.width * scale));
  canvas.height = Math.max(1, Math.round(image.height * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("This system cannot draw the image.");
  context.imageSmoothingQuality = "high";
  context.drawImage(await loadImage(image.svg), 0, 0, canvas.width, canvas.height);
  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((b) => {
      if (b) resolve(b);
      else reject(new Error("The image could not be encoded."));
    }, "image/png");
  });
  return blobToBase64(blob);
}

export interface ExportRequest {
  model: ExportModel;
  format: ExportFormat;
  /// File name offered in the dialog, without extension.
  name: string;
  dark: boolean;
  options?: SvgOptions;
}

// WHAT:  Asks where to save, draws, and writes. Returns the path written, or null
//        when the user cancelled the dialog.
export async function exportDiagram(request: ExportRequest): Promise<string | null> {
  const safeName = request.name.replace(/[\\/:*?"<>|]+/g, "").trim() || "diagram";
  const path = await pickDiagramSavePath(safeName, request.format);
  if (path === null) return null;
  const image = buildSvg(request.model, resolveExportTheme(request.dark), request.options);
  const data = request.format === "svg" ? image.svg : await renderPng(image);
  return ipc("save_diagram_export", { path, format: request.format, data });
}
