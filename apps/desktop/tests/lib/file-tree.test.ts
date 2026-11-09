import { expect, test } from 'bun:test'
import type { DiffFile } from '../../src/bindings/DiffFile'
import { buildFileTree, type FileTreeNode } from '../../src/lib/file-tree'

function file(path: string, additions = 1, deletions = 0): DiffFile {
  return { path, oldPath: null, status: 'modified', additions, deletions }
}

function outline(nodes: FileTreeNode[]): unknown[] {
  return nodes.map((node) =>
    node.kind === 'file'
      ? node.name
      : {
          [node.name]: outline(node.children),
          stats: [node.additions, node.deletions]
        }
  )
}

test('folders come first, sorted by name with numbers in order', () => {
  const tree = buildFileTree([
    file('b.ts'),
    file('src/file10.ts'),
    file('a.ts'),
    file('src/file2.ts'),
    file('docs/readme.md')
  ])

  expect(outline(tree)).toEqual([
    { docs: ['readme.md'], stats: [1, 0] },
    { src: ['file2.ts', 'file10.ts'], stats: [2, 0] },
    'a.ts',
    'b.ts'
  ])
})

test('a chain of single folders shares one row and sums what it holds', () => {
  const tree = buildFileTree([
    file('src/components/chat/Row.tsx', 3, 1),
    file('src/components/chat/Card.tsx', 2, 0),
    file('src/lib/tree.ts', 5, 5)
  ])

  expect(outline(tree)).toEqual([
    {
      src: [
        { 'components/chat': ['Card.tsx', 'Row.tsx'], stats: [5, 1] },
        { lib: ['tree.ts'], stats: [5, 5] }
      ],
      stats: [10, 6]
    }
  ])
  const [src] = tree
  expect(src?.kind === 'folder' && src.children[0]?.path).toBe(
    'src/components/chat/'
  )
})

test('binary files count no lines and Windows paths split like others', () => {
  const tree = buildFileTree([
    { ...file('assets\\logo.png'), additions: null, deletions: null },
    file('assets\\icon.svg', 4, 0)
  ])

  expect(outline(tree)).toEqual([
    { assets: ['icon.svg', 'logo.png'], stats: [4, 0] }
  ])
})
