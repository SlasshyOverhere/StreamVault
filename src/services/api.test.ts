import { describe, it, expect, vi, beforeEach } from 'vitest'
import {
  getPosterUrl,
  getPlayerPreference,
  setPlayerPreference,
  getSeriesAudioPreference,
  setSeriesAudioPreference,
  getSeriesSubtitlePreference,
  setSeriesSubtitlePreference,
  getSeriesSpoilerEnabled,
  setSeriesSpoilerEnabled,
  getCachedSeriesAudioTracks,
  setCachedSeriesAudioTracks,
  getCachedSeriesSubtitleTracks,
  setCachedSeriesSubtitleTracks,
  mergeCachedSeriesAudioTracks,
  mergeCachedSeriesSubtitleTracks,
  resolveSeriesAudioPreferenceForPlayback,
  resolveSeriesSubtitlePreferenceForPlayback,
  getTmdbImageUrl,
  getTabVisibility,
  setTabVisibility,
  isBetaEnabled,
  setBetaEnabled,
} from './api'
import type { MediaItem, AudioTrackOption } from './api'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}))

// Mock localStorage
const store: Record<string, string> = {}
const localStorageMock = {
  getItem: vi.fn((key: string) => store[key] ?? null),
  setItem: vi.fn((key: string, value: string) => { store[key] = value }),
  removeItem: vi.fn((key: string) => { delete store[key] }),
  clear: vi.fn(() => { for (const k in store) delete store[k] }),
  get length() { return Object.keys(store).length },
  key: vi.fn((i: number) => Object.keys(store)[i] ?? null),
}
vi.stubGlobal('localStorage', localStorageMock)

beforeEach(() => {
  vi.clearAllMocks()
  for (const k in store) delete store[k]
})

describe('getPosterUrl', () => {
  it('returns null when no poster_path', () => {
    expect(getPosterUrl({ id: 1, title: 'X', media_type: 'movie' } as MediaItem)).toBeNull()
  })

  it('returns http URLs as-is', () => {
    expect(getPosterUrl({ id: 1, title: 'X', media_type: 'movie', poster_path: 'https://example.com/p.jpg' } as MediaItem))
      .toBe('https://example.com/p.jpg')
  })

  it('returns asset:// URLs as-is', () => {
    expect(getPosterUrl({ id: 1, title: 'X', media_type: 'movie', poster_path: 'asset:///foo' } as MediaItem))
      .toBe('asset:///foo')
  })

  it('returns null for relative paths (caller must use async getCachedImageUrl)', () => {
    expect(getPosterUrl({ id: 1, title: 'X', media_type: 'movie', poster_path: '/abc123.jpg' } as MediaItem))
      .toBeNull()
  })
})

describe('getTmdbImageUrl', () => {
  it('returns null for undefined path', () => {
    expect(getTmdbImageUrl(undefined)).toBeNull()
  })

  it('builds URL with default size', () => {
    expect(getTmdbImageUrl('/poster.jpg')).toBe('https://image.tmdb.org/t/p/w300/poster.jpg')
  })

  it('builds URL with custom size', () => {
    expect(getTmdbImageUrl('/backdrop.jpg', 'original')).toBe('https://image.tmdb.org/t/p/original/backdrop.jpg')
    expect(getTmdbImageUrl('/img.jpg', 'w92')).toBe('https://image.tmdb.org/t/p/w92/img.jpg')
  })
})

describe('playerPreference', () => {
  it('defaults to "ask"', () => {
    expect(getPlayerPreference()).toBe('ask')
  })

  it('round-trips through localStorage', () => {
    setPlayerPreference('mpv')
    expect(getPlayerPreference()).toBe('mpv')
    setPlayerPreference('vlc')
    expect(getPlayerPreference()).toBe('vlc')
  })
})

describe('seriesAudioPreference', () => {
  it('returns null when not set', () => {
    expect(getSeriesAudioPreference(1)).toBeNull()
  })

  it('round-trips through localStorage', () => {
    setSeriesAudioPreference(42, 'jpn')
    expect(getSeriesAudioPreference(42)).toBe('jpn')
  })

  it('returns null for empty/whitespace strings', () => {
    setSeriesAudioPreference(1, '  ')
    expect(getSeriesAudioPreference(1)).toBeNull()
  })

  it('deletes preference when set to null', () => {
    setSeriesAudioPreference(1, 'eng')
    setSeriesAudioPreference(1, null)
    expect(getSeriesAudioPreference(1)).toBeNull()
  })
})

describe('seriesSubtitlePreference', () => {
  it('returns null when not set', () => {
    expect(getSeriesSubtitlePreference(1)).toBeNull()
  })

  it('round-trips through localStorage', () => {
    setSeriesSubtitlePreference(5, 'eng')
    expect(getSeriesSubtitlePreference(5)).toBe('eng')
  })

  it('deletes preference when set to null', () => {
    setSeriesSubtitlePreference(1, 'eng')
    setSeriesSubtitlePreference(1, null)
    expect(getSeriesSubtitlePreference(1)).toBeNull()
  })
})

describe('seriesSpoilerEnabled', () => {
  it('defaults to true when not set', () => {
    expect(getSeriesSpoilerEnabled(1)).toBe(true)
  })

  it('returns true when explicitly enabled (deleted from map)', () => {
    setSeriesSpoilerEnabled(1, true)
    expect(getSeriesSpoilerEnabled(1)).toBe(true)
  })

  it('returns false when explicitly disabled', () => {
    setSeriesSpoilerEnabled(1, false)
    expect(getSeriesSpoilerEnabled(1)).toBe(false)
  })
})

