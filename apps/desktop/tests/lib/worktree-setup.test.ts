import { describe, expect, test } from 'bun:test'
import {
  parseSetupFiles,
  parseSharedSetup,
  setupFilePath,
  shareSetup
} from '@/lib/worktree-setup'

describe('setupFilePath', () => {
  test('makes paths relative with forward slashes', () => {
    expect(setupFilePath('  .env ')).toBe('.env')
    expect(setupFilePath('config\\local.properties')).toBe(
      'config/local.properties'
    )
    expect(setupFilePath('./secrets//*.env/')).toBe('secrets/*.env')
  })

  test('takes blank lines as nothing', () => {
    expect(setupFilePath('   ')).toBeNull()
  })

  test('refuses paths outside the project or into .git', () => {
    for (const path of [
      '/etc/passwd',
      '\\\\server\\share',
      'C:\\Users\\nxtvoid\\.env',
      'd:secrets',
      '../other/.env',
      'config/../../.env',
      '.git/config',
      '.GIT'
    ]) {
      expect(setupFilePath(path)).toBeUndefined()
    }
  })
})

describe('parseSetupFiles', () => {
  test('keeps each file once and lists the refused lines', () => {
    expect(
      parseSetupFiles('.env\r\n\n ./.env\n../x \nsecrets/*.env\n/abs')
    ).toEqual({
      files: ['.env', 'secrets/*.env'],
      invalid: ['../x', '/abs']
    })
  })
})

describe('shared setups', () => {
  test('read back what was shared', () => {
    const setup = { files: ['.env'], command: 'bun install', wait: true }
    const text = shareSetup(setup)
    expect(JSON.parse(text)).toEqual({ hoshiWorktreeSetup: 1, ...setup })
    expect(parseSharedSetup(`  ${text}\n`)).toEqual(setup)
  })

  test('fill in what a shared setup leaves out', () => {
    expect(parseSharedSetup('{"hoshiWorktreeSetup":1,"command":"x"}')).toEqual({
      files: [],
      command: 'x',
      wait: false
    })
  })

  test('refuse text that is not a shared setup', () => {
    for (const text of [
      '',
      'bun install',
      'null',
      '{"command":"x"}',
      '{"hoshiWorktreeSetup":2,"command":"x"}',
      '{"hoshiWorktreeSetup":1,"files":".env"}',
      '{"hoshiWorktreeSetup":1,"files":[1]}',
      '{"hoshiWorktreeSetup":1,"wait":"yes"}'
    ]) {
      expect(parseSharedSetup(text)).toBeNull()
    }
  })
})
