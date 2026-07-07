import { describe, it, expect } from 'vitest'
import {
  toRemoteStreamData,
  streamNeedsDebrid,
  type StremioRawStream,
} from './StremioStreamAdapter'

describe('StremioStreamAdapter', () => {
  it('returns null when stream has no playable identifier', () => {
    const s: StremioRawStream = { title: 'Empty' }
    expect(toRemoteStreamData(s, { addonName: 'X' })).toBeNull()
  })

  it('maps a stream with a direct URL', () => {
    const s: StremioRawStream = {
      url: 'https://cdn.example.com/movie.mkv',
      title: 'Movie 2024 1080p',
    }
    const out = toRemoteStreamData(s, { addonName: 'ExampleAddon' })
    expect(out).not.toBeNull()
    expect(out!.url).toBe('https://cdn.example.com/movie.mkv')
    expect(out!.parsedQuality).toBe('1080p')
    expect(out!.parsedSource).toBe('ExampleAddon')
  })

  it('uses resolvedUrl over the raw URL when provided', () => {
    const s: StremioRawStream = {
      url: 'https://example.com/abc',
      title: 'Movie 4K',
    }
    const out = toRemoteStreamData(s, {
      addonName: 'Test Addon',
      resolvedUrl: 'https://cdn.example.com/final.mkv',
    })
    expect(out!.url).toBe('https://cdn.example.com/final.mkv')
    expect(out!.parsedQuality).toBe('4K')
  })

  it('falls back to addon name when stream has no display fields', () => {
    const s: StremioRawStream = { infoHash: 'DEADBEEF' }
    const out = toRemoteStreamData(s, { addonName: 'Test Addon' })
    expect(out).not.toBeNull()
    expect(out!.name).toBe('Test Addon stream')
  })

  it('streamNeedsDebrid is true when only infoHash is set', () => {
    expect(streamNeedsDebrid({ infoHash: 'X' })).toBe(true)
  })

  it('streamNeedsDebrid is false when URL is set', () => {
    expect(streamNeedsDebrid({ url: 'https://x', infoHash: 'X' })).toBe(false)
  })

  it('streamNeedsDebrid is false when nothing is set', () => {
    expect(streamNeedsDebrid({})).toBe(false)
  })
})
