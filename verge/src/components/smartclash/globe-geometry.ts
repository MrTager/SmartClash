export function geographicPoint(
  latitude: number,
  longitude: number,
): GlobePoint {
  const lat = (latitude * Math.PI) / 180
  const lon = (longitude * Math.PI) / 180
  return {
    x: Math.cos(lat) * Math.sin(lon),
    y: Math.sin(lat),
    z: Math.cos(lat) * Math.cos(lon),
  }
}

export interface GlobePoint {
  x: number
  y: number
  z: number
}

export interface GlobeRotation {
  yaw: number
  pitch: number
}

export const TAU = Math.PI * 2

export function rotatePoint(
  point: GlobePoint,
  { yaw, pitch }: GlobeRotation,
): GlobePoint {
  const x = point.x * Math.cos(yaw) + point.z * Math.sin(yaw)
  const z = point.z * Math.cos(yaw) - point.x * Math.sin(yaw)
  return {
    x,
    y: point.y * Math.cos(pitch) - z * Math.sin(pitch),
    z: point.y * Math.sin(pitch) + z * Math.cos(pitch),
  }
}

export function projectPoint(
  point: GlobePoint,
  radius: number,
  centerX: number,
  centerY: number,
) {
  return {
    x: centerX + point.x * radius,
    y: centerY - point.y * radius,
    z: point.z,
  }
}

export function isOccluded(point: GlobePoint) {
  return point.z < 0 && point.x ** 2 + point.y ** 2 < 1 - 1e-6
}

export function clampPitch(pitch: number) {
  return Math.max(-1.1, Math.min(1.1, pitch))
}

export function wrapYaw(yaw: number) {
  return ((yaw % TAU) + TAU) % TAU
}

export function canvasSize(width: number, height: number, dpr: number) {
  const ratio = Math.max(1, Math.min(3, dpr || 1))
  return {
    width: Math.max(1, Math.round(width * ratio)),
    height: Math.max(1, Math.round(height * ratio)),
    ratio,
  }
}

export function frameDelta(previous: number | null, now: number) {
  return previous === null
    ? 0
    : Math.max(0, Math.min(0.05, (now - previous) / 1000))
}

export function canAnimate(
  animated: boolean,
  reducedMotion: boolean,
  hidden: boolean,
  hasSize: boolean,
) {
  return animated && !reducedMotion && !hidden && hasSize
}

// 点位只取决于链路序号和长度，不接收名称、IP 或任何地理信息。
export function layoutChain(count: number): GlobePoint[] {
  return Array.from({ length: count }, (_, index) => {
    const angle = count <= 1 ? 0.58 : -1.08 + (index / (count - 1)) * 2.16
    const y = count <= 1 ? 0.12 : Math.sin(index * 2.4 + 0.3) * 0.36
    const band = Math.sqrt(1 - y * y)
    return { x: Math.sin(angle) * band, y, z: Math.cos(angle) * band }
  })
}

export function sampleArc(
  from: GlobePoint,
  to: GlobePoint,
  lift = 0.22,
  segments = 64,
): GlobePoint[] {
  const dot = Math.max(
    -1,
    Math.min(1, from.x * to.x + from.y * to.y + from.z * to.z),
  )
  const angle = Math.acos(dot)
  let tangent = {
    x: to.x - dot * from.x,
    y: to.y - dot * from.y,
    z: to.z - dot * from.z,
  }
  let length = Math.hypot(tangent.x, tangent.y, tangent.z)
  if (length < 1e-6) {
    const axis =
      Math.abs(from.y) < 0.9 ? { x: 0, y: 1, z: 0 } : { x: 1, y: 0, z: 0 }
    const along = from.x * axis.x + from.y * axis.y + from.z * axis.z
    tangent = {
      x: axis.x - along * from.x,
      y: axis.y - along * from.y,
      z: axis.z - along * from.z,
    }
    length = Math.hypot(tangent.x, tangent.y, tangent.z)
  }
  const steps = Math.max(2, Math.floor(segments))
  return Array.from({ length: steps + 1 }, (_, index) => {
    if (index === 0) return { ...from }
    if (index === steps) return { ...to }
    const t = index / steps
    const radius = 1 + Math.sin(Math.PI * t) * lift
    const a = Math.cos(angle * t)
    const b = Math.sin(angle * t) / length
    return {
      x: (from.x * a + tangent.x * b) * radius,
      y: (from.y * a + tangent.y * b) * radius,
      z: (from.z * a + tangent.z * b) * radius,
    }
  })
}

export function createGraticule(): GlobePoint[][] {
  const lines: GlobePoint[][] = []
  for (let meridian = 0; meridian < 12; meridian += 1) {
    const angle = (meridian / 12) * Math.PI
    lines.push(
      Array.from({ length: 97 }, (_, step) => {
        const t = (step / 96) * TAU
        return {
          x: Math.sin(t) * Math.cos(angle),
          y: Math.cos(t),
          z: Math.sin(t) * Math.sin(angle),
        }
      }),
    )
  }
  for (let parallel = -3; parallel <= 3; parallel += 1) {
    const angle = (parallel / 8) * Math.PI
    const y = Math.sin(angle)
    const band = Math.cos(angle)
    lines.push(
      Array.from({ length: 97 }, (_, step) => {
        const t = (step / 96) * TAU
        return { x: Math.cos(t) * band, y, z: Math.sin(t) * band }
      }),
    )
  }
  return lines
}

export function createParticles(count: number): GlobePoint[] {
  const goldenAngle = Math.PI * (3 - Math.sqrt(5))
  return Array.from({ length: count }, (_, index) => {
    const y = 1 - ((index + 0.5) / count) * 2
    const band = Math.sqrt(1 - y * y)
    return {
      x: Math.cos(index * goldenAngle) * band,
      y,
      z: Math.sin(index * goldenAngle) * band,
    }
  })
}

export function createOrbit(tilt: number, radius: number): GlobePoint[] {
  return Array.from({ length: 161 }, (_, step) => {
    const angle = (step / 160) * TAU
    return {
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * Math.sin(tilt) * radius,
      z: Math.sin(angle) * Math.cos(tilt) * radius,
    }
  })
}
