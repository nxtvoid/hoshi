import { describe, expect, test } from 'bun:test'
import { type FirstRunInput, firstRunAction } from '../../src/state/first-run'

const fresh: FirstRunInput = {
  ready: true,
  completedAt: null,
  hasProjects: false,
  hasThreads: false,
  hasHistory: false,
  pathname: '/'
}

describe('firstRunAction', () => {
  test('an empty workspace opens the setup from the home route', () => {
    expect(firstRunAction(fresh)).toBe('welcome')
  })

  test('waits until the snapshot and the settings load', () => {
    expect(firstRunAction({ ...fresh, ready: false })).toBe('wait')
    expect(firstRunAction({ ...fresh, completedAt: undefined })).toBe('wait')
  })

  test('a finished or skipped setup stays closed', () => {
    expect(
      firstRunAction({ ...fresh, completedAt: '2026-11-09T08:00:00.000Z' })
    ).toBe('none')
  })

  test('an existing user is marked done without seeing it', () => {
    expect(firstRunAction({ ...fresh, hasProjects: true })).toBe('complete')
    expect(firstRunAction({ ...fresh, hasThreads: true })).toBe('complete')
    expect(firstRunAction({ ...fresh, hasHistory: true })).toBe('complete')
    expect(
      firstRunAction({ ...fresh, hasProjects: true, pathname: '/usage' })
    ).toBe('complete')
  })

  test('another route is left alone', () => {
    expect(firstRunAction({ ...fresh, pathname: '/settings/general' })).toBe(
      'wait'
    )
  })
})
