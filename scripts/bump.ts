// Sets hoshi's version, which every crate takes from the workspace: `bun run bump 0.2.0`.

const version = Bun.argv[2]
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error('Usage: bun run bump <major.minor.patch>')
  process.exit(1)
}

const root = new URL('..', import.meta.url).pathname.replace(/^\/(\w:)/, '$1')
const cargoPath = `${root}Cargo.toml`

const cargo = await Bun.file(cargoPath).text()
const workspaceVersion = /(\[workspace\.package\][^[]*?\nversion = )"([^"]*)"/
const previous = cargo.match(workspaceVersion)?.[2]
if (!previous) {
  console.error('No version under [workspace.package] in Cargo.toml')
  process.exit(1)
}
await Bun.write(cargoPath, cargo.replace(workspaceVersion, `$1"${version}"`))

const lock = Bun.spawnSync(['cargo', 'update', '--workspace'], {
  cwd: root,
  stdout: 'inherit',
  stderr: 'inherit'
})
if (lock.exitCode !== 0) process.exit(lock.exitCode)

console.log(`hoshi ${previous} -> ${version}. Next:
  git commit -am "chore: release v${version}"
  git tag v${version}
  git push && git push origin v${version}`)
