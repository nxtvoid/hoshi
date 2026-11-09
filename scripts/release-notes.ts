// Prints the notes of a release: one bullet per change since the previous `v*` tag.
// `bun scripts/release-notes.ts <tag>`

const DOWNLOAD = 'Download the installer for your system from the assets below.'

/** A commit subject as a change, or `null` for commits that aren't one. */
export function changeBullet(subject: string) {
  if (/^chore(\([^)]*\))?!?: release v/.test(subject)) return null
  if (/^Merge /.test(subject)) return null
  const change = subject.replace(/^[a-z]+(\([^)]*\))?!?:\s*/, '').trim()
  if (!change) return null
  return `- ${change[0]?.toUpperCase()}${change.slice(1)}`
}

export function releaseNotes(subjects: string[]) {
  const bullets = subjects
    .map(changeBullet)
    .filter((bullet): bullet is string => bullet !== null)
  return bullets.length > 0 ? `${bullets.join('\n')}\n\n${DOWNLOAD}` : DOWNLOAD
}

function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { stderr: 'pipe' })
  return result.success ? result.stdout.toString().trim() : null
}

if (import.meta.main) {
  const tag = Bun.argv[2]
  if (!tag) {
    console.error('Usage: bun scripts/release-notes.ts <tag>')
    process.exit(1)
  }
  const previous = git(
    'describe',
    '--tags',
    '--abbrev=0',
    '--first-parent',
    '--match',
    'v*',
    `${tag}^`
  )
  const range = previous ? `${previous}..${tag}` : tag
  const log = git('log', '--first-parent', '--format=%s', range)
  if (log === null) {
    console.error(`Can't read the history of ${tag}`)
    process.exit(1)
  }
  console.log(releaseNotes(log ? log.split('\n') : []))
}
