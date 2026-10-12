// SOT: map-tile-geometry, web-mercator, map-basemap-urls, map-fit-bounds, map-points
import type { QueryOutcome } from "@/lib/bindings";
import { cellText, rowsOf, toNumber } from "./cells";

// WHAT:  The arithmetic behind the map tile's basemap: Web Mercator projection,
//        which raster tiles cover a viewport, panning, zooming about a point,
//        fitting a set of points, and the Carto basemap URLs.
// WHY:   A map library would bring a WebGL renderer, a style engine and a
//        worker for a tile that draws a few markers on a background. XYZ raster
//        tiles in plain <img> elements need only this much geometry, and it can
//        be tested without a browser.
// HOW:   A view is a centre (lat, lon) and a fractional zoom. The world is
//        256 * 2^zoom pixels wide; tiles are fetched at the integer zoom below
//        and drawn scaled, so a wheel zooms smoothly between levels. Longitude
//        wraps (the antimeridian pans through); latitude stops at the Mercator
//        limit.
// WHERE: ./SlippyMap.tsx, ./charts.tsx (MapChart); tiles come from basemaps.cartocdn.com
//        (allowed by img-src in src-tauri/tauri.conf.json)

const TILE = 256;
export const MAX_LAT = 85.0511287798;
export const MIN_ZOOM = 0;
export const MAX_ZOOM = 18;

export interface LatLon {
  lat: number;
  lon: number;
}

export interface MapView {
  center: LatLon;
  zoom: number;
}

export interface MapPoint extends LatLon {
  label: string;
}

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, value));
}

