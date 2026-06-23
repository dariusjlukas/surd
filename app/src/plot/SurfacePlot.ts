// Framework-free ThreeJS surface painter, the 3D sibling of LinePlot. React
// (Surface3DView) owns all state — orbit angles, sizes, interaction; this
// class turns (heights grid, orbit) into pixels.
//
// Geometry lives in a normalized unit box (x, y → [-1, 1], heights → a
// flatter z band) with the data's z mapped onto THREE's +Y so the default
// up-vector and a plain spherical orbit do the right thing. Grid cells
// touching a null sample (pole / domain gap) are skipped, not bridged —
// the same honesty contract as the 2D painter.
//
// View and data are decoupled, like LinePlot: the mesh is built once per
// heights grid (over the window it was *sampled* on) and setView slides /
// scales it inside the box when the *view* window moves — so pan/zoom is
// instant on the stale surface and the resampled data lands later. Stale
// geometry pushed outside the box is clipped at the frame.

import * as THREE from 'three'
import type { SurfaceRender } from '../state/settings'
import { discSprite, themeColor } from './LinePlot'

/** Opacity of a 'glass' (semi-transparent) surface — low enough to read the
 * grid and far side through it, high enough to keep the colormap legible. */
const GLASS_OPACITY = 0.55

/** Scatter marker diameter in CSS pixels (matches the 2D plot). */
const MARKER_PX = 7

/** Height of the z band relative to the unit footprint — surfaces read
 * better slightly flattened. Exported for the label overlay's scene math. */
export const Z_SCALE = 0.62
/** Camera field of view — shared by the painter and projectToPx so DOM
 * labels land exactly where the painter draws. */
export const FOV = 40
/** Look slightly below center: lifts the scene in the frame so the near
 * bottom corner (and its tick labels) clears the frame edge. */
const TARGET_Y = -0.26
/** Color ramp endpoints (HSL hue): low → blue, high → orange. Data colormaps
 * stay theme-independent, like any plotting library's. */
const HUE_LO = 0.62
const HUE_HI = 0.05

export interface Orbit {
  azimuth: number
  elevation: number
  radius: number
}

export const DEFAULT_ORBIT: Orbit = {
  azimuth: 0.65,
  elevation: 0.5,
  radius: 3.8,
}

export function clampOrbit(o: Orbit): Orbit {
  return {
    azimuth: o.azimuth,
    elevation: Math.min(1.45, Math.max(0.05, o.elevation)),
    radius: Math.min(8, Math.max(1.6, o.radius)),
  }
}

/** Robust z-range over the finite heights (plus any scatter z's, so overlaid
 * points share the box), padded — values beyond the range clamp to the
 * top/bottom of the box.
 *
 * The job is to ignore a pole spike (one cell near a singularity must not
 * flatten the rest) *without* clipping a smooth surface's legitimate extremes
 * — a plain z = x + y reaches its min/max in the corners, and a hard 2%–98%
 * cut would fold those corners flat. So we keep the true [min, max] and only
 * rein an end in when it sits more than a full core-span (the 2%–98% width)
 * beyond the bulk: a smooth surface's extreme is a fraction of a core-span
 * past p98, a real spike is many core-spans past it. */
export function zRange(
  heights: (number | null)[],
  extra: number[] = [],
): [number, number] {
  const zs = [...heights.filter((h): h is number => h !== null), ...extra].sort(
    (a, b) => a - b,
  )
  if (zs.length === 0) return [-1, 1]
  const min = zs[0]
  const max = zs[zs.length - 1]
  const p02 = zs[Math.floor(zs.length * 0.02)]
  const p98 = zs[Math.min(zs.length - 1, Math.floor(zs.length * 0.98))]
  const core = p98 - p02
  // A degenerate core (a near-constant surface) leaves no scale to fence
  // against — just bracket the data; nothing to tame.
  let lo = core === 0 ? min : Math.max(min, p02 - core)
  let hi = core === 0 ? max : Math.min(max, p98 + core)
  if (lo === hi) {
    lo -= 1
    hi += 1
  }
  const pad = (hi - lo) * 0.02
  return [lo - pad, hi + pad]
}

