/** Link definitions, footnotes, raw HTML and directives can tie blocks together. */
const DOCUMENT_WIDE = /^ {0,3}(?:\[[^\]\n]+\]:|<[A-Za-z!?/]|:::|\$\$)|\[\^/m
const FENCE = /^ {0,3}(`{3,}|~{3,})/
/** After a blank line these still belong to the block before: list items, indented text. */
const CONTINUATION = /^(?:[ \t]|(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$))/

/**
 * Markdown cut at the blank lines that close every open block, so each piece renders as it
 * would within the whole text; `null` when the text uses syntax that spans blocks.
 */
export function splitMarkdownBlocks(text: string): string[] | null {
  if (text.includes('\r') || DOCUMENT_WIDE.test(text)) return null
  const blocks: string[] = []
  let blockStart = 0
  let lineStart = 0
  let fence: string | null = null
  let fenceIndent = 0
  let afterBlank = false
  for (const line of text.split('\n')) {
    const opener = FENCE.exec(line)?.[1]
    if (fence !== null) {
      if (/\S/.test(line.slice(0, fenceIndent))) break
      if (
        opener &&
        opener[0] === fence[0] &&
        opener.length >= fence.length &&
        line.trim() === opener
      ) {
        fence = null
      }
    } else if (line.trim() === '') {
      afterBlank = true
    } else {
      if (afterBlank && lineStart > blockStart && !CONTINUATION.test(line)) {
        blocks.push(text.slice(blockStart, lineStart))
        blockStart = lineStart
      }
      afterBlank = false
      fence = opener ?? null
      if (opener) fenceIndent = line.indexOf(opener)
    }
    lineStart += line.length + 1
  }
  blocks.push(text.slice(blockStart))
  return blocks
}
