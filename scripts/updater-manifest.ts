// Writes the updater's `latest.json` for a release from its signed installers:
// `bun scripts/updater-manifest.ts <dir> <version> <owner/repo>`.
// Each installer in <dir> needs its `.sig` next to it.

import { readdir } from 'node:fs/promises'

/** Updater targets each installer serves; the plain `os-arch` key is the fallback. */
const TARGETS: [RegExp, string[]][] = [
  [/-setup\.exe$/, ['windows-x86_64-nsis', 'windows-x86_64']],
  [/\.msi$/, ['windows-x86_64-msi']],
  [/\.AppImage$/, ['linux-x86_64-appimage', 'linux-x86_64']],
  [/\.deb$/, ['linux-x86_64-deb']],
  [/\.rpm$/, ['linux-x86_64-rpm']]
]

const [dir, version, repository] = Bun.argv.slice(2)
if (!dir || !version || !repository) {
  console.error(
    'Usage: bun scripts/updater-manifest.ts <dir> <version> <owner/repo>'
  )
  process.exit(1)
}

const files = await readdir(dir)
const platforms: Record<string, { signature: string; url: string }> = {}
for (const file of files) {
  const targets = TARGETS.find(([pattern]) => pattern.test(file))?.[1]
  if (!targets) continue
  if (!files.includes(`${file}.sig`)) {
    console.error(`${file} has no signature`)
    process.exit(1)
  }
  const signature = (await Bun.file(`${dir}/${file}.sig`).text()).trim()
  const url = `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(file)}`
  for (const target of targets) {
    if (platforms[target]) {
      console.error(`Two installers for ${target}`)
      process.exit(1)
    }
    platforms[target] = { signature, url }
  }
}

for (const required of ['windows-x86_64', 'linux-x86_64']) {
  if (!platforms[required]) {
    console.error(`No installer for ${required}`)
    process.exit(1)
  }
}

console.log(
  JSON.stringify(
    {
      version,
      notes: `hoshi ${version}`,
      pub_date: new Date().toISOString(),
      platforms
    },
    null,
    2
  )
)
