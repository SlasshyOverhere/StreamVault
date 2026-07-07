import { describe, it, expect, vi, beforeEach } from 'vitest'

// Mock Tauri invoke + convertFileSrc
const mockInvoke = vi.fn()
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
  convertFileSrc: vi.fn((p: string) => `asset://${p}`),
}))

// Mock localStorage
const store: Record<string, string> = {}
vi.stubGlobal('localStorage', {
  getItem: vi.fn((k: string) => store[k] ?? null),
  setItem: vi.fn((k: string, v: string) => { store[k] = v }),
  removeItem: vi.fn((k: string) => { delete store[k] }),
  clear: vi.fn(() => { for (const k in store) delete store[k] }),
})

// Mock window for saveConfig
vi.stubGlobal('window', {
  dispatchEvent: vi.fn(),
  setTimeout: globalThis.setTimeout,
})

beforeEach(() => {
  vi.resetAllMocks()
  for (const k in store) delete store[k]
})

// ─── helpers that return [] / default on error ──────────────────────────

describe('service wrappers: return default on error', async () => {
  const mod = await import('./api')

  const cases: [string, () => Promise<unknown>, unknown, string, boolean?][] = [
    ['getLibrary', () => mod.getLibrary('movie'), [], 'get_library'],
    ['getLibraryFiltered', () => mod.getLibraryFiltered('tv', '', true), [], 'get_library_filtered'],
    ['getDdlMedia', () => mod.getDdlMedia('movie'), [], 'get_ddl_media'],
    ['getRecentlyAdded', () => mod.getRecentlyAdded(5), [], 'get_recently_added'],
    ['getWatchHistory', () => mod.getWatchHistory(), [], 'get_watch_history'],
    ['getWatchHistoryEvents', () => mod.getWatchHistoryEvents(), [], 'get_watch_history_events'],
    ['getEpisodes', () => mod.getEpisodes(1), [], 'get_episodes'],
    ['getAudioTracks', () => mod.getAudioTracks(1), [], 'get_audio_tracks'],
    ['getSubtitleTracks', () => mod.getSubtitleTracks(1), [], 'get_subtitle_tracks'],
    ['getEpisodesForDelete', () => mod.getEpisodesForDelete(1), [], 'get_episodes_for_delete'],
    ['getActiveMpvSessions', () => mod.getActiveMpvSessions(), [], 'get_active_mpv_sessions', true],
  ]

  for (const [name, fn, defaultVal, cmd, noArgs] of cases) {
    it(`${name} returns data on success`, async () => {
      mockInvoke.mockResolvedValueOnce(['item'])
      expect(await fn()).toEqual(['item'])
      if (noArgs) {
        expect(mockInvoke).toHaveBeenCalledWith(cmd)
      } else {
        expect(mockInvoke).toHaveBeenCalledWith(cmd, expect.any(Object))
      }
    })

    it(`${name} returns default on error`, async () => {
      mockInvoke.mockRejectedValueOnce(new Error('fail'))
      expect(await fn()).toEqual(defaultVal)
    })
  }
})

// ─── helpers that return specific defaults on error ─────────────────────

