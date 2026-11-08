import { describe, expect, test } from 'bun:test'
import type { ProviderUpdate } from '@/bindings/ProviderUpdate'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import {
  appBadge,
  downloadPercent,
  providerBadge,
  strongestBadge,
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
    expect(appBadge({ status: 'failed', version: '0.2.0' })).toBe('failed')
  })
})

describe('strongestBadge', () => {
  test('a running update outranks everything, and nothing hides', () => {
    expect(strongestBadge(null, null)).toBeNull()
    expect(strongestBadge('available', null)).toBe('available')
    expect(strongestBadge('available', 'running', 'failed')).toBe('running')
    expect(strongestBadge('done', 'failed')).toBe('failed')
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
