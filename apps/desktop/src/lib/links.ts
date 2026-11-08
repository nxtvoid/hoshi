/** The lowercased host of an http(s) link; `null` for anything else. */
export function webLinkHost(href: string | undefined): string | null {
  if (!href) return null
  try {
    const url = new URL(href)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
    return url.hostname.toLowerCase() || null
  } catch {
    return null
  }
}

export function faviconUrl(host: string) {
  return `https://${host}/favicon.ico`
}

/**
 * How many leading characters of a link's text stay on the line of its icon when it
 * wraps: the protocol of a written-out URL, otherwise the first character.
 */
export function leadingLength(text: string) {
  const protocol = /^https?:\/\//i.exec(text)?.[0]
  return protocol ? protocol.length : Math.min(text.length, 1)
}

const PULL_REQUEST_URL =
  /^https?:\/\/github\.com\/[^/]+\/[^/]+\/pull\/\d+(?:[/?#]|$)/i

/** Whether a link points to a pull request on a forge hoshi reads. */
export function isPullRequestUrl(href: string) {
  return PULL_REQUEST_URL.test(href)
}
