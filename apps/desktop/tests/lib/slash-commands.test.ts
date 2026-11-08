import { expect, test } from 'bun:test'
import { rankCommands } from '../../src/lib/slash-commands'

const command = (name: string, description = '', aliases: string[] = []) => ({
  name,
  description,
  aliases
})

const COMMANDS = [
  command('init', 'Initialize a new CLAUDE.md file'),
  command('code-review', 'Review the current diff', ['review']),
  command('security-review', 'Review pending changes'),
  command('compact', 'Free up context'),
  command('context', 'Show current context usage'),
  command('pdf', 'Work with PDF files', ['anthropic-skills:pdf'])
]

const names = (query: string) =>
  rankCommands(COMMANDS, query, (found) => found).map((found) => found.name)

test('an empty query keeps every command in order', () => {
  expect(names('')).toEqual(COMMANDS.map((found) => found.name))
})

test('exact names and aliases come first, then prefixes, then words inside', () => {
  expect(names('review')).toEqual(['code-review', 'security-review'])
  expect(names('co')).toEqual(['code-review', 'compact', 'context'])
})

test('descriptions and loose spellings still match', () => {
  expect(names('claude.md')).toEqual(['init'])
  expect(names('cmpct')).toEqual(['compact'])
})

test('a namespaced alias finds its skill', () => {
  expect(names('anthropic-skills:p')).toEqual(['pdf'])
})
