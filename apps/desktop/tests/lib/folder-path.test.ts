import { describe, expect, test } from 'bun:test'
import {
  completeFolderName,
  expandHome,
  filterFolders,
  isAbsolutePath,
  isValidFolderName,
  joinPath,
  repositoryFolderName,
  resolveFolderQuery,
  splitFolderQuery
} from '@/lib/folder-path'

describe('splitFolderQuery', () => {
  test('splits at the last separator of either kind on Windows', () => {
    expect(splitFolderQuery('C:\\Users\\me/pro', '\\')).toEqual({
      directory: 'C:\\Users\\me/',
      name: 'pro'
    })
    expect(splitFolderQuery('/home/me/', '/')).toEqual({
      directory: '/home/me/',
      name: ''
    })
  })

  test('only a backslash separates on Windows', () => {
    expect(splitFolderQuery('/home/a\\b', '/')).toEqual({
      directory: '/home/',
      name: 'a\\b'
    })
  })

  test('a query without separators is all name', () => {
    expect(splitFolderQuery('proj', '/')).toEqual({
      directory: '',
      name: 'proj'
    })
  })
})

describe('isAbsolutePath', () => {
  test('needs a drive or a share on Windows and a slash elsewhere', () => {
    expect(isAbsolutePath('C:\\code', '\\')).toBe(true)
    expect(isAbsolutePath('c:/code', '\\')).toBe(true)
    expect(isAbsolutePath('\\\\server\\share', '\\')).toBe(true)
    expect(isAbsolutePath('code\\app', '\\')).toBe(false)
    expect(isAbsolutePath('/home/me', '/')).toBe(true)
    expect(isAbsolutePath('home/me', '/')).toBe(false)
  })
})

describe('resolveFolderQuery', () => {
  test('drops a trailing separator except at a root', () => {
    expect(resolveFolderQuery('C:\\code\\app\\', '\\')).toBe('C:\\code\\app')
    expect(resolveFolderQuery('C:\\', '\\')).toBe('C:\\')
    expect(resolveFolderQuery('/', '/')).toBe('/')
    expect(resolveFolderQuery(' /home/me/app ', '/')).toBe('/home/me/app')
  })
})

describe('joinPath', () => {
  test('adds a separator only when needed', () => {
    expect(joinPath('/home/me', 'app', '/')).toBe('/home/me/app')
    expect(joinPath('/home/me/', 'app', '/')).toBe('/home/me/app')
    expect(joinPath('C:\\', 'app', '\\')).toBe('C:\\app')
  })
})

describe('expandHome', () => {
  test('replaces a leading tilde with the home folder', () => {
    expect(expandHome('~', '/home/me', '/')).toBe('/home/me/')
    expect(expandHome('~/code', '/home/me', '/')).toBe('/home/me/code')
    expect(expandHome('~\\code', 'C:\\Users\\me', '\\')).toBe(
      'C:\\Users\\me\\code'
    )
    expect(expandHome('~code', '/home/me', '/')).toBe('~code')
    expect(expandHome('~/code', null, '/')).toBe('~/code')
  })
})

describe('filterFolders', () => {
  const folders = ['.config', 'Apps', 'my-app', 'notes']

  test('puts names that start with the text before names that contain it', () => {
    expect(filterFolders(folders, 'ap')).toEqual(['Apps', 'my-app'])
  })

  test('shows hidden folders once the text starts with a dot', () => {
    expect(filterFolders(folders, '')).toEqual(['Apps', 'my-app', 'notes'])
    expect(filterFolders(folders, '.c')).toEqual(['.config'])
  })
})

describe('completeFolderName', () => {
  test('completes to what the matches share', () => {
    expect(completeFolderName(['project-a', 'Project-b'], 'pro')).toEqual({
      name: 'project-',
      complete: false
    })
    expect(completeFolderName(['notes', 'apps'], 'no')).toEqual({
      name: 'notes',
      complete: true
    })
    expect(completeFolderName(['notes'], 'x')).toBeNull()
  })
})

describe('repositoryFolderName', () => {
  test('names the folder the way git clone does', () => {
    expect(repositoryFolderName('https://github.com/acme/web-app.git')).toBe(
      'web-app'
    )
    expect(repositoryFolderName('git@github.com:acme/web-app.git')).toBe(
      'web-app'
    )
    expect(repositoryFolderName('https://host/acme/tool/')).toBe('tool')
    expect(repositoryFolderName('git@host:tool.git')).toBe('tool')
    expect(repositoryFolderName('')).toBe('')
  })
})

describe('isValidFolderName', () => {
  test('refuses names the file system would read as paths', () => {
    expect(isValidFolderName('my app')).toBe(true)
    expect(isValidFolderName('  ')).toBe(false)
    expect(isValidFolderName('a/b')).toBe(false)
    expect(isValidFolderName('a:b')).toBe(false)
    expect(isValidFolderName('..')).toBe(false)
  })
})
