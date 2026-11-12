import { describe, expect, test } from 'bun:test'
import {
  clampPosition,
  EDGE_GAP,
  type Obstacles,
  resizeFrame,
  resolveCanvas,
  resolveFrame
} from '../../src/lib/mini-browser-layout'

const container = { width: 1000, height: 700 }
const desktop = { width: 1280, height: 800 }
const composer = { left: 200, right: 800, height: 120 }
const detailsCard = { left: 700, right: 988, bottom: 200 }

describe('resolveFrame', () => {
  test('opens in the top right corner at the page shape', () => {
    const frame = resolveFrame({
      width: null,
      position: null,
      source: desktop,
      container
    })
    expect(frame).toEqual({
      x: container.width - EDGE_GAP - 320,
      y: EDGE_GAP,
      width: 320,
      height: 200
    })
  })

  test('a phone page opens at the minimum width, at its shape', () => {
    const frame = resolveFrame({
      width: null,
      position: null,
      source: { width: 390, height: 844 },
      container
    })
    expect(frame.width).toBe(240)
    expect(frame.height).toBe(Math.round((240 * 844) / 390))
  })

  test('opens under the details card, flush with its right edge', () => {
    const frame = resolveFrame({
      width: null,
      position: null,
      source: desktop,
      container,
      obstacles: { composer: null, detailsCard }
    })
    expect(frame.x + frame.width).toBe(detailsCard.right)
    expect(frame.y).toBe(detailsCard.bottom + EDGE_GAP)
  })

  test('keeps the width the user chose and stays inside a narrow chat', () => {
    const frame = resolveFrame({
      width: 600,
      position: { x: 380, y: 40 },
      source: desktop,
      container: { width: 500, height: 700 }
    })
    expect(frame.width).toBeLessThanOrEqual(500 - EDGE_GAP * 2)
    expect(frame.x).toBeGreaterThanOrEqual(EDGE_GAP)
  })
})

describe('clampPosition', () => {
  const player = { width: 320, height: 200 }
  const obstacles: Obstacles = { composer, detailsCard: null }

  test('lifts the player off the composer', () => {
    const position = clampPosition(
      { x: 300, y: 600 },
      container,
      player,
      obstacles
    )
    expect(position.y + player.height).toBeLessThanOrEqual(
      container.height - composer.height - EDGE_GAP
    )
  })

  test('a column beside the composer keeps its place', () => {
    const position = clampPosition(
      { x: 820, y: 480 },
      { width: 1200, height: 700 },
      player,
      obstacles
    )
    expect(position).toEqual({ x: 820, y: 480 })
  })

  test('stays inside the chat', () => {
    expect(clampPosition({ x: -50, y: -50 }, container, player)).toEqual({
      x: EDGE_GAP,
      y: EDGE_GAP
    })
  })
})

describe('resizeFrame', () => {
  const start = { x: 400, y: 100, width: 320, height: 200 }

  test('dragging the east edge grows the frame at the page shape', () => {
    const frame = resizeFrame({
      start,
      direction: 'east',
      delta: { x: 160, y: 0 },
      source: desktop,
      container
    })
    expect(frame).toEqual({ x: 400, y: 100, width: 480, height: 300 })
  })

  test('the west edge keeps the right side in place', () => {
    const frame = resizeFrame({
      start,
      direction: 'west',
      delta: { x: -80, y: 0 },
      source: desktop,
      container
    })
    expect(frame.x + frame.width).toBe(start.x + start.width)
    expect(frame.width).toBe(400)
  })

  test('never shrinks below the minimum', () => {
    const frame = resizeFrame({
      start,
      direction: 'southeast',
      delta: { x: -500, y: -500 },
      source: desktop,
      container
    })
    expect(frame.width).toBe(240)
    expect(frame.height).toBe(150)
  })
})

describe('resolveCanvas', () => {
  const opened = {
    width: null,
    position: null,
    source: desktop,
    interaction: 'drag' as const
  }

  test('opens in the bottom right corner and moves the chat aside', () => {
    const wide = { width: 1400, height: 800 }
    const { frame, chat, overlapsChat, moved } = resolveCanvas({
      container: wide,
      player: opened,
      composerHeight: 120
    })
    expect(frame.x + frame.width).toBe(wide.width - EDGE_GAP)
    expect(frame.y + frame.height).toBe(wide.height - EDGE_GAP)
    expect(overlapsChat).toBe(false)
    expect(moved).toBe(true)
    expect(chat.left + chat.width).toBeLessThanOrEqual(frame.x - EDGE_GAP)
    expect(chat.width).toBeGreaterThanOrEqual(640)
  })

  test('leaves the chat alone when the player already clears it', () => {
    const { moved } = resolveCanvas({
      container: { width: 1900, height: 800 },
      player: opened,
      composerHeight: 120
    })
    expect(moved).toBe(false)
  })

  test('in a narrow chat it floats over the chat, above the composer', () => {
    const { frame, overlapsChat, moved } = resolveCanvas({
      container,
      player: opened,
      composerHeight: 120
    })
    expect(overlapsChat).toBe(true)
    expect(moved).toBe(false)
    expect(frame.y + frame.height).toBeLessThanOrEqual(
      container.height - 120 - EDGE_GAP
    )
  })

  test('a drag stops where the chat would turn unreadable', () => {
    const { frame } = resolveCanvas({
      container: { width: 1400, height: 800 },
      player: { ...opened, width: 320, position: { x: 100, y: 300 } },
      composerHeight: 120
    })
    expect(frame.x).toBe(48 + 640 + EDGE_GAP)
  })
})
