// Framework-free ThreeJS surface painter, the 3D sibling of LinePlot. React
// (Surface3DView) owns all state — orbit angles, sizes, interaction; this
// class turns (heights grid, orbit) into pixels.
//
// Geometry lives in a normalized unit box (x, y → [-1, 1], heights → a
// flatter z band) with the data's z mapped onto THREE's +Y so the default
// up-vector and a plain spherical orbit do the right thing. Grid cells
// touching a null sample (pole / domain gap) are skipped, not bridged —
// the same honesty contract as the 2D painter.

import * as THREE from 'three'
import { themeColor } from './LinePlot'

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

/** Robust z-range: 2%–98% quantiles of the finite heights, padded — one pole
 * spike must not flatten the rest of the surface (values beyond the range
 * clamp to the top/bottom of the box). */
export function zRange(heights: (number | null)[]): [number, number] {
  const zs = heights
    .filter((h): h is number => h !== null)
    .sort((a, b) => a - b)
  if (zs.length === 0) return [-1, 1]
  let lo = zs[Math.floor(zs.length * 0.02)]
  let hi = zs[Math.min(zs.length - 1, Math.floor(zs.length * 0.98))]
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
  private raycaster = new THREE.Raycaster()

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
    })
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

  /** Rebuild the surface mesh from a row-major heights grid (y outer, x
   * inner). Values are normalized into the unit box against [zlo, zhi];
   * out-of-range values clamp to the box top/bottom. */
  setData(
    heights: (number | null)[],
    nx: number,
    ny: number,
    zlo: number,
    zhi: number,
  ) {
    disposeGroup(this.surface)
    disposeGroup(this.frame)

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
    this.mesh = new THREE.Mesh(
      geometry,
      new THREE.MeshLambertMaterial({
        vertexColors: true,
        side: THREE.DoubleSide,
      }),
    )
    this.surface.add(this.mesh)

    this.buildFrame()
    this.render()
  }

  /** Where a ray through the given NDC pointer position hits the surface
   * (scene coords), or null. Drives the measurement cursor. */
  pick(ndcX: number, ndcY: number): THREE.Vector3 | null {
    if (!this.mesh) return null
    this.raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), this.camera)
    return this.raycaster.intersectObject(this.mesh)[0]?.point ?? null
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

function disposeGroup(group: THREE.Group) {
  for (const child of [...group.children]) {
    group.remove(child)
    const obj = child as THREE.Mesh
    obj.geometry?.dispose()
    const mat = obj.material as THREE.Material | undefined
    mat?.dispose()
  }
}
