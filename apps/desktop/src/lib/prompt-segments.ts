import type { MessageAttachment } from '@/bindings/MessageAttachment'

export type SegmentKind = 'text' | 'file' | 'pullRequest' | 'skill' | 'command'

export type PromptSegment = {
  kind: SegmentKind
  text: string
  start: number
  end: number
}

/** The pull requests and skills the prompt's `#123` and `$name` stand for. */
export type KnownMentions = {
  pullRequests: ReadonlySet<number>
  skills: ReadonlySet<string>
}

const MENTION =
  /(?<=^|\s)(?:(@"[^"\n]+"|@[^\s"]*[^\s".,;:!?)])|#(\d+)(?=$|[\s.,;:!?)])|\$([A-Za-z][\w:-]*)(?=$|\s))/g
const LEADING_COMMAND = /^\s*(\/[\w:-]+)(?=$|\s)/

/**
 * Splits a prompt into plain text and the mentions shown as chips: any `@path`,
 * the `/command` it starts with, and the `#123` and `$name` in `known`.
 */
export function promptSegments(
  text: string,
  known: KnownMentions
): PromptSegment[] {
  const chips: PromptSegment[] = []
  const command = LEADING_COMMAND.exec(text)
  if (command?.[1]) {
    const start = text.indexOf(command[1])
    chips.push({
      kind: 'command',
      text: command[1],
      start,
      end: start + command[1].length
    })
  }
  for (const match of text.matchAll(MENTION)) {
    const [whole, file, number, skill] = match
    const kind: SegmentKind | null = file
      ? 'file'
      : number && known.pullRequests.has(Number(number))
        ? 'pullRequest'
        : skill && known.skills.has(skill)
          ? 'skill'
          : null
    if (!kind) continue
    chips.push({
      kind,
      text: whole,
      start: match.index,
      end: match.index + whole.length
    })
  }
  chips.sort((a, b) => a.start - b.start)

  const segments: PromptSegment[] = []
  let cursor = 0
  for (const chip of chips) {
    if (chip.start < cursor) continue
    if (chip.start > cursor) {
      segments.push(plain(text, cursor, chip.start))
    }
    segments.push(chip)
    cursor = chip.end
  }
  if (cursor < text.length) segments.push(plain(text, cursor, text.length))
  return segments
}

function plain(text: string, start: number, end: number): PromptSegment {
  return { kind: 'text', text: text.slice(start, end), start, end }
}

/** The `#123` and `$name` a message's attachments stand for. */
export function knownMentions(
  attachments: readonly MessageAttachment[]
): KnownMentions {
  const pullRequests = new Set<number>()
  const skills = new Set<string>()
  for (const attachment of attachments) {
    if (attachment.type === 'pullRequest') pullRequests.add(attachment.number)
    if (attachment.type === 'skill') skills.add(attachment.name)
  }
  return { pullRequests, skills }
}
