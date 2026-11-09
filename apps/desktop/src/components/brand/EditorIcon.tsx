import { useId } from 'react'
import { cn } from '@hoshi/utils'
import { HugeiconsIcon } from '@hugeicons/react'
import type { Editor } from '@/bindings/Editor'
import { SourceCodeIcon } from '@hugeicons/core-free-icons'

type IconProps = { className?: string }

/** Each editor's own mark, in its brand colors, as the editors show it. */
export function EditorIcon({
  editor,
  className
}: {
  editor: Editor
  className?: string
}) {
  switch (editor) {
    case 'vscode':
      return <VisualStudioCodeIcon className={className} />
    case 'cursor':
      return <CursorIcon className={className} />
    case 'zed':
      return <ZedIcon className={className} />
    case 'fileManager':
      return isMac() ? (
        <FinderIcon className={className} />
      ) : (
        <FileExplorerIcon className={className} />
      )
    case 'idea':
    case 'webstorm':
    case 'pycharm':
    case 'goland':
    case 'rider':
    case 'clion':
    case 'phpstorm':
    case 'rubymine':
    case 'rustrover':
      return (
        <JetBrainsIcon {...JETBRAINS_MARKS[editor]} className={className} />
      )
    case 'windsurf':
      return (
        <HugeiconsIcon
          icon={SourceCodeIcon}
          strokeWidth={2}
          className={cn('text-muted-foreground', className)}
        />
      )
  }
}

export function isMac() {
  return navigator.userAgent.includes('Mac')
}

export function isWindows() {
  return navigator.userAgent.includes('Windows')
}

function VisualStudioCodeIcon({ className }: IconProps) {
  const id = useId()
  const mask = `${id}-mask`
  return (
    <svg
      viewBox='0 0 100 100'
      fill='none'
      aria-hidden='true'
      className={cn('shrink-0', className)}
    >
      <mask
        id={mask}
        width='100'
        height='100'
        x='0'
        y='0'
        maskUnits='userSpaceOnUse'
      >
        <path
          fill='#fff'
          fillRule='evenodd'
          d='M70.912 99.317a6.223 6.223 0 0 0 4.96-.19l20.589-9.907A6.25 6.25 0 0 0 100 83.587V16.413a6.25 6.25 0 0 0-3.54-5.632L75.874.874a6.226 6.226 0 0 0-7.104 1.21L29.355 38.04 12.187 25.01a4.162 4.162 0 0 0-5.318.236l-5.506 5.009a4.168 4.168 0 0 0-.004 6.162L16.247 50 1.36 63.583a4.168 4.168 0 0 0 .004 6.162l5.506 5.01a4.162 4.162 0 0 0 5.318.236l17.168-13.032L68.77 97.917a6.217 6.217 0 0 0 2.143 1.4ZM75.015 27.3 45.11 50l29.906 22.701V27.3Z'
          clipRule='evenodd'
        />
      </mask>
      <g mask={`url(#${mask})`}>
        <path
          fill='#0065A9'
          d='M96.461 10.796 75.857.876a6.23 6.23 0 0 0-7.107 1.207l-67.451 61.5a4.167 4.167 0 0 0 .004 6.162l5.51 5.009a4.167 4.167 0 0 0 5.32.236l81.228-61.62c2.725-2.067 6.639-.124 6.639 3.297v-.24a6.25 6.25 0 0 0-3.539-5.63Z'
        />
        <path
          fill='#007ACC'
          d='m96.461 89.204-20.604 9.92a6.229 6.229 0 0 1-7.107-1.207l-67.451-61.5a4.167 4.167 0 0 1 .004-6.162l5.51-5.009a4.167 4.167 0 0 1 5.32-.236l81.228 61.62c2.725 2.067 6.639.124 6.639-3.297v.24a6.25 6.25 0 0 1-3.539 5.63Z'
        />
        <path
          fill='#1F9CF0'
          d='M75.858 99.126a6.232 6.232 0 0 1-7.108-1.21c2.306 2.307 6.25.674 6.25-2.588V4.672c0-3.262-3.944-4.895-6.25-2.589a6.232 6.232 0 0 1 7.108-1.21l20.6 9.908A6.25 6.25 0 0 1 100 16.413v67.174a6.25 6.25 0 0 1-3.541 5.633l-20.601 9.906Z'
        />
      </g>
    </svg>
  )
}

