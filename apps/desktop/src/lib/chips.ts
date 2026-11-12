/** How each kind of mention looks, in the composer and in sent messages. */
export const CHIP_CLASSES = {
  file: 'bg-link/12 text-link',
  command: 'bg-link/12 text-link',
  pullRequest: 'bg-pr-open/12 text-pr-open',
  skill: 'bg-provider-claude/12 text-provider-claude',
  image: 'bg-link/12 text-link',
  attachedFile: 'bg-link/12 text-link'
} as const
