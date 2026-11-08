import { useId } from 'react'
import type { ForgeKind } from '@/bindings/ForgeKind'

/** Each forge's own mark: GitHub's in `currentColor`, Bitbucket's in its brand blues. */
export function ForgeIcon({
  forge,
  className
}: {
  forge: ForgeKind
  className?: string
}) {
  return forge === 'github' ? (
    <GitHubMark className={className} />
  ) : (
    <BitbucketMark className={className} />
  )
}

function GitHubMark({ className }: { className?: string }) {
  return (
    <svg viewBox='0 0 16 16' aria-hidden='true' className={className}>
      <path
        fill='currentColor'
        fillRule='evenodd'
        d='M8 0C3.58 0 0 3.58 0 8C0 11.54 2.29 14.53 5.47 15.59C5.87 15.66 6.02 15.42 6.02 15.21C6.02 15.02 6.01 14.39 6.01 13.72C4 14.09 3.48 13.23 3.32 12.78C3.23 12.55 2.84 11.84 2.5 11.65C2.22 11.5 1.82 11.13 2.49 11.12C3.12 11.11 3.57 11.7 3.72 11.94C4.44 13.15 5.59 12.81 6.05 12.6C6.12 12.08 6.33 11.73 6.56 11.53C4.78 11.33 2.92 10.64 2.92 7.58C2.92 6.71 3.23 5.99 3.74 5.43C3.66 5.23 3.38 4.41 3.82 3.31C3.82 3.31 4.49 3.1 6.02 4.13C6.66 3.95 7.34 3.86 8.02 3.86C8.7 3.86 9.38 3.95 10.02 4.13C11.55 3.09 12.22 3.31 12.22 3.31C12.66 4.41 12.38 5.23 12.3 5.43C12.81 5.99 13.12 6.7 13.12 7.58C13.12 10.65 11.25 11.33 9.47 11.53C9.76 11.78 10.01 12.26 10.01 13.01C10.01 14.08 10 14.94 10 15.21C10 15.42 10.15 15.67 10.55 15.59C13.71 14.53 16 11.53 16 8C16 3.58 12.42 0 8 0Z'
      />
    </svg>
  )
}

function BitbucketMark({ className }: { className?: string }) {
  // Gradient ids are document-wide, and a page can show the mark many times.
  const gradient = `bitbucket-${useId().replace(/[^\w-]/g, '')}`
  return (
    <svg
      viewBox='8.4 14.39 2481.29 2231.21'
      aria-hidden='true'
      className={className}
    >
      <defs>
        <linearGradient
          id={gradient}
          x1={945.1094}
          y1={1524.8389}
          x2={944.4923}
          y2={1524.1893}
          gradientTransform='matrix(1996.6343 0 0 -1480.3047 -1884485.625 2258195)'
          gradientUnits='userSpaceOnUse'
        >
          <stop offset={0.18} stopColor='var(--forge-bitbucket-deep)' />
          <stop offset={1} stopColor='var(--forge-bitbucket)' />
        </linearGradient>
      </defs>
      <path
        fill='var(--forge-bitbucket)'
        d='M88.92,14.4C45.02,13.83,8.97,48.96,8.41,92.86c-0.06,4.61,0.28,9.22,1.02,13.77l337.48,2048.72 c8.68,51.75,53.26,89.8,105.74,90.24h1619.03c39.38,0.5,73.19-27.9,79.49-66.78l337.49-2071.78c7.03-43.34-22.41-84.17-65.75-91.2 c-4.55-0.74-9.15-1.08-13.76-1.02L88.92,14.4z M1509.99,1495.09H993.24l-139.92-731h781.89L1509.99,1495.09z'
      />
      <path
        fill={`url(#${gradient})`}
        d='M2379.27,763.06h-745.5l-125.12,730.42H992.31l-609.67,723.67c19.32,16.71,43.96,26,69.5,26.21h1618.13 c39.35,0.51,73.14-27.88,79.44-66.72L2379.27,763.06z'
      />
    </svg>
  )
}