/** Project scene-space points to CSS pixel positions for the given orbit and
 * frame size — the DOM-label counterpart of the painter's camera (same
 * spherical-position math, same FOV, by construction). */
export function projectToPx(
  points: [number, number, number][],
  orbit: Orbit,
  width: number,
  height: number,
): { x: number; y: number; visible: boolean }[] {
  const { azimuth, elevation, radius } = clampOrbit(orbit)
  const cam = new THREE.PerspectiveCamera(FOV, width / height, 0.1, 100)
  cam.position.set(
    radius * Math.cos(elevation) * Math.sin(azimuth),
    radius * Math.sin(elevation),
    radius * Math.cos(elevation) * Math.cos(azimuth),
  )
  cam.lookAt(0, TARGET_Y, 0)
  cam.updateMatrixWorld()
  const v = new THREE.Vector3()
  return points.map(([x, y, z]) => {
    v.set(x, y, z).project(cam)
    return {
      x: ((v.x + 1) / 2) * width,
      y: ((1 - v.y) / 2) * height,
      visible: v.z > -1 && v.z < 1,
    }
  })
}

export class SurfacePlot {
  private renderer: THREE.WebGLRenderer
  private scene = new THREE.Scene()
  private camera = new THREE.PerspectiveCamera(FOV, 1, 0.1, 100)
  private surface = new THREE.Group()
  private frame = new THREE.Group()
  private mesh: THREE.Mesh | null = null
  private points: THREE.Points | null = null
  /** Draw style for the surface mesh; a rebuild (setData) re-reads it so a
   * resample keeps the chosen look. */
  private renderMode: SurfaceRender = 'solid'
  private raycaster = new THREE.Raycaster()
  /** Clip stale geometry at the frame: pan/zoom moves the surface before the
   * resample lands, and whatever leaves the box must not paint over it. */
  private clipPlanes = [
    new THREE.Plane(new THREE.Vector3(1, 0, 0), 1),
    new THREE.Plane(new THREE.Vector3(-1, 0, 0), 1),
    new THREE.Plane(new THREE.Vector3(0, 0, 1), 1),
    new THREE.Plane(new THREE.Vector3(0, 0, -1), 1),
  ]

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
    })
    this.renderer.localClippingEnabled = true
    this.scene.add(this.surface, this.frame)
    this.scene.add(new THREE.AmbientLight(0xffffff, 0.65))
    const sun = new THREE.DirectionalLight(0xffffff, 1.6)
    sun.position.set(1.5, 2.5, 2)
    this.scene.add(sun)
    this.setOrbit(DEFAULT_ORBIT)
  }

  resize(width: number, height: number) {
    this.renderer.setPixelRatio(window.devicePixelRatio || 1)
    this.renderer.setSize(width, height, false)
    this.camera.aspect = width / height
    this.camera.updateProjectionMatrix()
    this.render()
  }

  setOrbit(o: Orbit) {
    const { azimuth, elevation, radius } = clampOrbit(o)
    this.camera.position.set(
      radius * Math.cos(elevation) * Math.sin(azimuth),
      radius * Math.sin(elevation),
      radius * Math.cos(elevation) * Math.cos(azimuth),
    )
    this.camera.lookAt(0, TARGET_Y, 0)
    this.render()
  }

  /** Switch the surface draw style — solid, semi-transparent ('glass'), or
   * wireframe. Updates the live material in place (no geometry rebuild) and
   * is remembered for the next setData so a resample keeps the look. */
  setRenderMode(mode: SurfaceRender) {
    if (mode === this.renderMode) return
    this.renderMode = mode
    if (this.mesh) {
      applyRenderMode(this.mesh.material as THREE.MeshLambertMaterial, mode)
      this.render()
    }
  }

  /** Rebuild the surface mesh from a row-major heights grid (y outer, x
   * inner). Values are normalized into the unit box against [zlo, zhi];
   * out-of-range values clamp to the box top/bottom. */
  setData(
    heights: (number | null)[],
    nx: number,
    ny: number,
    zlo: number,
    zhi: number,
    scatter: [number, number, number][] = [],
  ) {
    disposeGroup(this.surface)
    disposeGroup(this.frame)
    this.mesh = null
    this.points = null

    // A surface mesh, when there is one (a points-only plot passes nx = 0).
    if (nx >= 2 && ny >= 2) {
      const positions = new Float32Array(nx * ny * 3)
      const colors = new Float32Array(nx * ny * 3)
      const c = new THREE.Color()
      for (let j = 0; j < ny; j++) {
        for (let i = 0; i < nx; i++) {
          const idx = j * nx + i
          const h = heights[idx]
          const t =
            h === null ? 0 : Math.min(1, Math.max(0, (h - zlo) / (zhi - zlo)))
          // data (x, y, z) → THREE (x, z, -y): z-up data on a y-up stage
          positions[idx * 3] = -1 + (2 * i) / (nx - 1)
          positions[idx * 3 + 1] = (2 * t - 1) * Z_SCALE
          positions[idx * 3 + 2] = -(-1 + (2 * j) / (ny - 1))
          c.setHSL(HUE_LO + (HUE_HI - HUE_LO) * t, 0.8, 0.55)
          colors[idx * 3] = c.r
          colors[idx * 3 + 1] = c.g
          colors[idx * 3 + 2] = c.b
        }
      }

      // index quads whose four corners are all real samples
      const indices: number[] = []
      for (let j = 0; j < ny - 1; j++) {
        for (let i = 0; i < nx - 1; i++) {
          const p00 = j * nx + i
          const p10 = p00 + 1
          const p01 = p00 + nx
          const p11 = p01 + 1
          if ([p00, p10, p01, p11].every((p) => heights[p] !== null)) {
            indices.push(p00, p01, p10, p10, p01, p11)
          }
        }
      }

      const geometry = new THREE.BufferGeometry()
      geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
      geometry.setAttribute('color', new THREE.BufferAttribute(colors, 3))
      geometry.setIndex(indices)
      geometry.computeVertexNormals()
      const material = new THREE.MeshLambertMaterial({
        vertexColors: true,
        side: THREE.DoubleSide,
        clippingPlanes: this.clipPlanes,
      })
      applyRenderMode(material, this.renderMode)
      this.mesh = new THREE.Mesh(geometry, material)
      this.surface.add(this.mesh)
    }

    // Scatter markers (already mapped to scene coordinates by the caller),
    // drawn as round sprites that depth-test against the surface.
    if (scatter.length) {
      const pos = new Float32Array(scatter.length * 3)
      for (let k = 0; k < scatter.length; k++) {
        pos[k * 3] = scatter[k][0]
        pos[k * 3 + 1] = scatter[k][1]
        pos[k * 3 + 2] = scatter[k][2]
      }
      const g = new THREE.BufferGeometry()
      g.setAttribute('position', new THREE.BufferAttribute(pos, 3))
      this.points = new THREE.Points(
        g,
        new THREE.PointsMaterial({
          color: themeColor('--accent', 0x7dd3fc).getHex(),
          size: MARKER_PX * (window.devicePixelRatio || 1),
          sizeAttenuation: false,
          map: discSprite(),
          alphaTest: 0.5,
          transparent: true,
          clippingPlanes: this.clipPlanes,
        }),
      )
      this.surface.add(this.points)
    }

    this.buildFrame()
    this.render()
  }

  /** Place the surface for the current view window. The mesh fills the unit
   * box for the window it was sampled on; when the view window differs
   * (pan/zoom answered from stale data), an affine slide/scale per axis maps
   * one onto the other — instant, no resample needed. Identity once the
   * fresh samples land. */
  setView(scaleX: number, scaleZ: number, posX: number, posZ: number) {
    this.surface.scale.set(scaleX, 1, scaleZ)
    this.surface.position.set(posX, 0, posZ)
    this.render()
  }

  /** Where a ray through the given NDC pointer position hits the surface
   * (scene coords), or null. Drives the measurement cursor. */
  pick(ndcX: number, ndcY: number): THREE.Vector3 | null {
    if (!this.mesh) return null
    this.raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), this.camera)
    return this.raycaster.intersectObject(this.mesh)[0]?.point ?? null
  }

  /** Index of the scatter marker nearest the pointer ray (within a small
   * screen-space tolerance), or null. The raycaster works in world space, so
   * it honors the surface group's pan/zoom transform automatically. */
  pickPoint(ndcX: number, ndcY: number): number | null {
    if (!this.points) return null
    this.raycaster.params.Points.threshold = 0.06
    this.raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), this.camera)
    return this.raycaster.intersectObject(this.points)[0]?.index ?? null
  }

  /** Where that ray crosses the floor plane (y = −Z_SCALE), or null when the
   * ray runs away from it. Domain panning grabs the floor: it always hits,
   * even where the surface has gaps. */
  pickFloor(ndcX: number, ndcY: number): THREE.Vector3 | null {
    this.raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), this.camera)
    const floor = new THREE.Plane(new THREE.Vector3(0, 1, 0), Z_SCALE)
    return this.raycaster.ray.intersectPlane(floor, new THREE.Vector3())
  }

  /** PNG of the current frame (see LinePlot.snapshot for why this works
   * without preserveDrawingBuffer). */
  snapshot(): string {
    this.render()
    return this.renderer.domElement.toDataURL('image/png')
  }

  dispose() {
    disposeGroup(this.surface)
    disposeGroup(this.frame)
    this.mesh = null
    this.points = null
    this.renderer.dispose()
  }

  /** Bounding-box edges + a floor grid, in theme colors. */
  private buildFrame() {
    const axis = new THREE.LineBasicMaterial({
      color: themeColor('--plot-axis', 0x475569),
    })
    const grid = new THREE.LineBasicMaterial({
      color: themeColor('--plot-grid', 0x27303f),
    })

    const box = new THREE.BoxGeometry(2, 2 * Z_SCALE, 2)
    this.frame.add(new THREE.LineSegments(new THREE.EdgesGeometry(box), axis))
    box.dispose()

    const floor: number[] = []
    const y = -Z_SCALE
    const divisions = 4
    for (let k = 0; k <= divisions; k++) {
      const t = -1 + (2 * k) / divisions
      floor.push(t, y, -1, t, y, 1, -1, y, t, 1, y, t)
    }
    const g = new THREE.BufferGeometry()
    g.setAttribute('position', new THREE.Float32BufferAttribute(floor, 3))
    this.frame.add(new THREE.LineSegments(g, grid))
  }

  private render() {
    this.renderer.render(this.scene, this.camera)
  }
}

/** Stamp a draw style onto the surface material. Wireframe shows the triangle
 * mesh; glass blends the shaded surface at reduced opacity (depthWrite stays
 * on, so the surface still sorts cleanly against the markers and frame). */
function applyRenderMode(mat: THREE.MeshLambertMaterial, mode: SurfaceRender) {
  mat.wireframe = mode === 'wire'
  mat.transparent = mode === 'glass'
  mat.opacity = mode === 'glass' ? GLASS_OPACITY : 1
  mat.needsUpdate = true
}

function disposeGroup(group: THREE.Group) {
  for (const child of [...group.children]) {
    group.remove(child)
    const obj = child as THREE.Mesh
    obj.geometry?.dispose()
    const mat = obj.material as THREE.Material | undefined
    mat?.dispose()
  }
}
