import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { localPath } from '@/lib/local-paths'
import { openLocalFile } from './open-local-file'
import { Children, memo, type ReactNode, useState } from 'react'
import { GithubIcon, Globe02Icon } from '@hugeicons/core-free-icons'
import { PullRequestLinkCard } from '@/components/pull-request/PullRequestHoverCard'
import {
  faviconUrl,
  isPullRequestUrl,
  leadingLength,
  webLinkHost
} from '@/lib/links'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

const BRAND_ICONS: Record<string, typeof GithubIcon> = {
  'github.com': GithubIcon
}

const failedHosts = new Set<string>()

/**
 * A link in markdown. Web links show their site's icon and, on hover, where they go;
 * pull request links show the pull request; a file on this machine opens where it belongs.
 */
export function MarkdownLink({
  href,
  children
}: {
  href: string | undefined
  children: ReactNode
}) {
  const local = localPath(href)
  if (local) return <LocalFileLink path={local}>{children}</LocalFileLink>
  const host = webLinkHost(href)
  const link = (
    <a
      href={href}
      className='text-link decoration-dotted underline-offset-[3px] hover:underline focus-visible:underline'
      onClick={(event) => {
        if (href?.startsWith('#')) return
        event.preventDefault()
        if (href) void openUrl(href)
      }}
    >
      {host ? <WithIcon host={host}>{children}</WithIcon> : children}
    </a>
  )
  if (!host || !href) return link
  if (isPullRequestUrl(href)) {
    return <PullRequestLinkCard url={href} trigger={link} />
  }
  if (plainText(children) === href) return link
  return (
    <Tooltip>
      <TooltipTrigger render={link} />
      <TooltipContent side='top' className='max-w-sm break-all'>
        {href}
      </TooltipContent>
    </Tooltip>
  )
}

function LocalFileLink({
  path,
  children
}: {
  path: string
  children: ReactNode
}) {
  const link = (
    <button
      type='button'
      onClick={() => openLocalFile(path)}
      className='text-link cursor-pointer text-left decoration-dotted underline-offset-[3px] hover:underline focus-visible:underline'
    >
      {children}
    </button>
  )
  if (plainText(children) === path) return link
  return (
    <Tooltip>
      <TooltipTrigger render={link} />
      <TooltipContent side='top' className='max-w-sm break-all'>
        {path}
      </TooltipContent>
    </Tooltip>
  )
}

function WithIcon({ host, children }: { host: string; children: ReactNode }) {
  const [first, ...rest] = Children.toArray(children)
  if (typeof first !== 'string' || first.length === 0) {
    return (
      <>
        <SiteIcon host={host} />
        {children}
      </>
    )
  }
  const lead = leadingLength(first)
  return (
    <>
      <span className='whitespace-nowrap'>
        <SiteIcon host={host} />
        {first.slice(0, lead)}
      </span>
      {first.slice(lead)}
      {rest}
    </>
  )
}

const SiteIcon = memo(function SiteIcon({ host }: { host: string }) {
  const [failed, setFailed] = useState(() => failedHosts.has(host))
  const brand = BRAND_ICONS[host]
  return (
    <span
      aria-hidden='true'
      className='me-1 inline-flex size-3.5 align-[-0.125em]'
    >
      {brand || failed ? (
        <HugeiconsIcon
          icon={brand ?? Globe02Icon}
          strokeWidth={2}
          className='size-full'
        />
      ) : (
        <img
          src={faviconUrl(host)}
          alt=''
          loading='lazy'
          draggable={false}
          className='size-full rounded-sm'
          onError={() => {
            failedHosts.add(host)
            setFailed(true)
          }}
        />
      )}
    </span>
  )
})

function plainText(children: ReactNode) {
  return Children.toArray(children)
    .map((child) => (typeof child === 'string' ? child : ''))
    .join('')
}