describe('cachedAudioTracks', () => {
  it('returns null when not cached', () => {
    expect(getCachedSeriesAudioTracks(1)).toBeNull()
  })

  it('round-trips through localStorage', () => {
    const tracks: AudioTrackOption[] = [
      { stream_index: 0, label: 'English', detail: 'Stereo', language_code: 'eng' },
      { stream_index: 1, label: 'Japanese', detail: 'Surround', language_code: 'jpn' },
    ]
    setCachedSeriesAudioTracks(10, tracks)
    expect(getCachedSeriesAudioTracks(10)).toEqual(tracks)
  })
})

describe('cachedSubtitleTracks', () => {
  it('returns null when not cached', () => {
    expect(getCachedSeriesSubtitleTracks(1)).toBeNull()
  })

  it('round-trips through localStorage', () => {
    const tracks: AudioTrackOption[] = [
      { stream_index: 2, label: 'English', detail: 'Forced' },
    ]
    setCachedSeriesSubtitleTracks(7, tracks)
    expect(getCachedSeriesSubtitleTracks(7)).toEqual(tracks)
  })
})

describe('mergeCachedSeriesAudioTracks', () => {
  it('stores tracks when no existing cache', () => {
    const tracks: AudioTrackOption[] = [
      { stream_index: 0, label: 'English', detail: '' },
    ]
    mergeCachedSeriesAudioTracks(1, tracks)
    expect(getCachedSeriesAudioTracks(1)).toHaveLength(1)
  })

  it('merges new tracks with existing ones', () => {
    const existing: AudioTrackOption[] = [
      { stream_index: 0, label: 'English', detail: '', language_code: 'eng' },
    ]
    setCachedSeriesAudioTracks(1, existing)

    const incoming: AudioTrackOption[] = [
      { stream_index: 1, label: 'Japanese', detail: '', language_code: 'jpn' },
    ]
    mergeCachedSeriesAudioTracks(1, incoming)
    const result = getCachedSeriesAudioTracks(1)
    expect(result).toHaveLength(2)
  })

  it('deduplicates identical tracks', () => {
    const track: AudioTrackOption = { stream_index: 0, label: 'English', detail: '', language_code: 'eng' }
    setCachedSeriesAudioTracks(1, [track])
    mergeCachedSeriesAudioTracks(1, [track])
    expect(getCachedSeriesAudioTracks(1)).toHaveLength(1)
  })
})

describe('mergeCachedSeriesSubtitleTracks', () => {
  it('stores tracks when no existing cache', () => {
    mergeCachedSeriesSubtitleTracks(1, [{ stream_index: 0, label: 'English', detail: '' }])
    expect(getCachedSeriesSubtitleTracks(1)).toHaveLength(1)
  })
})

describe('resolveSeriesAudioPreferenceForPlayback', () => {
  it('returns null for null/undefined seriesId', () => {
    expect(resolveSeriesAudioPreferenceForPlayback(null)).toBeNull()
    expect(resolveSeriesAudioPreferenceForPlayback(undefined)).toBeNull()
  })

  it('returns null when no preference stored', () => {
    expect(resolveSeriesAudioPreferenceForPlayback(1)).toBeNull()
  })

  it('returns stored preference when no cached tracks', () => {
    setSeriesAudioPreference(1, 'jpn')
    expect(resolveSeriesAudioPreferenceForPlayback(1)).toBe('jpn')
  })

  it('returns mpv_value when cached track matches preference', () => {
    setSeriesAudioPreference(1, 'Japanese')
    setCachedSeriesAudioTracks(1, [
      { stream_index: 0, label: 'Japanese', detail: '', language_code: 'jpn', mpv_value: 'aid=2' },
    ])
    expect(resolveSeriesAudioPreferenceForPlayback(1)).toBe('aid=2')
  })

  it('falls back to stored preference when no track matches', () => {
    setSeriesAudioPreference(1, 'Korean')
    setCachedSeriesAudioTracks(1, [
      { stream_index: 0, label: 'English', detail: '', language_code: 'eng', mpv_value: 'aid=1' },
    ])
    expect(resolveSeriesAudioPreferenceForPlayback(1)).toBe('Korean')
  })
})

describe('resolveSeriesSubtitlePreferenceForPlayback', () => {
  it('returns null for null/undefined seriesId', () => {
    expect(resolveSeriesSubtitlePreferenceForPlayback(null)).toBeNull()
  })

  it('returns stored preference when no cached tracks', () => {
    setSeriesSubtitlePreference(1, 'eng')
    expect(resolveSeriesSubtitlePreferenceForPlayback(1)).toBe('eng')
  })
})

describe('tabVisibility', () => {
  it('defaults to cloud-only mode', () => {
    expect(getTabVisibility()).toEqual({ showLocal: false, showCloud: true })
  })

  it('round-trips through localStorage', () => {
    setTabVisibility({ showLocal: true, showCloud: true })
    expect(getTabVisibility()).toEqual({ showLocal: true, showCloud: true })
  })
})

describe('betaFeatures', () => {
  it('defaults to disabled', () => {
    expect(isBetaEnabled()).toBe(false)
  })

  it('round-trips through localStorage', () => {
    setBetaEnabled(true)
    expect(isBetaEnabled()).toBe(true)
    setBetaEnabled(false)
    expect(isBetaEnabled()).toBe(false)
  })
})
