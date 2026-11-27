export const repository = 'nxtvoid/hoshi'
export const repositoryUrl = `https://github.com/${repository}`
export const releasesUrl = `${repositoryUrl}/releases`
export const latestReleaseUrl = `${releasesUrl}/latest`
export const discussionsUrl = `${repositoryUrl}/discussions`

const latestReleaseApi = `https://api.github.com/repos/${repository}/releases/latest`

export type Platform = 'windows' | 'linux'

export type Release = {
  version: string | null
  notesUrl: string
  downloads: Record<Platform, string>
}

type ApiRelease = {
  tag_name: string
  html_url: string
  assets: { name: string; browser_download_url: string }[]
}

const assetSuffix: Record<Platform, string> = {
  windows: '_x64-setup.exe',
  linux: '_amd64.deb'
}

export const fallbackRelease: Release = {
  version: null,
  notesUrl: latestReleaseUrl,
  downloads: { windows: latestReleaseUrl, linux: latestReleaseUrl }
}

function assetUrl(release: ApiRelease, platform: Platform): string {
  const asset = release.assets.find((a) =>
    a.name.endsWith(assetSuffix[platform])
  )
  return asset?.browser_download_url ?? release.html_url
}

export function toRelease(release: ApiRelease): Release {
  return {
    version: release.tag_name.replace(/^v/, ''),
    notesUrl: release.html_url,
    downloads: {
      windows: assetUrl(release, 'windows'),
      linux: assetUrl(release, 'linux')
    }
  }
}

export async function fetchLatestRelease(token?: string): Promise<Release> {
  try {
    const headers: Record<string, string> = {
      Accept: 'application/vnd.github+json'
    }
    if (token) headers.Authorization = `Bearer ${token}`
    const response = await fetch(latestReleaseApi, { headers })
    if (!response.ok) return fallbackRelease
    return toRelease((await response.json()) as ApiRelease)
  } catch {
    return fallbackRelease
  }
}
