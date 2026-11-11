import { expect, test } from 'bun:test'
import type { LocalServer } from '../../src/bindings/LocalServer'
import {
  ageTick,
  serverAge,
  serverDetail,
  stopsFromRow
} from '../../src/lib/servers'

const NOW = Date.UTC(2026, 10, 10, 12, 0, 0)

const server = (changes: Partial<LocalServer>): LocalServer => ({
  port: 18400,
  pid: 4242,
  processName: 'java',
  origin: 'agent',
  startedAt: NOW - 12 * 60_000 - 30_000,
  ...changes
})

test('a server is stopped from its row unless a background command may be it', () => {
  expect(stopsFromRow(server({}), 0)).toBe(true)
  expect(stopsFromRow(server({}), 1)).toBe(false)
  expect(stopsFromRow(server({ origin: 'terminal' }), 2)).toBe(true)
})

test('a young server has its age told each second', () => {
  expect(ageTick([server({ startedAt: NOW - 5_000 })], NOW)).toBe(1_000)
  expect(ageTick([server({}), server({ startedAt: null })], NOW)).toBe(30_000)
  expect(ageTick([], NOW)).toBe(30_000)
})

test('the age is told in its largest whole unit', () => {
  expect(serverAge(NOW - 40_000, NOW, 'en')).toBe('40 sec')
  expect(serverAge(NOW - 12 * 60_000 - 59_000, NOW, 'en')).toBe('12 min')
  expect(serverAge(NOW - 2 * 3_600_000 - 1, NOW, 'en')).toBe('2 hr')
  expect(serverAge(NOW - 3 * 86_400_000, NOW, 'en')).toBe('3 days')
  expect(serverAge(NOW + 5_000, NOW, 'en')).toBe('0 sec')
})

test('the detail leaves out what is unknown', () => {
  expect(serverDetail(server({}), 'Agent', NOW, 'en')).toBe(
    'Agent · java · 12 min'
  )
  expect(
    serverDetail(
      server({ processName: null, origin: 'terminal' }),
      'Terminal',
      NOW,
      'en'
    )
  ).toBe('Terminal · 12 min')
  expect(serverDetail(server({ startedAt: null }), 'Agent', NOW, 'en')).toBe(
    'Agent · java'
  )
})
