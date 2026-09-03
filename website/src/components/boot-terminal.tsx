import { useEffect, useMemo, useRef, useState } from 'react'
import { INSTALL_COMMAND } from '@/lib/release'

type Tone = 'fg' | 'dim' | 'green' | 'dimgreen' | 'amber' | 'cyan' | 'red' | 'purple'

type Step =
  | { type: 'cmd'; text: string }
  | { type: 'out'; lines: { text: string; tone?: Tone }[] }

type DoneEntry = { step: Step; shown: number }

const TONE_CLASS: Record<Tone, string> = {
  fg: 'text-foreground',
  dim: 'text-muted-foreground',
  green: 'text-ansi-green',
  dimgreen: 'text-ansi-dimgreen',
  amber: 'text-ansi-amber',
  cyan: 'text-ansi-cyan',
  red: 'text-ansi-red',
  purple: 'text-ansi-purple',
}

const SCRIPT: Step[] = [
  { type: 'cmd', text: 'sleipnir --info' },
  {
    type: 'out',
    lines: [
      { text: 'render    gpui · metal / direct3d 11 / vulkan', tone: 'fg' },
      { text: 'pty       native · conpty on windows', tone: 'fg' },
      { text: 'config    ~/.config/sleipnir/settings.json', tone: 'cyan' },
      { text: '          zed-compatible · hot reload', tone: 'dim' },
      { text: 'session   tabs · splits · windows · restored', tone: 'fg' },
    ],
  },
  { type: 'cmd', text: INSTALL_COMMAND },
  {
    type: 'out',
    lines: [
      { text: '✓ platform detected · aarch64-apple-darwin', tone: 'dimgreen' },
      { text: '✓ checksum verified · sha256 sidecar', tone: 'dimgreen' },
      { text: '✓ installed → /Applications/Sleipnir.app', tone: 'green' },
    ],
  },
  { type: 'cmd', text: 'open -a Sleipnir' },
  {
    type: 'out',
    lines: [{ text: 'no account. no cloud. no electron. just your shell.', tone: 'purple' }],
  },
]

const TYPE_MS = 22
const LINE_MS = 95
const STEP_MS = 220

function prefersReducedMotion() {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/**
 * Scripted terminal session. Commands type themselves character by character,
 * output prints line by line, then an idle blinking prompt remains.
 */
export function BootTerminal() {
  const reduced = useMemo(prefersReducedMotion, [])
  const [done, setDone] = useState<DoneEntry[]>(
    reduced ? SCRIPT.map((step) => ({ step, shown: Number.POSITIVE_INFINITY })) : [],
  )
  const [typing, setTyping] = useState<string | null>(null)
  const [finished, setFinished] = useState(reduced)
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (reduced) return
    let cancelled = false
    const timers: ReturnType<typeof setTimeout>[] = []
    const after = (ms: number, fn: () => void) => {
      timers.push(setTimeout(fn, ms))
    }

    const showLines = (step: Step, shown: number) => {
      setDone((d) => {
        const idx = d.findIndex((e) => e.step === step)
        const entry: DoneEntry = { step, shown }
        if (idx === -1) return [...d, entry]
        const copy = [...d]
        copy[idx] = entry
        return copy
      })
    }

    let stepIndex = 0
    const runStep = () => {
      if (cancelled) return
      const step = SCRIPT[stepIndex]
      if (!step) {
        setFinished(true)
        return
      }
      if (step.type === 'cmd') {
        let i = 0
        const tick = () => {
          if (cancelled) return
          i += 1
          setTyping(step.text.slice(0, i))
          if (i < step.text.length) {
            after(TYPE_MS + Math.random() * 26, tick)
          } else {
            after(STEP_MS, () => {
              setTyping(null)
              showLines(step, Number.POSITIVE_INFINITY)
              stepIndex += 1
              runStep()
            })
          }
        }
        tick()
      } else {
        let i = 0
        const nextLine = () => {
          if (cancelled) return
          i += 1
          showLines(step, i)
          if (i < step.lines.length) {
            after(LINE_MS, nextLine)
          } else {
            after(STEP_MS, () => {
              stepIndex += 1
              runStep()
            })
          }
        }
        nextLine()
      }
    }
    after(500, runStep)

    return () => {
      cancelled = true
      timers.forEach(clearTimeout)
    }
  }, [reduced])

  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [done, typing])

  return (
    <div ref={scrollRef} className="min-h-[300px] overflow-hidden md:min-h-[340px]">
      {done.map(({ step, shown }, si) =>
        step.type === 'cmd' ? (
          <div key={si} className="whitespace-pre-wrap wrap-anywhere">
            <span className="text-ansi-green">$ </span>
            <span className="text-foreground">{step.text}</span>
          </div>
        ) : (
          <div key={si}>
            {step.lines.slice(0, shown).map((line, i) => (
              <div key={i} className="boot-line whitespace-pre-wrap wrap-anywhere">
                <span className={TONE_CLASS[line.tone ?? 'fg']}>{line.text}</span>
              </div>
            ))}
          </div>
        ),
      )}
      {typing !== null && (
        <div className="whitespace-pre-wrap wrap-anywhere">
          <span className="text-ansi-green">$ </span>
          <span className="text-foreground">{typing}</span>
          <span className="block-cursor" aria-hidden />
        </div>
      )}
      {finished && (
        <div>
          <span className="text-ansi-green">$ </span>
          <span className="block-cursor" aria-hidden />
        </div>
      )}
    </div>
  )
}
