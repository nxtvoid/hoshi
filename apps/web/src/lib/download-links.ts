import { fetchLatestRelease, type Platform } from './release'

function visitorPlatform(): Platform {
  const ua = navigator.userAgent
  return /Linux/.test(ua) && !/Android/.test(ua) ? 'linux' : 'windows'
}

function showPlatform(platform: Platform) {
  for (const el of document.querySelectorAll<HTMLElement>(
    '[data-for-platform]'
  )) {
    el.hidden = el.dataset.forPlatform !== platform
  }
}

async function refreshLinks() {
  const release = await fetchLatestRelease()
  if (!release.version) return
  for (const el of document.querySelectorAll<HTMLAnchorElement>(
    '[data-download]'
  )) {
    const platform = el.dataset.download as Platform
    el.href = release.downloads[platform]
  }
  for (const el of document.querySelectorAll<HTMLElement>(
    '[data-release-version]'
  )) {
    el.textContent = `v${release.version}`
  }
  for (const el of document.querySelectorAll<HTMLElement>(
    '[data-release-meta]'
  )) {
    el.hidden = false
  }
  for (const el of document.querySelectorAll<HTMLAnchorElement>(
    '[data-release-notes]'
  )) {
    el.href = release.notesUrl
  }
}

showPlatform(visitorPlatform())
refreshLinks()