function CursorIcon({ className }: IconProps) {
  return (
    <svg
      viewBox='0 0 466.73 532.09'
      aria-hidden='true'
      className={cn('fill-foreground shrink-0', className)}
    >
      <path d='M457.43,125.94L244.42,2.96c-6.84-3.95-15.28-3.95-22.12,0L9.3,125.94c-5.75,3.32-9.3,9.46-9.3,16.11v247.99c0,6.65,3.55,12.79,9.3,16.11l213.01,122.98c6.84,3.95,15.28,3.95,22.12,0l213.01-122.98c5.75-3.32,9.3-9.46,9.3-16.11v-247.99c0-6.65-3.55-12.79-9.3-16.11h-.01ZM444.05,151.99l-205.63,356.16c-1.39,2.4-5.06,1.42-5.06-1.36v-233.21c0-4.66-2.49-8.97-6.53-11.31L24.87,145.67c-2.4-1.39-1.42-5.06,1.36-5.06h411.26c5.84,0,9.49,6.33,6.57,11.39h-.01Z' />
    </svg>
  )
}

function ZedIcon({ className }: IconProps) {
  return (
    <svg
      viewBox='0 0 96 96'
      fill='none'
      aria-hidden='true'
      className={cn('text-foreground shrink-0', className)}
    >
      <path
        fill='currentColor'
        fillRule='evenodd'
        clipRule='evenodd'
        d='M9 6a3 3 0 0 0-3 3v66H0V9a9 9 0 0 1 9-9h80.379c4.009 0 6.016 4.847 3.182 7.682L43.055 57.187H57V51h6v7.688a4.5 4.5 0 0 1-4.5 4.5H37.055L26.743 73.5H73.5V36h6v37.5a6 6 0 0 1-6 6H20.743L10.243 90H87a3 3 0 0 0 3-3V21h6v66a9 9 0 0 1-9 9H6.621c-4.009 0-6.016-4.847-3.182-7.682L52.757 39H39v6h-6v-7.5a4.5 4.5 0 0 1 4.5-4.5h21.257l10.5-10.5H22.5V60h-6V22.5a6 6 0 0 1 6-6h52.757L85.757 6H9Z'
      />
    </svg>
  )
}

type JetBrainsEditor = Extract<
  Editor,
  | 'idea'
  | 'webstorm'
  | 'pycharm'
  | 'goland'
  | 'rider'
  | 'clion'
  | 'phpstorm'
  | 'rubymine'
  | 'rustrover'
>

const JETBRAINS_MARKS: Record<
  JetBrainsEditor,
  { letters: string; color: string }
> = {
  idea: { letters: 'IJ', color: '#FE315D' },
  webstorm: { letters: 'WS', color: '#07C3F2' },
  pycharm: { letters: 'PC', color: '#21D789' },
  goland: { letters: 'GO', color: '#3BEA62' },
  rider: { letters: 'RD', color: '#DD1265' },
  clion: { letters: 'CL', color: '#22D88F' },
  phpstorm: { letters: 'PS', color: '#B345F1' },
  rubymine: { letters: 'RM', color: '#FE2857' },
  rustrover: { letters: 'RR', color: '#FF8100' }
}

function JetBrainsIcon({
  letters,
  color,
  className
}: IconProps & { letters: string; color: string }) {
  return (
    <svg
      viewBox='0 0 24 24'
      aria-hidden='true'
      className={cn('shrink-0', className)}
    >
      <rect x='1' y='1' width='22' height='22' rx='5' fill={color} />
      <rect x='5' y='5' width='14' height='14' fill='#000' />
      <text
        x='6.5'
        y='12.6'
        fill='#fff'
        fontSize='6.4'
        fontWeight='700'
        fontFamily='system-ui, sans-serif'
      >
        {letters}
      </text>
      <rect x='6.5' y='15.6' width='5.5' height='1.3' fill='#fff' />
    </svg>
  )
}

function FileExplorerIcon({ className }: IconProps) {
  return (
    <svg
      viewBox='0 0 24 24'
      fill='none'
      aria-hidden='true'
      className={cn('shrink-0', className)}
    >
      <path
        d='M2 5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2Z'
        fill='#D99A16'
      />
      <path d='M2 9h20v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2Z' fill='#FFCE45' />
      <path d='M8 14h8v7H8Z' fill='#58B8E8' />
      <path d='M10 16h4v2h-4Z' fill='#1879B9' />
    </svg>
  )
}

function FinderIcon({ className }: IconProps) {
  return (
    <svg
      viewBox='0 0 24 24'
      fill='none'
      aria-hidden='true'
      className={cn('shrink-0', className)}
    >
      <rect x='2' y='2' width='20' height='20' rx='4' fill='#36A9F5' />
      <path
        d='M13 2h5a4 4 0 0 1 4 4v12a4 4 0 0 1-4 4h-6c-1-4-1-7 0-10H9c0-4 2-8 4-10Z'
        fill='#D9F1FF'
      />
      <path
        d='M7 7v2m10-2v2M6 15c3 3 9 3 12 0'
        stroke='#163A59'
        strokeWidth='1.5'
        strokeLinecap='round'
      />
    </svg>
  )
}
