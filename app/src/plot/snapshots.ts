// A tiny registry of live plot snapshots, keyed by the owning cell's id.
//
// Both plot painters are WebGL (no preserveDrawingBuffer), so a plot's <canvas>
// reads back blank once the frame is composited — the browser's own print path
// captures nothing. The painters work around this with a render-then-toDataURL
// `snapshot()`; this registry lets the PDF export reach those live snapshots
// without coupling to the plot components. A mounted PlotView / Surface3DView
// registers its `() => painter.snapshot()` here; the exporter calls it to embed
// the exact view the user is looking at as a PNG.

const registry = new Map<string, () => string | Promise<string>>()

/** Register a cell's plot snapshot source (sync, or async for the composite
 * exports that rasterize labels first). Returns an unregister function for
 * the component's effect cleanup. */
export function registerPlotSnapshot(
  cellId: string,
  snapshot: () => string | Promise<string>,
): () => void {
  registry.set(cellId, snapshot)
  return () => {
    if (registry.get(cellId) === snapshot) registry.delete(cellId)
  }
}

/** The cell's plot as a PNG data URL, or null if no live view is registered
 * (e.g. the plot's lazy chunk hasn't mounted, or it belongs to a notebook
 * that isn't the open one). */
export async function plotSnapshot(cellId: string): Promise<string | null> {
  const snapshot = registry.get(cellId)
  if (!snapshot) return null
  try {
    return (await snapshot()) || null
  } catch {
    return null
  }
}
