import { describe, expect, test } from 'bun:test'
import type { ProviderUpdate } from '@/bindings/ProviderUpdate'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import {
  appBadge,
  checkDue,
  downloadPercent,
  providerBadge,
  RECHECK_ON_FOCUS_AFTER,
  strongestBadge,
  updateButton,
  updateEnded
} from '@/lib/updates'

function version(
  update: ProviderUpdate,
  updateAvailable = false
): ProviderVersion {
  return {
    installed: '2.1.311',
    latest: '2.1.319',
    updateAvailable,
    updateCommand: 'claude update',
    update
  }
}

describe('providerBadge', () => {
  test('hides when nothing is known or the CLI is current', () => {
    expect(providerBadge(null, false)).toBeNull()
    expect(providerBadge(version({ status: 'idle' }), false)).toBeNull()
  })

  test('shows an available update and a running one', () => {
    expect(providerBadge(version({ status: 'idle' }, true), true)).toBe(
      'available'
    )
    expect(
      providerBadge(version({ status: 'running', line: null }), true)
    ).toBe('running')
  })

  test('shows how an update ended until it was seen', () => {
    const failed = version(
      { status: 'failed', reason: { kind: 'timedOut' }, output: null },
      true
    )

    expect(providerBadge(version({ status: 'succeeded' }), false)).toBe('done')
    expect(providerBadge(failed, false)).toBe('failed')
    expect(providerBadge(version({ status: 'unchanged' }, true), false)).toBe(
      'failed'
    )
    expect(providerBadge(version({ status: 'succeeded' }), true)).toBeNull()
    expect(providerBadge(failed, true)).toBe('available')
  })
})

describe('appBadge', () => {
  test('follows the download', () => {
    expect(appBadge({ status: 'idle' })).toBeNull()
    expect(appBadge({ status: 'available', version: '0.2.0' })).toBe(
      'available'
    )
    expect(
      appBadge({ status: 'downloading', version: '0.2.0', percent: 40 })
    ).toBe('running')
    expect(appBadge({ status: 'ready', version: '0.2.0' })).toBe('ready')
    expect(appBadge({ status: 'failed', version: '0.2.0' })).toBe('failed')
  })

  test('a check alone points at nothing', () => {
    expect(appBadge({ status: 'checking' })).toBeNull()
    expect(appBadge({ status: 'upToDate', version: '0.1.0' })).toBeNull()
    expect(appBadge({ status: 'checkFailed', message: 'offline' })).toBeNull()
  })
})

describe('updateButton', () => {
  test('checks the feed while no update is around', () => {
    expect(updateButton({ status: 'idle' }, null, true)).toEqual({
      kind: 'check',
      checking: false
    })
    expect(updateButton({ status: 'checking' }, null, true)).toEqual({
      kind: 'check',
      checking: true
    })
    expect(
      updateButton({ status: 'upToDate', version: '0.1.0' }, null, true)
    ).toEqual({ kind: 'upToDate', version: '0.1.0' })
    expect(
      updateButton({ status: 'checkFailed', message: 'offline' }, null, true)
    ).toEqual({ kind: 'checkFailed', message: 'offline' })
  })

  test('opens the details once an update is found or running', () => {
    expect(
      updateButton({ status: 'available', version: '0.2.0' }, null, true)
    ).toEqual({ kind: 'details', badge: 'available' })
    expect(
      updateButton({ status: 'ready', version: '0.2.0' }, 'available', true)
    ).toEqual({ kind: 'details', badge: 'ready' })
    expect(updateButton({ status: 'checking' }, 'done', true)).toEqual({
      kind: 'details',
      badge: 'done'
    })
  })

  test('is off in dev builds unless the CLI has an update', () => {
    expect(updateButton({ status: 'idle' }, null, false)).toEqual({
      kind: 'off'
    })
    expect(updateButton({ status: 'idle' }, 'available', false)).toEqual({
      kind: 'details',
      badge: 'available'
    })
  })
})

describe('checkDue', () => {
  test('on focus only after a while', () => {
    expect(checkDue(null, 0)).toBe(true)
    expect(checkDue(1000, 1000 + RECHECK_ON_FOCUS_AFTER - 1)).toBe(false)
    expect(checkDue(1000, 1000 + RECHECK_ON_FOCUS_AFTER)).toBe(true)
  })
})

describe('strongestBadge', () => {
  test('a running update outranks everything, and nothing hides', () => {
    expect(strongestBadge(null, null)).toBeNull()
    expect(strongestBadge('available', null)).toBe('available')
    expect(strongestBadge('available', 'running', 'failed')).toBe('running')
    expect(strongestBadge('done', 'failed')).toBe('failed')
    expect(strongestBadge('ready', 'failed', 'available')).toBe('ready')
  })
})

describe('updateEnded', () => {
  test('only when a running update finishes', () => {
    const running: ProviderUpdate = { status: 'running', line: 'Downloading' }

    expect(updateEnded(running, { status: 'succeeded' })).toBe(true)
    expect(updateEnded(running, running)).toBe(false)
    expect(updateEnded(undefined, { status: 'succeeded' })).toBe(false)
    expect(updateEnded({ status: 'idle' }, { status: 'idle' })).toBe(false)
  })
})

describe('downloadPercent', () => {
  test('is whole and unknown without a size', () => {
    expect(downloadPercent(0, 0)).toBeNull()
    expect(downloadPercent(512, 1024)).toBe(50)
    expect(downloadPercent(1023, 1024)).toBe(99)
    expect(downloadPercent(2048, 1024)).toBe(100)
  })
})
