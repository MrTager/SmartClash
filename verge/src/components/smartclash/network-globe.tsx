import { useEffect, useRef } from 'react'

import type { SmartClashMap } from '@/services/smartclash-types'

import {
  canvasSize,
  clampPitch,
  createGraticule,
  frameDelta,
  geographicPoint,
  isOccluded,
  projectPoint,
  rotatePoint,
  sampleArc,
  type GlobePoint,
} from './globe-geometry'
import land from './world-land.json'

const graticule = createGraticule()
const continents = land.rings.map((ring) =>
  ring.map(([lon, lat]) => geographicPoint(lat, lon)),
)

export function NetworkGlobe({
  map,
  animated,
  focused,
}: {
  map: SmartClashMap
  animated: boolean
  focused: string | null
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const latestRef = useRef({ map, focused })
  useEffect(() => {
    latestRef.current = { map, focused }
  }, [map, focused])
  useEffect(() => {
    const canvas = canvasRef.current
    const context = canvas?.getContext('2d')
    if (!canvas || !context) return
    const motion = window.matchMedia('(prefers-reduced-motion: reduce)')
    let width = 0
    let height = 0
    let yaw = -1.85
    let pitch = 0.25
    let zoom = 1
    let drag: { x: number; y: number } | null = null
    let frame = 0
    let previous: number | null = null
    let phase = 0
    let nextStaticFrame = 0

    const line = (points: GlobePoint[], color: string, thickness = 1) => {
      context.beginPath()
      let open = false
      for (const point of points) {
        const rotated = rotatePoint(point, { yaw, pitch })
        if (isOccluded(rotated)) {
          open = false
          continue
        }
        const screen = projectPoint(
          rotated,
          Math.min(width, height) * 0.32 * zoom,
          width / 2,
          height / 2,
        )
        if (open) context.lineTo(screen.x, screen.y)
        else context.moveTo(screen.x, screen.y)
        open = true
      }
      context.strokeStyle = color
      context.lineWidth = thickness
      context.stroke()
    }
    const dot = (point: GlobePoint, color: string, size: number) => {
      const rotated = rotatePoint(point, { yaw, pitch })
      if (isOccluded(rotated)) return
      const screen = projectPoint(
        rotated,
        Math.min(width, height) * 0.32 * zoom,
        width / 2,
        height / 2,
      )
      context.beginPath()
      context.arc(screen.x, screen.y, size, 0, Math.PI * 2)
      context.fillStyle = color
      context.shadowColor = color
      context.shadowBlur = 12
      context.fill()
      context.shadowBlur = 0
    }
    const draw = () => {
      if (!width || !height) return
      const radius = Math.min(width, height) * 0.32 * zoom
      context.clearRect(0, 0, width, height)
      context.fillStyle = '#070e1c'
      context.fillRect(0, 0, width, height)
      const glow = context.createRadialGradient(
        width / 2,
        height / 2,
        radius * 0.8,
        width / 2,
        height / 2,
        radius * 1.35,
      )
      glow.addColorStop(0, '#15597680')
      glow.addColorStop(1, '#070e1c00')
      context.fillStyle = glow
      context.fillRect(0, 0, width, height)
      const ocean = context.createRadialGradient(
        width / 2 - radius * 0.4,
        height / 2 - radius * 0.5,
        0,
        width / 2,
        height / 2,
        radius,
      )
      ocean.addColorStop(0, '#123c51')
      ocean.addColorStop(0.8, '#0a2033')
      ocean.addColorStop(1, '#071220')
      context.fillStyle = ocean
      context.beginPath()
      context.arc(width / 2, height / 2, radius, 0, Math.PI * 2)
      context.fill()
      graticule.forEach((points) => line(points, '#4bb5d41c'))
      continents.forEach((points) => line(points, '#52b8bb88', 1.1))
      const { map: data, focused: selected } = latestRef.current
      for (const route of data.routes) {
        const highlight = route.id === selected
        const points = route.locations.flatMap((location) =>
          location.latitude === null || location.longitude === null
            ? []
            : [geographicPoint(location.latitude, location.longitude)],
        )
        if (points.length !== route.locations.length) continue
        for (let i = 1; i < points.length; i += 1) {
          const arc = sampleArc(points[i - 1], points[i])
          line(arc, highlight ? '#ffc578' : '#45dbd570', highlight ? 2.2 : 1)
          if (animated && !motion.matches)
            dot(
              arc[
                Math.floor(((phase * 0.3 + i * 0.17) % 1) * (arc.length - 1))
              ],
              highlight ? '#ffe0a9' : '#90fff0',
              2.4,
            )
        }
        if (points.length)
          dot(points[points.length - 1], '#ffb864', highlight ? 4 : 2.5)
      }
      data.nodes.forEach((node) => {
        if (node.location.latitude !== null && node.location.longitude !== null)
          dot(
            geographicPoint(node.location.latitude, node.location.longitude),
            '#70f4e1',
            3,
          )
      })
    }
    const loop = (now: number) => {
      const delta = frameDelta(previous, now)
      previous = now
      if (!document.hidden && animated && !motion.matches && !drag) {
        yaw += delta * 0.035
        phase += delta
      }
      if (
        !document.hidden &&
        ((animated && !motion.matches) || now >= nextStaticFrame)
      ) {
        draw()
        nextStaticFrame = now + 1000
      }
      frame = requestAnimationFrame(loop)
    }
    const resize = new ResizeObserver(([entry]) => {
      width = entry.contentRect.width
      height = entry.contentRect.height
      const size = canvasSize(width, height, window.devicePixelRatio)
      canvas.width = size.width
      canvas.height = size.height
      context.setTransform(size.ratio, 0, 0, size.ratio, 0, 0)
      draw()
    })
    const down = (event: PointerEvent) => {
      drag = { x: event.clientX, y: event.clientY }
      canvas.setPointerCapture(event.pointerId)
    }
    const move = (event: PointerEvent) => {
      if (!drag) return
      yaw += (event.clientX - drag.x) * 0.006
      pitch = clampPitch(pitch + (event.clientY - drag.y) * 0.006)
      drag = { x: event.clientX, y: event.clientY }
      draw()
    }
    const up = () => {
      drag = null
    }
    const wheel = (event: WheelEvent) => {
      event.preventDefault()
      zoom = Math.max(0.7, Math.min(1.7, zoom - event.deltaY * 0.001))
      draw()
    }
    const reset = () => {
      yaw = -1.85
      pitch = 0.25
      zoom = 1
      draw()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key.startsWith('Arrow')) {
        event.preventDefault()
        yaw +=
          event.key === 'ArrowLeft'
            ? -0.1
            : event.key === 'ArrowRight'
              ? 0.1
              : 0
        pitch = clampPitch(
          pitch +
            (event.key === 'ArrowUp'
              ? -0.1
              : event.key === 'ArrowDown'
                ? 0.1
                : 0),
        )
        draw()
      } else if (event.key === 'Home') reset()
    }
    resize.observe(canvas)
    canvas.addEventListener('pointerdown', down)
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerup', up)
    canvas.addEventListener('pointercancel', up)
    canvas.addEventListener('wheel', wheel, { passive: false })
    canvas.addEventListener('dblclick', reset)
    canvas.addEventListener('keydown', key)
    frame = requestAnimationFrame(loop)
    return () => {
      cancelAnimationFrame(frame)
      resize.disconnect()
      canvas.removeEventListener('pointerdown', down)
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerup', up)
      canvas.removeEventListener('pointercancel', up)
      canvas.removeEventListener('wheel', wheel)
      canvas.removeEventListener('dblclick', reset)
      canvas.removeEventListener('keydown', key)
    }
  }, [animated])
  return (
    <canvas
      ref={canvasRef}
      tabIndex={0}
      aria-label="全球实时连接地球。拖动或方向键旋转，滚轮缩放，双击或 Home 键复位。"
      style={{
        width: '100%',
        height: 400,
        display: 'block',
        borderRadius: 16,
        touchAction: 'none',
        cursor: 'grab',
      }}
    />
  )
}
