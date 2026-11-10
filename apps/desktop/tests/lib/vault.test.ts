import { describe, expect, test } from 'bun:test'
import {
  accessChoices,
  effectiveAccess,
  fieldLabels,
  secretsAnswering,
  secretsInReach
} from '../../src/lib/vault'
import type { VaultSecret } from '../../src/bindings/VaultSecret'

const secret = (
  id: string,
  projectId: string | null,
  labels: string[] = ['']
): VaultSecret => ({
  id,
  name: id,
  description: '',
  projectId,
  access: 'ask',
  fields: labels.map((label, index) => ({
    id: `f${index}`,
    label,
    masked: label !== 'Email'
  })),
  createdAt: '2026-11-09T10:00:00.000Z',
  updatedAt: '2026-11-09T10:00:00.000Z',
  lastUsedAt: null
})

describe('secretsInReach', () => {
  const secrets = [
    secret('general', null),
    secret('mine', 'p'),
    secret('theirs', 'q')
  ]

  test("a thread reaches its project's secrets first, then the general ones", () => {
    expect(secretsInReach(secrets, 'p').map((each) => each.id)).toEqual([
      'mine',
      'general'
    ])
  })

  test('a chat reaches only the general ones', () => {
    expect(secretsInReach(secrets, null).map((each) => each.id)).toEqual([
      'general'
    ])
  })
})

describe('secretsAnswering', () => {
  const secrets = [
    secret('token', null),
    secret('login', 'p', ['Email', 'Password']),
    secret('other login', 'q', ['User', 'Password'])
  ]

  test('a login is answered with a login, a single value with a single one', () => {
    expect(secretsAnswering(secrets, 'p', 2).map((each) => each.id)).toEqual([
      'login'
    ])
    expect(secretsAnswering(secrets, 'p', 1).map((each) => each.id)).toEqual([
      'token'
    ])
  })
})

describe('fieldLabels', () => {
  test('a single unnamed value has none', () => {
    expect(fieldLabels(secret('token', null))).toEqual([])
    expect(fieldLabels(secret('login', null, ['Email', 'Password']))).toEqual([
      'Email',
      'Password'
    ])
  })
})

describe('effectiveAccess', () => {
  test('a secret that asks is taken freely once agents may take any', () => {
    expect(effectiveAccess('ask', true)).toBe('allow')
    expect(effectiveAccess('ask', false)).toBe('ask')
  })

  test('a hidden secret stays hidden whatever the setting', () => {
    expect(effectiveAccess('hidden', true)).toBe('hidden')
    expect(effectiveAccess('allow', false)).toBe('allow')
  })
})

describe('accessChoices', () => {
  test('asking first is not offered once agents take any secret', () => {
    expect(accessChoices(false)).toEqual(['ask', 'allow', 'hidden'])
    expect(accessChoices(true)).toEqual(['allow', 'hidden'])
  })
})