describe('service wrappers: specific defaults on error', async () => {
  const mod = await import('./api')

  it('getLibraryStats returns zeros on error', async () => {
    mockInvoke.mockResolvedValueOnce({ movies: 5, shows: 3, episodes: 20 })
    expect(await mod.getLibraryStats()).toEqual({ movies: 5, shows: 3, episodes: 20 })
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getLibraryStats()).toEqual({ movies: 0, shows: 0, episodes: 0 })
  })

  it('getConfig returns {} on error', async () => {
    mockInvoke.mockResolvedValueOnce({ mpv_path: '/usr/bin/mpv' })
    expect(await mod.getConfig()).toEqual({ mpv_path: '/usr/bin/mpv' })
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getConfig()).toEqual({})
  })

  it('getResumeInfo returns default on error', async () => {
    const defaultInfo = { has_progress: false, position: 0, duration: 0, time_str: '00:00:00', progress_percent: 0 }
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getResumeInfo(1)).toEqual(defaultInfo)
  })

  it('getMpvStatus returns default on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getMpvStatus(42)).toEqual({ is_playing: false, media_id: 42 })
  })

  it('getMediaTechnicalDetails returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getMediaTechnicalDetails(1)).toBeNull()
  })

  it('checkNeedsTranscode returns false on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.checkNeedsTranscode('/path')).toBe(false)
  })

  it('getMovieDetails returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getMovieDetails(1)).toBeNull()
  })

  it('getTvDetails returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getTvDetails(1)).toBeNull()
  })

  it('getTvSeasonEpisodes returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getTvSeasonEpisodes(1, 1)).toBeNull()
  })

  it('getEpisodeImdbRatings returns {} on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getEpisodeImdbRatings(1, 1, [1, 2])).toEqual({})
  })

  it('getAppVersion returns "0.0.0" on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getAppVersion()).toBe('0.0.0')
  })

  it('isGdriveConnected returns false on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.isGdriveConnected()).toBe(false)
  })

  it('getGdriveAccountInfo returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getGdriveAccountInfo()).toBeNull()
  })

  it('getAnalyticsData returns empty structure on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    const result = await mod.getAnalyticsData()
    expect(result.overview.total_watch_time_seconds).toBe(0)
    expect(result.heatmap).toEqual([])
  })

  it('searchContent throws on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    await expect(mod.searchContent('query')).rejects.toThrow('fail')
  })

  it('getTmdbReviews returns [] on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getTmdbReviews(1, 'movie')).toEqual([])
  })

  it('getImdbDetails returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getImdbDetails({ imdbId: 'tt123' })).toBeNull()
  })

  it('getDownloadJobs returns [] on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getDownloadJobs()).toEqual([])
  })

  it('getGdriveAccountInfo returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getGdriveAccountInfo()).toBeNull()
  })

  it('wtGetRoomState returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.wtGetRoomState()).toBeNull()
  })

  it('wtIsActive returns false on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.wtIsActive()).toBe(false)
  })

  it('wtGetClientId returns "" on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.wtGetClientId()).toBe('')
  })

  it('wtGetClientId returns "" when invoke returns null', async () => {
    mockInvoke.mockResolvedValueOnce(null)
    expect(await mod.wtGetClientId()).toBe('')
  })

  it('getBundledMpvInfo returns default on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getBundledMpvInfo()).toEqual({ exists: false, path: '' })
  })
})

// ─── helpers that throw on error ────────────────────────────────────────

