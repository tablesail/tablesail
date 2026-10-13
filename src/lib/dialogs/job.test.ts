import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { routeInvoke, settle, type InvokeHandlers } from '../../test/helpers'
import type { JobProgress } from '../types'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => core)

import { JobPoller } from './job.svelte'

const prog = (over: Partial<JobProgress> = {}): JobProgress => ({ rows: 0, bytes: 0, done: false, cancelled: false, error: null, ...over })

function install(statuses: (JobProgress | Error)[], handlers: InvokeHandlers = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = []
  let i = 0
  const routed = routeInvoke({
    job_status: () => {
      const next = statuses[Math.min(i++, statuses.length - 1)]
      if (next instanceof Error) throw next
      return next
    },
    job_cancel: () => undefined,
    ...handlers,
  })
  core.invoke.mockReset()
  core.invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args })
    return routed(command, args)
  })
  return calls
}

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
})

describe('JobPoller: initial state', () => {
  it('is idle', () => {
    const j = new JobPoller()
    expect(j.jobId).toBeNull()
    expect(j.progress).toBeNull()
    expect(j.error).toBeNull()
    expect(j.elapsedMs).toBe(0)
    expect(j.running).toBe(false)
    expect(j.finished).toBe(false)
  })
})

describe('JobPoller.track', () => {
  it('polls the job and stops when it is done', async () => {
    const calls = install([prog({ rows: 10 }), prog({ rows: 50 }), prog({ rows: 100, bytes: 2048, done: true })])
    const j = new JobPoller()
    const p = j.track('job-1')
    expect(j.jobId).toBe('job-1')
    expect(j.running).toBe(true)
    await vi.advanceTimersByTimeAsync(1000)
    await p
    expect(calls.filter((c) => c.command === 'job_status').map((c) => c.args)).toEqual([{ jobId: 'job-1' }, { jobId: 'job-1' }, { jobId: 'job-1' }])
    expect(j.progress).toEqual(prog({ rows: 100, bytes: 2048, done: true }))
    expect(j.running).toBe(false)
    expect(j.finished).toBe(true)
  })

  it('starts with a zeroed progress before the first answer', async () => {
    install([prog({ done: true })])
    const j = new JobPoller()
    const p = j.track('j')
    expect(j.progress).toEqual(prog())
    await p
  })

  it('polls every 200 ms', async () => {
    const calls = install([prog({ rows: 1 }), prog({ rows: 2 }), prog({ done: true })])
    const j = new JobPoller()
    const p = j.track('j')
    await settle(10)
    expect(calls).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(199)
    expect(calls).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(calls).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(200)
    await p
    expect(calls).toHaveLength(3)
  })

  it('reflects progress while the job runs', async () => {
    install([prog({ rows: 10 }), prog({ rows: 99 }), prog({ done: true, rows: 120 })])
    const j = new JobPoller()
    const p = j.track('j')
    await settle(10)
    expect(j.progress?.rows).toBe(10)
    expect(j.running).toBe(true)
    expect(j.finished).toBe(false)
    await vi.advanceTimersByTimeAsync(200)
    expect(j.progress?.rows).toBe(99)
    await vi.advanceTimersByTimeAsync(200)
    await p
    expect(j.progress?.rows).toBe(120)
  })

  it('tracks elapsed time', async () => {
    install([prog(), prog(), prog({ done: true })])
    const j = new JobPoller()
    const p = j.track('j')
    await vi.advanceTimersByTimeAsync(500)
    await p
    expect(j.elapsedMs).toBeGreaterThanOrEqual(0)
    expect(Number.isFinite(j.elapsedMs)).toBe(true)
  })

  it('a cancelled job finishes the poll', async () => {
    const calls = install([prog({ rows: 5 }), prog({ cancelled: true, rows: 5 })])
    const j = new JobPoller()
    const p = j.track('j')
    await vi.advanceTimersByTimeAsync(1000)
    await p
    expect(j.running).toBe(false)
    expect(j.finished).toBe(true)
    expect(j.progress?.cancelled).toBe(true)
    expect(calls.filter((c) => c.command === 'job_status')).toHaveLength(2)
  })

  it('a failed job finishes the poll and keeps its message in progress', async () => {
    install([prog({ error: 'disk full', rows: 3 })])
    const j = new JobPoller()
    await j.track('j')
    expect(j.running).toBe(false)
    expect(j.finished).toBe(true)
    expect(j.progress?.error).toBe('disk full')
    expect(j.error).toBeNull() // `error` is for failures of the status call itself
  })

  it('a failing status call stops the poll, keeps the counters and surfaces the error', async () => {
    install([prog({ rows: 40, bytes: 900 }), new Error('backend unreachable')])
    const j = new JobPoller()
    const p = j.track('j')
    await vi.advanceTimersByTimeAsync(1000)
    await p
    expect(j.error).toBe('backend unreachable')
    expect(j.progress).toEqual(prog({ rows: 40, bytes: 900, error: 'backend unreachable' }))
    expect(j.running).toBe(false)
    expect(j.finished).toBe(true)
  })

  it('a status failure on the very first call starts from zero counters', async () => {
    install([new Error('nope')])
    const j = new JobPoller()
    await j.track('j')
    expect(j.progress).toEqual(prog({ error: 'nope' }))
  })

  it('formats CoreError rejections', async () => {
    install([prog()], {
      job_status: () => {
        throw { kind: 'notFound', message: 'no such job', code: null, statement: null }
      },
    })
    const j = new JobPoller()
    await j.track('gone')
    expect(j.error).toBe('no such job')
  })

  it('surfaces a job warning carried in the final progress', async () => {
    install([prog({ done: true, warning: 'long values truncated' })])
    const j = new JobPoller()
    await j.track('j')
    expect(j.progress?.warning).toBe('long values truncated')
  })
})

describe('JobPoller.cancel', () => {
  it('asks the backend to cancel the job being tracked', async () => {
    const calls = install([prog(), prog(), prog({ cancelled: true })])
    const j = new JobPoller()
    const p = j.track('job-9')
    await settle(10)
    await j.cancel()
    expect(calls.find((c) => c.command === 'job_cancel')?.args).toEqual({ jobId: 'job-9' })
    await vi.advanceTimersByTimeAsync(1000)
    await p
  })

  it('does nothing before a job is tracked', async () => {
    const calls = install([prog()])
    await new JobPoller().cancel()
    expect(calls).toEqual([])
  })

  it('records a failure to cancel', async () => {
    install([prog()], {
      job_cancel: () => {
        throw new Error('cannot cancel')
      },
    })
    const j = new JobPoller()
    const p = j.track('j')
    await settle(10)
    await j.cancel()
    expect(j.error).toBe('cannot cancel')
    j.dispose()
    await vi.advanceTimersByTimeAsync(1000)
    await p
  })
})

describe('JobPoller.dispose', () => {
  it('stops polling', async () => {
    const calls = install([prog()])
    const j = new JobPoller()
    const p = j.track('j')
    await settle(10)
    j.dispose()
    await vi.advanceTimersByTimeAsync(5000)
    await p
    expect(calls.filter((c) => c.command === 'job_status')).toHaveLength(1)
  })

  it('disposing before tracking makes track a no-op', async () => {
    const calls = install([prog({ done: true })])
    const j = new JobPoller()
    j.dispose()
    await j.track('j')
    expect(calls).toEqual([])
    expect(j.jobId).toBe('j')
  })
})
