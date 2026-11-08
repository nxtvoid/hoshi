import { expect, test } from 'bun:test'
import {
  codeLanguageLabel,
  fenceLanguage,
  searchCodeLanguages
} from '../../src/lib/code-languages'

test('a fence on its own line opens a block in its language', () => {
  expect(fenceLanguage('```tsx')).toBe('tsx')
  expect(fenceLanguage('  ```  ')).toBe('')
  expect(fenceLanguage('```c++')).toBe('c++')
  expect(fenceLanguage('see ```tsx')).toBeNull()
  expect(fenceLanguage('```tsx const')).toBeNull()
})

test('ids and aliases show their name; unknown ids show as written', () => {
  expect(codeLanguageLabel('tsx')).toBe('TSX')
  expect(codeLanguageLabel('TS')).toBe('TypeScript')
  expect(codeLanguageLabel('zig')).toBe('zig')
})

test('search matches names, ids and aliases', () => {
  const ids = (query: string) =>
    searchCodeLanguages(query).map((language) => language.id)

  expect(ids('script')).toEqual(['typescript', 'javascript'])
  expect(ids('yml')).toEqual(['yaml'])
})
