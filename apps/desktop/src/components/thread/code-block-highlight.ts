import type { CodeToken } from '@/lib/highlight'
import type { Mappable } from '@tiptap/pm/transform'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'

/** Longer blocks stay plain, so typing in them never stalls. */
export const MAX_HIGHLIGHTED_CHARS = 20_000

export type CodeBlockAt = { node: ProseMirrorNode; pos: number }

export type Tokenize = (
  code: string,
  language: string
) => Promise<CodeToken[] | null>

/** Each code block's tokens, relative to its content, while the block lives. */
export type BlockTokens = WeakMap<ProseMirrorNode, Promise<CodeToken[] | null>>

export function codeBlocks(doc: ProseMirrorNode): CodeBlockAt[] {
  const blocks: CodeBlockAt[] = []
  doc.descendants((node, pos) => {
    if (node.type.name !== 'codeBlock') return true
    if (node.textContent.length <= MAX_HIGHLIGHTED_CHARS) {
      blocks.push({ node, pos })
    }
    return false
  })
  return blocks
}

function blockTokens(
  node: ProseMirrorNode,
  cache: BlockTokens,
  tokenize: Tokenize
) {
  let tokens = cache.get(node)
  if (!tokens) {
    tokens = tokenize(node.textContent, String(node.attrs.language ?? ''))
    cache.set(node, tokens)
    tokens.catch(() => {
      if (cache.get(node) === tokens) cache.delete(node)
    })
  }
  return tokens
}

/** The blocks' colored runs at their positions in the document. */
export async function highlightedRuns(
  blocks: CodeBlockAt[],
  cache: BlockTokens,
  tokenize: Tokenize
): Promise<CodeToken[]> {
  const runs: CodeToken[] = []
  for (const { node, pos } of blocks) {
    for (const token of (await blockTokens(node, cache, tokenize)) ?? []) {
      runs.push({
        from: pos + 1 + token.from,
        to: pos + 1 + token.to,
        style: token.style
      })
    }
  }
  return runs
}

/** Where the blocks sit after a change; a block the change removed is dropped. */
export function mapCodeBlocks(
  blocks: CodeBlockAt[],
  mapping: Mappable
): CodeBlockAt[] {
  return blocks.flatMap(({ node, pos }) => {
    const inside = mapping.mapResult(pos + 1)
    return inside.deleted ? [] : [{ node, pos: inside.pos - 1 }]
  })
}

export function sameCodeBlocks(a: CodeBlockAt[], b: CodeBlockAt[]) {
  return (
    a.length === b.length &&
    a.every(
      (block, index) =>
        block.node === b[index].node && block.pos === b[index].pos
    )
  )
}
