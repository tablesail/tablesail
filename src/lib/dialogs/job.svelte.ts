import { api, errorMessage } from '../api'
import type { JobProgress } from '../types'

/** Polls a background export/import job until it finishes. */
export class JobPoller {
  jobId = $state<string | null>(null)
  progress = $state<JobProgress | null>(null)
  error = $state<string | null>(null)
  startedAt = 0
  elapsedMs = $state(0)

  private stopped = false

  get running(): boolean {
    return this.jobId !== null && !(this.progress?.done || this.progress?.cancelled || this.progress?.error)
  }

  get finished(): boolean {
    return !!this.progress && !this.running
  }

  async track(jobId: string) {
    this.jobId = jobId
    this.progress = { rows: 0, bytes: 0, done: false, cancelled: false, error: null }
    this.startedAt = performance.now()
    while (!this.stopped) {
      try {
        const p = await api.jobStatus(jobId)
        this.progress = p
        this.elapsedMs = performance.now() - this.startedAt
        if (p.done || p.cancelled || p.error) return
      } catch (e) {
        this.error = errorMessage(e)
        this.progress = { rows: this.progress?.rows ?? 0, bytes: this.progress?.bytes ?? 0, done: false, cancelled: false, error: this.error }
        return
      }
      await new Promise((r) => setTimeout(r, 200))
    }
  }

  async cancel() {
    if (!this.jobId) return
    try {
      await api.jobCancel(this.jobId)
    } catch (e) {
      this.error = errorMessage(e)
    }
  }

  dispose() {
    this.stopped = true
  }
}