describe('service wrappers: throw on error', async () => {
  const mod = await import('./api')

  const cases: [string, () => Promise<unknown>][] = [
    ['deleteMediaFiles', () => mod.deleteMediaFiles([1])],
    ['deleteSeries', () => mod.deleteSeries(1, true)],
    ['deleteSeriesCloudFolder', () => mod.deleteSeriesCloudFolder(1)],
    ['saveConfig', () => mod.saveConfig({})],
    ['autoDetectMpv', () => mod.autoDetectMpv()],
    ['downloadBundledMpv', () => mod.downloadBundledMpv()],
    ['getMediaInfo', () => mod.getMediaInfo(1)],
    ['getStreamUrl', () => mod.getStreamUrl(1)],
    ['getStreamUrlWithTranscode', () => mod.getStreamUrlWithTranscode(1)],
    ['startTranscodeStream', () => mod.startTranscodeStream('/path')],
    ['clearProgress', () => mod.clearProgress(1)],
    ['playMedia', () => mod.playMedia(1, false)],
    ['playMediaNative', () => mod.playMediaNative(1, false)],
    ['playWithVlc', () => mod.playWithVlc(1, false)],
    ['resolveWatchHistoryMedia', () => mod.resolveWatchHistoryMedia({} as any)],
    ['removeFromWatchHistory', () => mod.removeFromWatchHistory(1)],
    ['removeWatchHistoryEntry', () => mod.removeWatchHistoryEntry('evt-1')],
    ['clearAllWatchHistory', () => mod.clearAllWatchHistory()],
    ['syncWatchHistory', () => mod.syncWatchHistory()],
    ['markAsComplete', () => mod.markAsComplete(1)],
    ['checkForUpdates', () => mod.checkForUpdates()],
    ['downloadUpdate', () => mod.downloadUpdate('https://example.com')],
    ['installUpdate', () => mod.installUpdate('/path')],
    ['wtCreateRoom', () => mod.wtCreateRoom(1, 'title', undefined, 'nick')],
    ['wtJoinRoom', () => mod.wtJoinRoom('ABC', 1, 'title', undefined, 'nick')],
    ['wtLeaveRoom', () => mod.wtLeaveRoom()],
    ['wtSetReady', () => mod.wtSetReady(120)],
    ['wtStartPlayback', () => mod.wtStartPlayback()],
    ['wtSendSync', () => mod.wtSendSync('play', 0)],
    ['wtSendMpvCommand', () => mod.wtSendMpvCommand('s', 'play', 0)],
    ['clearCloudCache', () => mod.clearCloudCache()],
    ['searchTmdb', () => mod.searchTmdb('query')],
    ['getTmdbTrending', () => mod.getTmdbTrending()],
    ['refreshSeriesMetadata', () => mod.refreshSeriesMetadata(1)],
  ]

  for (const [name, fn] of cases) {
    it(`${name} throws on error`, async () => {
      mockInvoke.mockRejectedValueOnce(new Error('boom'))
      await expect(fn()).rejects.toThrow('boom')
    })
  }
})

// ─── swallow-error wrappers (void return, no throw) ─────────────────────

describe('service wrappers: swallow errors', async () => {
  const mod = await import('./api')

  it('updateWatchProgress does not throw', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    await expect(mod.updateWatchProgress(1, 10, 100)).resolves.toBeUndefined()
  })

  it('updateEpisodeDuration does not throw', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    await expect(mod.updateEpisodeDuration(1, 120)).resolves.toBeUndefined()
  })

  it('stopTranscodeStream does not throw', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    await expect(mod.stopTranscodeStream(1)).resolves.toBeUndefined()
  })
})

// ─── no-try/catch direct invoke wrappers ────────────────────────────────

describe('service wrappers: direct invoke (no try/catch)', async () => {
  const mod = await import('./api')

  it('getTmdbReleaseSchedule calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ tmdbId: 1 })
    const result = await mod.getTmdbReleaseSchedule(1, 'movie')
    expect(result).toEqual({ tmdbId: 1 })
  })

  it('getMovieReminders calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce([{ id: 1 }])
    expect(await mod.getMovieReminders()).toEqual([{ id: 1 }])
  })

  it('getMovieReminders(true) passes includeInactive', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.getMovieReminders(true)
    expect(mockInvoke).toHaveBeenCalledWith('get_movie_reminders', { includeInactive: true })
  })

  it('createMovieReminder calls invoke with backend input', async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1 })
    await mod.createMovieReminder({ tmdbId: '123', mediaType: 'movie', title: 'X', reminderAt: '2026-01-01' })
    expect(mockInvoke).toHaveBeenCalledWith('create_movie_reminder', expect.objectContaining({
      reminder: expect.objectContaining({ tmdbId: '123', source: 'manual', trackingMode: 'single' }),
    }))
  })

  it('updateMovieReminder calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1 })
    await mod.updateMovieReminder(1, { tmdbId: '123', mediaType: 'movie', title: 'X', reminderAt: '2026-01-01' })
    expect(mockInvoke).toHaveBeenCalledWith('update_movie_reminder', expect.objectContaining({ id: 1 }))
  })

  it('deleteMovieReminder calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.deleteMovieReminder(1)
    expect(mockInvoke).toHaveBeenCalledWith('delete_movie_reminder', { id: 1 })
  })

  it('setMovieReminderActive calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1 })
    await mod.setMovieReminderActive(1, false)
    expect(mockInvoke).toHaveBeenCalledWith('set_movie_reminder_active', { id: 1, isActive: false })
  })

  it('getWatchlistItems calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.getWatchlistItems()
    expect(mockInvoke).toHaveBeenCalledWith('get_watchlist_items', { includeInactive: false })
  })

  it('createOrUpdateWatchlistItem calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1 })
    await mod.createOrUpdateWatchlistItem({ tmdbId: '1', mediaType: 'movie', title: 'X' })
    expect(mockInvoke).toHaveBeenCalledWith('create_or_update_watchlist_item', expect.anything())
  })

  it('updateWatchlistItem calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1 })
    await mod.updateWatchlistItem(1, { tmdbId: '1', mediaType: 'movie', title: 'X' })
    expect(mockInvoke).toHaveBeenCalledWith('update_watchlist_item', expect.objectContaining({ id: 1 }))
  })

  it('deleteWatchlistItem calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.deleteWatchlistItem(1)
    expect(mockInvoke).toHaveBeenCalledWith('delete_watchlist_item', { id: 1 })
  })

  it('syncWatchlist calls invoke', async () => {
    mockInvoke.mockResolvedValueOnce({ synced: true })
    expect(await mod.syncWatchlist()).toEqual({ synced: true })
  })

  it('clearAllAppData clears localStorage then invokes', async () => {
    store['key'] = 'value'
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.clearAllAppData()
    expect(store).toEqual({})
    expect(mockInvoke).toHaveBeenCalledWith('clear_all_app_data', { confirmed: true })
  })
})

