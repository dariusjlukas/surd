// Framework-free ThreeJS line-plot painter. React (PlotView) owns all state —
// window, samples, interaction; this class only turns (view, data) into
// pixels. Geometry lives in data coordinates and an orthographic camera is
// mapped to the view window, so pan/zoom is a camera update, not a rebuild.

import * as THREE from 'three'
import { Line2 } from 'three/addons/lines/Line2.js'
import { LineGeometry } from 'three/addons/lines/LineGeometry.js'
import { LineMaterial } from 'three/addons/lines/LineMaterial.js'
import type { SamplePoint } from '../engine/types'

/** Curve stroke width in CSS pixels. Native WebGL lines are stuck at 1
 * device pixel (linewidth is ignored almost everywhere), which reads as a
 * thin, aliased hairline — so curves use the fat-line addon instead, which
 * extrudes screen-space quads and antialiases like any other triangle. */
const CURVE_WIDTH_PX = 2

/** Scatter marker diameter in CSS pixels. */
const MARKER_PX = 7

/** A soft-edged disc texture for round markers, built once and shared by every
 * scatter material (PointsMaterial draws textured square sprites; the alpha
 * disc is what makes them read as circles). */
let discTexture: THREE.Texture | null = null
function discSprite(): THREE.Texture {
  if (discTexture) return discTexture
  const s = 64
  const canvas = document.createElement('canvas')
  canvas.width = canvas.height = s
  const ctx = canvas.getContext('2d')!
  ctx.beginPath()
  ctx.arc(s / 2, s / 2, s / 2 - 1, 0, Math.PI * 2)
  ctx.fillStyle = '#fff'
  ctx.fill()
  const tex = new THREE.CanvasTexture(canvas)
  discTexture = tex
  return tex
}

export interface ViewWindow {
  a: number // x min
  b: number // x max
  lo: number // y min
  hi: number // y max
}

/** Theme token → THREE color, sampled at draw time so grid/axis/curve track
 * the active theme (index.css guarantees these vars hold plain hex). */
export function themeColor(token: string, fallback: number): THREE.Color {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim()
  return v ? new THREE.Color(v) : new THREE.Color(fallback)
}

/** Series i's color token: the accent first, then the fixed palette,
 * cycling. The legend chips use the same tokens via CSS var(). */
export function seriesColorToken(i: number): string {
  const slot = i % 6
  return slot === 0 ? '--accent' : `--plot-s${slot + 1}`
}

