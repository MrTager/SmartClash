import { describe, expect, it } from 'vitest'

import {
  geographicPoint,
  isOccluded,
  rotatePoint,
  sampleArc,
  canvasSize,
  frameDelta,
} from './globe-geometry'

describe('SmartClash globe geometry', () => {
  it('preserves actual geographic coordinates through rotation', () => {
    const tokyo = geographicPoint(35.68, 139.69)
    expect(Math.hypot(tokyo.x, tokyo.y, tokyo.z)).toBeCloseTo(1)
    const rotated = rotatePoint(tokyo, {
      yaw: (-139.69 * Math.PI) / 180,
      pitch: 0,
    })
    expect(rotated.x).toBeCloseTo(0)
    expect(rotated.z).toBeGreaterThan(0)
    expect(isOccluded(rotated)).toBe(false)
    expect(isOccluded({ x: 0, y: 0, z: -1 })).toBe(true)
  })
  it('draws finite elevated arcs across the date line and antipodes', () => {
    for (const pair of [
      [geographicPoint(20, 179), geographicPoint(20, -179)],
      [geographicPoint(0, 0), geographicPoint(0, 180)],
    ]) {
      const arc = sampleArc(pair[0], pair[1])
      expect(arc[0]).toEqual(pair[0])
      expect(arc.at(-1)).toEqual(pair[1])
      expect(
        arc.every((point) => Object.values(point).every(Number.isFinite)),
      ).toBe(true)
      expect(Math.hypot(arc[32].x, arc[32].y, arc[32].z)).toBeGreaterThan(1)
    }
  })
  it('bounds high DPI backing stores and suspended animation jumps', () => {
    expect(canvasSize(400, 300, 8)).toEqual({
      width: 1200,
      height: 900,
      ratio: 3,
    })
    expect(frameDelta(null, 9000)).toBe(0)
    expect(frameDelta(0, 9000)).toBe(0.05)
  })
})