// ─── success path tests for throw-on-error wrappers ─────────────────────

describe('service wrappers: success paths', async () => {
  const mod = await import('./api')

  it('playMedia trims audio/subtitle and invokes', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.playMedia(1, true, '  jpn  ', '  eng  ', 120, 1024)
    expect(mockInvoke).toHaveBeenCalledWith('play_with_mpv', expect.objectContaining({
      mediaId: 1,
      resume: true,
      audioLanguage: 'jpn',
      subtitleLanguage: 'eng',
      durationSecondsOverride: 120,
      fileSizeBytesOverride: 1024,
    }))
  })

  it('playMedia nullifies zero/negative overrides', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.playMedia(1, false, null, null, 0, -1)
    expect(mockInvoke).toHaveBeenCalledWith('play_with_mpv', expect.objectContaining({
      durationSecondsOverride: null,
      fileSizeBytesOverride: null,
    }))
  })

  it('playMediaNative trims and invokes', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.playMediaNative(1, false, 'eng', null)
    expect(mockInvoke).toHaveBeenCalledWith('play_with_native_mpv', expect.objectContaining({
      audioLanguage: 'eng',
      subtitleLanguage: null,
    }))
  })

  it('fixMatch calls invoke with tmdbId', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)
    await mod.fixMatch(1, '12345', 'movie')
    expect(mockInvoke).toHaveBeenCalledWith('fix_match', expect.objectContaining({
      mediaId: 1,
      tmdbId: '12345',
      mediaType: 'movie',
    }))
  })

  it('getLibrary passes search param', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.getLibrary('tv', 'query')
    expect(mockInvoke).toHaveBeenCalledWith('get_library', { mediaType: 'tv', search: 'query' })
  })

  it('getLibrary passes null when search empty', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.getLibrary('movie')
    expect(mockInvoke).toHaveBeenCalledWith('get_library', { mediaType: 'movie', search: null })
  })

  it('wtLaunchMpv gets client ID first', async () => {
    mockInvoke.mockResolvedValueOnce('client-123') // wtGetClientId
    mockInvoke.mockResolvedValueOnce(42) // wtLaunchMpv
    const pid = await mod.wtLaunchMpv(1, 'ignored', 10)
    expect(pid).toBe(42)
    expect(mockInvoke).toHaveBeenCalledWith('wt_launch_mpv', expect.objectContaining({
      sessionId: 'client-123',
    }))
  })
})
