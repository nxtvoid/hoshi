import type { ExtraProps } from 'react-markdown'
import { fileUrl, localPath } from '@/lib/local-paths'

type HastElement = NonNullable<ExtraProps['node']>
type HastChild = HastElement['children'][number]

const URL_ATTRIBUTES: Record<string, string> = { a: 'href', img: 'src' }

/**
 * Turns the absolute paths an agent's links and images name into `file:` URLs, which the
 * sanitizer then keeps, for `MarkdownLink` and `MarkdownImage` to open and show.
 */
export function rehypeLocalPaths() {
  return (tree: { children: HastChild[] }) => {
    for (const child of tree.children) withFileUrls(child)
  }
}

function withFileUrls(node: HastChild) {
  if (node.type !== 'element') return
  const attribute = URL_ATTRIBUTES[node.tagName]
  const value = attribute ? node.properties[attribute] : undefined
  const path = typeof value === 'string' ? localPath(value) : null
  if (attribute && path) node.properties[attribute] = fileUrl(path)
  for (const child of node.children) withFileUrls(child)
}
