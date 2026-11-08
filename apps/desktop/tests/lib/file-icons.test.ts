import { expect, test } from 'bun:test'
import { fileIcon, languageFileName } from '../../src/lib/file-icons'

test('a file gets its kind of icon and color, anything else the default', () => {
  expect(fileIcon('src/App.tsx')).toEqual({
    name: 'file-tree-builtin-react',
    color: 'text-file-cyan'
  })
  expect(fileIcon('crates/core/src/lib.rs').color).toBe('text-file-orange')
  expect(fileIcon('notes.unknown')).toEqual({
    name: 'file-tree-builtin-default',
    color: 'text-file-gray'
  })
})

test('a code language borrows the icon of a file in it', () => {
  expect(languageFileName('tsx')).toBe('file.tsx')
  expect(languageFileName('TypeScript')).toBe('file.ts')
  expect(languageFileName('dockerfile')).toBe('Dockerfile')
  expect(languageFileName('')).toBe('file.txt')
  expect(fileIcon(languageFileName('bash')).name).toBe('file-tree-builtin-bash')
})
