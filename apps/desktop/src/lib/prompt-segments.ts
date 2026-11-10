import type { MessageAttachment } from '@/bindings/MessageAttachment'

export type MessageImage = Extract<MessageAttachment, { type: 'image' }>

export type SegmentKind =
  | 'text'
  | 'file'
  | 'pullRequest'
  | 'skill'
  | 'command'
  | 'image'

export type PromptSegment = {
  kind: SegmentKind
  text: string
  start: number
  end: number
  /** What an `[Image #n]` chip shows: the image's name. */
  label?: string
  /** The image an `[Image #n]` chip stands for. */
  image?: MessageImage
}

/** The pull requests, skills and images the prompt's `#123`, `$name` and `[Image #n]` stand for. */
export type KnownMentions = {
  pullRequests: ReadonlySet<number>
  skills: ReadonlySet<string>
  /** The message's images, in the order `[Image #n]` counts them. */
  images: readonly MessageImage[]
}

const MENTION =
  /(?<=^|\s)(?:(@"[^"\n]+"|@[^\s"]*[^\s".,;:!?)])|#(\d+)(?=$|[\s.,;:!?)])|\$([A-Za-z][\w:-]*)(?=$|\s))/g
const LEADING_COMMAND = /^\s*(\/[\w:-]+)(?=$|\s)/
const IMAGE = /\[Image #(\d+)\]/g

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
  for (const match of text.matchAll(IMAGE)) {
    const image = known.images[Number(match[1]) - 1]
    if (image === undefined) continue
    chips.push({
      kind: 'image',
      text: match[0],
      start: match.index,
      end: match.index + match[0].length,
      label: image.name,
      image
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

/** The `#123`, `$name` and `[Image #n]` a message's attachments stand for. */
export function knownMentions(
  attachments: readonly MessageAttachment[]
): KnownMentions {
  const pullRequests = new Set<number>()
  const skills = new Set<string>()
  const images: MessageImage[] = []
  for (const attachment of attachments) {
    if (attachment.type === 'pullRequest') pullRequests.add(attachment.number)
    if (attachment.type === 'skill') skills.add(attachment.name)
    if (attachment.type === 'image') images.push(attachment)
  }
  return { pullRequests, skills, images }
}