/// 0 at the top (north) edge of the world, 1 at the bottom.
export function mercatorY(lat: number): number {
  const rad = (clamp(lat, -MAX_LAT, MAX_LAT) * Math.PI) / 180;
  return (1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2;
}

function latOfMercatorY(y: number): number {
  const n = Math.PI * (1 - 2 * y);
  return (Math.atan(Math.sinh(n)) * 180) / Math.PI;
}

export function worldSize(zoom: number): number {
  return TILE * 2 ** zoom;
}

/// A longitude difference folded into [-180, 180].
function wrapDelta(delta: number): number {
  return ((((delta + 180) % 360) + 360) % 360) - 180;
}

export function wrapLon(lon: number): number {
  return wrapDelta(lon);
}

/// The smallest zoom at which the world still fills a viewport.
export function minZoomFor(width: number, height: number): number {
  return clamp(Math.log2(Math.max(width, height, 1) / TILE), MIN_ZOOM, MAX_ZOOM);
}

export function clampView(view: MapView, width: number, height: number): MapView {
  const zoom = clamp(view.zoom, minZoomFor(width, height), MAX_ZOOM);
  return { center: { lat: clamp(view.center.lat, -MAX_LAT, MAX_LAT), lon: wrapLon(view.center.lon) }, zoom };
}

/// Where a point falls in the viewport, in pixels from its top-left corner.
export function toScreen(point: LatLon, view: MapView, width: number, height: number): { x: number; y: number } {
  const world = worldSize(view.zoom);
  return {
    x: width / 2 + (wrapDelta(point.lon - view.center.lon) / 360) * world,
    y: height / 2 + (mercatorY(point.lat) - mercatorY(view.center.lat)) * world,
  };
}

/// The point under a viewport pixel.
export function fromScreen(x: number, y: number, view: MapView, width: number, height: number): LatLon {
  const world = worldSize(view.zoom);
  return {
    lat: latOfMercatorY(clamp(mercatorY(view.center.lat) + (y - height / 2) / world, 0, 1)),
    lon: wrapLon(view.center.lon + ((x - width / 2) / world) * 360),
  };
}

/// The view after the map is dragged by (dx, dy) pixels.
export function panBy(view: MapView, dx: number, dy: number, width: number, height: number): MapView {
  const center = fromScreen(width / 2 - dx, height / 2 - dy, view, width, height);
  return clampView({ center, zoom: view.zoom }, width, height);
}

/// The view after zooming by `delta` levels with the pixel (x, y) staying put.
export function zoomAt(view: MapView, delta: number, x: number, y: number, width: number, height: number): MapView {
  const anchor = fromScreen(x, y, view, width, height);
  const zoom = clamp(view.zoom + delta, minZoomFor(width, height), MAX_ZOOM);
  const next: MapView = { center: view.center, zoom };
  // Shift the centre so `anchor` is back under (x, y).
  const now = toScreen(anchor, next, width, height);
  return clampView({ center: fromScreen(width / 2 + (now.x - x), height / 2 + (now.y - y), next, width, height), zoom }, width, height);
}

// WHAT:  The view that shows every point with `padding` pixels to spare.
// HOW:   One point centres at city level; several take the highest zoom whose
//        projected bounding box fits, centred on the box.
export function fitView(points: readonly LatLon[], width: number, height: number, padding = 40): MapView {
  const first = points[0];
  if (first === undefined) return clampView({ center: { lat: 20, lon: 0 }, zoom: 1.5 }, width, height);
  if (points.length === 1) return clampView({ center: { lat: first.lat, lon: first.lon }, zoom: 10 }, width, height);
  // Longitudes are measured from the first point so a set straddling the
  // antimeridian fits as one span rather than a world wide one.
  const lons = points.map((p) => wrapDelta(p.lon - first.lon));
  const ys = points.map((p) => mercatorY(p.lat));
  const [minLon, maxLon] = [Math.min(...lons), Math.max(...lons)];
  const [minY, maxY] = [Math.min(...ys), Math.max(...ys)];
  const spanX = Math.max((maxLon - minLon) / 360, 1e-9);
  const spanY = Math.max(maxY - minY, 1e-9);
  const availW = Math.max(width - padding * 2, 16);
  const availH = Math.max(height - padding * 2, 16);
  const zoom = Math.log2(Math.min(availW / spanX, availH / spanY) / TILE);
  return clampView({ center: { lat: latOfMercatorY((minY + maxY) / 2), lon: first.lon + (minLon + maxLon) / 2 }, zoom: Math.min(zoom, 14) }, width, height);
}

// ---------------------------------------------------------------- tiles
export type Basemap = "light" | "dark";

const STYLES: Record<Basemap, string> = { light: "light_all", dark: "dark_all" };
const HOSTS = ["a", "b", "c", "d"] as const;

export function tileUrl(basemap: Basemap, z: number, x: number, y: number, retina: boolean): string {
  const host = HOSTS[(x + y) % HOSTS.length] ?? "a";
  return `https://${host}.basemaps.cartocdn.com/${STYLES[basemap]}/${z}/${x}/${y}${retina ? "@2x" : ""}.png`;
}

export interface PlacedTile {
  key: string;
  url: string;
  left: number;
  top: number;
  size: number;
}

/// Every tile that intersects the viewport, positioned in viewport pixels.
export function visibleTiles(view: MapView, width: number, height: number, basemap: Basemap, retina: boolean): PlacedTile[] {
  const zi = clamp(Math.floor(view.zoom), 0, MAX_ZOOM);
  const count = 2 ** zi;
  const size = TILE * 2 ** (view.zoom - zi);
  const world = worldSize(view.zoom);
  const left = ((view.center.lon + 180) / 360) * world - width / 2;
  const top = mercatorY(view.center.lat) * world - height / 2;
  const tiles: PlacedTile[] = [];
  const firstX = Math.floor(left / size);
  const lastX = Math.ceil((left + width) / size) - 1;
  const firstY = Math.max(0, Math.floor(top / size));
  const lastY = Math.min(count - 1, Math.ceil((top + height) / size) - 1);
  for (let ty = firstY; ty <= lastY; ty += 1) {
    for (let tx = firstX; tx <= lastX; tx += 1) {
      const wrapped = ((tx % count) + count) % count;
      tiles.push({ key: `${zi}/${tx}/${ty}`, url: tileUrl(basemap, zi, wrapped, ty, retina), left: tx * size - left, top: ty * size - top, size });
      if (tiles.length > 120) return tiles;
    }
  }
  return tiles;
}

// ---------------------------------------------------------------- data
/// The points a result holds: `lat` / `lon` (or `lng`) columns by name, else the
/// first two columns, and the first other column as a label.
export function parseMapPoints(outcome: QueryOutcome | null): MapPoint[] {
  const set = rowsOf(outcome);
  if (set === null) return [];
  const names = set.columns.map((c) => c.name.toLowerCase());
  const latAt = names.findIndex((c) => c.startsWith("lat"));
  const lonAt = names.findIndex((c) => c.startsWith("lon") || c.startsWith("lng"));
  const li = latAt >= 0 ? latAt : 0;
  const lo = lonAt >= 0 ? lonAt : 1;
  const labelAt = names.findIndex((_, i) => i !== li && i !== lo);
  return set.rows
    .map((r) => ({ lat: toNumber(r[li]), lon: toNumber(r[lo]), label: labelAt >= 0 ? cellText(r[labelAt]) : "" }))
    .filter((p) => Number.isFinite(p.lat) && Number.isFinite(p.lon) && Math.abs(p.lat) <= 90 && Math.abs(p.lon) <= 180);
}

/// The same set of coordinates, as one string, to tell new data from a refresh of the old.
export function pointsKey(points: readonly MapPoint[]): string {
  return points.map((p) => `${p.lat},${p.lon}`).join(";");
}
