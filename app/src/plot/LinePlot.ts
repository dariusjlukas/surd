// Framework-free ThreeJS line-plot painter. React (PlotView) owns all state —
// window, samples, interaction; this class only turns (view, data) into
// pixels. Geometry lives in data coordinates and an orthographic camera is
// mapped to the view window, so pan/zoom is a camera update, not a rebuild.

import * as THREE from 'three'
import type { SamplePoint } from '../engine/types'

export interface ViewWindow {
  a: number // x min
  b: number // x max
  lo: number // y min
  hi: number // y max
}

const COLOR_GRID = 0x27303f
const COLOR_AXIS = 0x475569
const COLOR_CURVE = 0x7dd3fc

export class LinePlot {
  private renderer: THREE.WebGLRenderer
  private scene = new THREE.Scene()
  private camera = new THREE.OrthographicCamera(0, 1, 1, 0, 0.1, 10)
  private grid = new THREE.Group()
  private curve = new THREE.Group()
  private view: ViewWindow = { a: 0, b: 1, lo: 0, hi: 1 }

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true })
    this.camera.position.z = 1
    this.scene.add(this.grid, this.curve)
  }

  resize(width: number, height: number) {
    this.renderer.setPixelRatio(window.devicePixelRatio || 1)
    this.renderer.setSize(width, height, false)
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

  /** Replace the curve. Samples split into continuous runs at nulls so poles
   * and domain gaps break the line instead of bridging it. */
  setData(points: SamplePoint[]) {
    disposeChildren(this.curve)
    const material = new THREE.LineBasicMaterial({ color: COLOR_CURVE })
    let run: number[] = []
    const flush = () => {
      if (run.length >= 6) {
        const g = new THREE.BufferGeometry()
        g.setAttribute('position', new THREE.Float32BufferAttribute(run, 3))
        this.curve.add(new THREE.Line(g, material))
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
    this.render()
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
      segments(gridVerts, new THREE.LineBasicMaterial({ color: COLOR_GRID })),
    )

    // zero axes, when in view, drawn on top of the grid
    const axisVerts: number[] = []
    if (a < 0 && b > 0) axisVerts.push(0, lo, 0, 0, hi, 0)
    if (lo < 0 && hi > 0) axisVerts.push(a, 0, 0, b, 0, 0)
    if (axisVerts.length) {
      this.grid.add(
        segments(axisVerts, new THREE.LineBasicMaterial({ color: COLOR_AXIS })),
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
