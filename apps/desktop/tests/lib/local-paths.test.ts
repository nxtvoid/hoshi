import { describe, expect, test } from 'bun:test'
import { fileUrl, localPath, opensInEditor } from '../../src/lib/local-paths'

describe('localPath', () => {
  test('reads absolute paths on any system', () => {
    expect(localPath('C:\\work\\shot.png')).toBe('C:\\work\\shot.png')
    expect(localPath('C:/work/a b.png')).toBe('C:/work/a b.png')
    expect(localPath('/home/user/shot.png')).toBe('/home/user/shot.png')
  })

  test('reads file URLs back into paths', () => {
    expect(localPath('file:///C:/work/a%20b.png')).toBe('C:/work/a b.png')
    expect(localPath('file:///home/user/a.png')).toBe('/home/user/a.png')
  })

  test('drops the line an agent adds', () => {
    expect(localPath('/repo/src/main.rs:42')).toBe('/repo/src/main.rs')
    expect(localPath('C:/repo/a.ts:10:3')).toBe('C:/repo/a.ts')
  })

  test('leaves web and relative links alone', () => {
    expect(localPath('https://example.com/a.png')).toBeNull()
    expect(localPath('//cdn.example.com/a.png')).toBeNull()
    expect(localPath('docs/a.md')).toBeNull()
    expect(localPath('#heading')).toBeNull()
    expect(localPath(undefined)).toBeNull()
  })
})

describe('fileUrl', () => {
  test('round-trips through localPath', () => {
    for (const path of ['C:/work/a b.png', '/home/user/ü.png']) {
      expect(localPath(fileUrl(path))).toBe(path)
    }
    expect(fileUrl('C:\\work\\a.png')).toBe('file:///C:/work/a.png')
  })
})

test('media opens in its app, code in the editor', () => {
  expect(opensInEditor('/repo/src/main.rs')).toBe(true)
  expect(opensInEditor('/repo/shot.PNG')).toBe(false)
  expect(opensInEditor('/repo/report.pdf')).toBe(false)
})
