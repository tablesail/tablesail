// Measured round-trip times per session (table pages and queries), for the latency card.

const KEEP = 40

class PulseStore {
  samples = $state<Record<string, number[]>>({})

  record(sessionId: string, ms: number) {
    if (!Number.isFinite(ms) || ms < 0) return
    const list = [...(this.samples[sessionId] ?? []), ms].slice(-KEEP)
    this.samples[sessionId] = list
  }

  for(sessionId: string): number[] {
    return this.samples[sessionId] ?? []
  }
}

export const pulse = new PulseStore()