export class LinePlot {
  private renderer: THREE.WebGLRenderer
  private scene = new THREE.Scene()
  private camera = new THREE.OrthographicCamera(0, 1, 1, 0, 0.1, 10)
  private grid = new THREE.Group()
  private curve = new THREE.Group()
  private view: ViewWindow = { a: 0, b: 1, lo: 0, hi: 1 }
  /** Canvas CSS size; LineMaterial needs it to convert linewidth to clip
   * space, so resize() keeps the curve materials in sync. */
  private size = new THREE.Vector2(1, 1)

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
    })
    this.camera.position.z = 1
    // Everything sits at z = 0, and three.js breaks the tie by material.id —
    // rebuildGrid makes fresh materials each view change, so without an
    // explicit renderOrder the grid would draw *over* the curves, punching a
    // gap wherever a gridline crosses (or runs tangent to) a curve.
    this.curve.renderOrder = 1
    this.scene.add(this.grid, this.curve)
  }

  resize(width: number, height: number) {
    this.renderer.setPixelRatio(window.devicePixelRatio || 1)
    this.renderer.setSize(width, height, false)
    this.size.set(width, height)
    // Only fat-line materials track the canvas resolution; scatter markers are
    // sized in pixels and need no update.
    for (const child of this.curve.children) {
      if (child instanceof Line2)
        (child.material as LineMaterial).resolution.copy(this.size)
    }
    this.render()
  }

  /** Map the camera to a data window and rebuild grid lines at the given tick
   * positions (computed by the caller from the same window). */
  setView(view: ViewWindow, xTicks: number[], yTicks: number[]) {
    this.view = view
    this.camera.left = view.a
    this.camera.right = view.b
    this.camera.top = view.hi
    this.camera.bottom = view.lo
    this.camera.updateProjectionMatrix()
    this.rebuildGrid(xTicks, yTicks)
    this.render()
  }

  /** Replace the curves — one entry per series, colored by the shared
   * palette (see seriesColorToken). A series flagged in `scatter` is drawn as
   * discrete markers; the rest split into continuous runs at nulls so poles
   * and domain gaps break the line instead of bridging it. */
  setData(series: SamplePoint[][], scatter: boolean[] = []) {
    disposeChildren(this.curve)
    series.forEach((points, i) => {
      const color = themeColor(seriesColorToken(i), 0x7dd3fc).getHex()
      if (scatter[i]) {
        const positions: number[] = []
        for (const [x, y] of points) if (y !== null) positions.push(x, y, 0)
        if (positions.length) {
          const g = new THREE.BufferGeometry()
          g.setAttribute(
            'position',
            new THREE.Float32BufferAttribute(positions, 3),
          )
          const m = new THREE.PointsMaterial({
            color,
            size: MARKER_PX * (window.devicePixelRatio || 1),
            sizeAttenuation: false,
            map: discSprite(),
            alphaTest: 0.5,
            transparent: true,
          })
          this.curve.add(new THREE.Points(g, m))
        }
        return
      }
      const material = new LineMaterial({
        color,
        linewidth: CURVE_WIDTH_PX,
      })
      material.resolution.copy(this.size)
      let run: number[] = []
      const flush = () => {
        if (run.length >= 6) {
          const g = new LineGeometry()
          g.setPositions(run)
          this.curve.add(new Line2(g, material))
        }
        run = []
      }
      for (const [x, y] of points) {
        if (y === null) {
          flush()
        } else {
          run.push(x, y, 0)
        }
      }
      flush()
    })
    this.render()
  }

  /** PNG of the current frame. Rendering immediately before reading is what
   * makes this work without preserveDrawingBuffer (the WebGL backbuffer is
   * cleared after compositing, but not within the same task). */
  snapshot(): string {
    this.render()
    return this.renderer.domElement.toDataURL('image/png')
  }

  dispose() {
    disposeChildren(this.grid)
    disposeChildren(this.curve)
    this.renderer.dispose()
  }

  private rebuildGrid(xTicks: number[], yTicks: number[]) {
    disposeChildren(this.grid)
    const { a, b, lo, hi } = this.view

    const gridVerts: number[] = []
    for (const t of xTicks) gridVerts.push(t, lo, 0, t, hi, 0)
    for (const t of yTicks) gridVerts.push(a, t, 0, b, t, 0)
    this.grid.add(
      segments(
        gridVerts,
        new THREE.LineBasicMaterial({
          color: themeColor('--plot-grid', 0x27303f),
        }),
      ),
    )

    // zero axes, when in view, drawn on top of the grid
    const axisVerts: number[] = []
    if (a < 0 && b > 0) axisVerts.push(0, lo, 0, 0, hi, 0)
    if (lo < 0 && hi > 0) axisVerts.push(a, 0, 0, b, 0, 0)
    if (axisVerts.length) {
      this.grid.add(
        segments(
          axisVerts,
          new THREE.LineBasicMaterial({
            color: themeColor('--plot-axis', 0x475569),
          }),
        ),
      )
    }
  }

  private render() {
    this.renderer.render(this.scene, this.camera)
  }
}

function segments(verts: number[], material: THREE.LineBasicMaterial) {
  const g = new THREE.BufferGeometry()
  g.setAttribute('position', new THREE.Float32BufferAttribute(verts, 3))
  return new THREE.LineSegments(g, material)
}

function disposeChildren(group: THREE.Group) {
  for (const child of [...group.children]) {
    group.remove(child)
    const line = child as THREE.Line
    line.geometry?.dispose()
    const mat = line.material as THREE.Material | undefined
    mat?.dispose()
  }
}
