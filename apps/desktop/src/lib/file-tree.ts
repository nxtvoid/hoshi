import type { DiffFile } from '@/bindings/DiffFile'

export type FileTreeNode =
  | {
      kind: 'folder'
      /** Unique within the tree; a chain of single folders shows as `a/b/c`. */
      path: string
      name: string
      additions: number
      deletions: number
      children: FileTreeNode[]
    }
  | { kind: 'file'; path: string; name: string; file: DiffFile }

type Draft = { folders: Map<string, Draft>; files: DiffFile[] }

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: 'base'
})

/**
 * Changed files as folders and files, folders first and each sorted by name. A folder
 * whose only child is a folder shares its row: `src/components/chat`.
 */
export function buildFileTree(files: DiffFile[]): FileTreeNode[] {
  const root: Draft = { folders: new Map(), files: [] }
  for (const file of files) {
    const parts = file.path.replaceAll('\\', '/').split('/')
    let draft = root
    for (const part of parts.slice(0, -1)) {
      let next = draft.folders.get(part)
      if (!next) {
        next = { folders: new Map(), files: [] }
        draft.folders.set(part, next)
      }
      draft = next
    }
    draft.files.push(file)
  }
  return nodesOf(root, '')
}

function nodesOf(draft: Draft, prefix: string): FileTreeNode[] {
  const folders = [...draft.folders]
    .sort(([a], [b]) => collator.compare(a, b))
    .map(([name, child]) => folderOf(name, child, prefix))
  const files = [...draft.files]
    .map((file) => {
      const path = file.path.replaceAll('\\', '/')
      const name = path.slice(path.lastIndexOf('/') + 1)
      return { kind: 'file' as const, path: file.path, name, file }
    })
    .sort((a, b) => collator.compare(a.name, b.name))
  return [...folders, ...files]
}

function folderOf(name: string, draft: Draft, prefix: string): FileTreeNode {
  let label = name
  let current = draft
  while (current.files.length === 0 && current.folders.size === 1) {
    const [[childName, child]] = [...current.folders] as [[string, Draft]]
    label = `${label}/${childName}`
    current = child
  }
  const path = `${prefix}${label}/`
  const children = nodesOf(current, path)
  const totals = changesOf(children)
  return { kind: 'folder', path, name: label, ...totals, children }
}

function changesOf(nodes: FileTreeNode[]) {
  let additions = 0
  let deletions = 0
  for (const node of nodes) {
    if (node.kind === 'folder') {
      additions += node.additions
      deletions += node.deletions
    } else {
      additions += node.file.additions ?? 0
      deletions += node.file.deletions ?? 0
    }
  }
  return { additions, deletions }
}

/** Whether the tree has a folder to open or close. */
export function hasFolders(nodes: FileTreeNode[]) {
  return nodes.some((node) => node.kind === 'folder')
}
