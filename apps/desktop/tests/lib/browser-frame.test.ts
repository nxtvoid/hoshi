import { describe, expect, test } from 'bun:test'
import { readBrowserFrame } from '../../src/lib/browser-frame'

describe('readBrowserFrame', () => {
  test('reads the thread, the size and the JPEG the engine wrote', () => {
    const id = new TextEncoder().encode('té')
    const bytes = new Uint8Array([
      id.length,
      0,
      ...id,
      0x00,
      0x05,
      0,
      0,
      0x20,
      0x03,
      0,
      0,
      0xff,
      0xd8,
      0xff
    ])

    const frame = readBrowserFrame(bytes.buffer)

    expect(frame.threadId).toBe('té')
    expect(frame.width).toBe(1280)
    expect(frame.height).toBe(800)
    expect([...frame.jpeg]).toEqual([0xff, 0xd8, 0xff])
  })
})
