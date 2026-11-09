import { cn } from '@hoshi/utils'
import { useRef, useState } from 'react'
import { m } from '@/paraglide/messages'
import { AgentStep } from './AgentStep'
import { ProjectStep } from './ProjectStep'
import { HugeiconsIcon } from '@hugeicons/react'
import { useThemeStore } from '@/state/theme-store'
import { useNavigate } from '@tanstack/react-router'
import { usePaletteStore } from '@/state/palette-store'
import { Button } from '@hoshi/ui/components/button'
import { useSettingsStore } from '@/state/settings-store'
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { LogoMark, Wordmark } from '@/components/brand/Logo'
import { THEME_OPTIONS } from '@/components/settings/AppearanceSection'
import {
  Wizard,
  WizardDescription,
  WizardFooter,
  WizardHeader,
  WizardPanel,
  WizardSteps,
  WizardTitle
} from '@hoshi/ui/components/wizard'

const STEPS = [
  m.onboarding_step_welcome,
  m.onboarding_step_agent,
  m.onboarding_step_project
]

export function WelcomeWizard() {
  const navigate = useNavigate()
  const [open, setOpen] = useState(true)
  const [step, setStep] = useState(0)
  const [projectId, setProjectId] = useState<string | null>(null)
  const afterExit = useRef<(() => void) | null>(null)
  const continueRef = useRef<HTMLButtonElement>(null)

  const exit = (then?: () => void) => {
    if (!open) return
    afterExit.current = then ?? null
    void useSettingsStore.getState().completeOnboarding()
    setOpen(false)
  }

  const exited = async () => {
    await navigate({ to: '/', replace: true })
    afterExit.current?.()
  }

  const next = (
    <Button ref={continueRef} onClick={() => setStep(step + 1)}>
      {m.onboarding_continue()}
      <HugeiconsIcon
        icon={ArrowRight01Icon}
        strokeWidth={2}
        data-icon='inline-end'
      />
    </Button>
  )
  const skip = (
    <Button variant='ghost' onClick={() => exit()}>
      {m.onboarding_skip()}
    </Button>
  )

  return (
    <Wizard
      open={open}
      label={m.onboarding()}
      initialFocus={continueRef}
      topBar={<div data-tauri-drag-region className='h-10' />}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) exit()
      }}
      onExited={() => void exited()}
    >
      <WizardHeader
        closeLabel={m.onboarding_close()}
        brand={
          <span
            role='img'
            aria-label={m.app_name()}
            className='flex items-center gap-2'
          >
            <LogoMark className='size-5' />
            <Wordmark className='h-4 w-auto' />
          </span>
        }
      >
        <WizardSteps
          label={m.onboarding_progress()}
          steps={STEPS.map((label) => label())}
          current={step}
          onSelect={setStep}
        />
      </WizardHeader>
      <WizardPanel className='min-h-80'>
        {step === 0 ? <WelcomeStep /> : null}
        {step === 1 ? <AgentStep /> : null}
        {step === 2 ? (
          <ProjectStep
            projectId={projectId}
            onAdded={setProjectId}
            onAddOrClone={() =>
              exit(() => usePaletteStore.getState().setOpen(true, 'addProject'))
            }
          />
        ) : null}
      </WizardPanel>
      {step < 2 ? (
        <WizardFooter leading={skip}>{next}</WizardFooter>
      ) : (
        <WizardFooter>
          {projectId ? (
            <Button onClick={() => exit()}>
              {m.onboarding_finish()}
              <HugeiconsIcon
                icon={ArrowRight01Icon}
                strokeWidth={2}
                data-icon='inline-end'
              />
            </Button>
          ) : (
            <Button variant='ghost' onClick={() => exit()}>
              {m.onboarding_skip_for_now()}
            </Button>
          )}
        </WizardFooter>
      )}
    </Wizard>
  )
}

function WelcomeStep() {
  const preference = useThemeStore((state) => state.preference)
  const setPreference = useThemeStore((state) => state.setPreference)

  return (
    <>
      <WizardTitle>{m.onboarding_welcome_title()}</WizardTitle>
      <WizardDescription>
        {m.onboarding_welcome_description()}
      </WizardDescription>
      <fieldset className='mt-6 grid grid-cols-3 gap-3'>
        <legend className='sr-only'>{m.theme()}</legend>
        {THEME_OPTIONS.map(({ value, label, icon }) => {
          const selected = preference === value
          return (
            <button
              key={value}
              type='button'
              aria-pressed={selected}
              onClick={() => setPreference(value)}
              className={cn(
                'bg-background focus-visible:ring-ring flex cursor-pointer flex-col items-center gap-2.5 rounded-xl border px-4 py-5 text-sm font-medium transition-colors outline-none focus-visible:ring-2',
                selected
                  ? 'border-primary ring-primary text-foreground ring-1'
                  : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground'
              )}
            >
              <HugeiconsIcon icon={icon} strokeWidth={2} className='size-5' />
              {label()}
            </button>
          )
        })}
      </fieldset>
    </>
  )
}
