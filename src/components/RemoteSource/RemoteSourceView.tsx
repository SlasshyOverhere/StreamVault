import { useState, useEffect, useCallback, useRef, memo, Component, type ReactNode, type ErrorInfo } from 'react'
import { invoke } from '@tauri-apps/api/tauri'
import { listen } from '@tauri-apps/api/event'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useToast } from '@/components/ui/use-toast'
import { RemoteSearchBar } from './RemoteSearchBar'
import { RemoteSearchResults } from './RemoteSearchResults'
import { RemoteMediaDetail } from './RemoteMediaDetail'
import { RemoteQualitySelector } from './RemoteQualitySelector'
import { RemoteCacheStatusBar } from './RemoteCacheStatusBar'
import { RemoteCleanupDialog } from './RemoteCleanupDialog'
import { AddonSetupWizard } from './AddonSetupWizard'
import { Film, Play, X, ArrowLeft, Check, Clock } from 'lucide-react'
import { listStremioAddons, fetchStremioStreams, type StremioAddon } from '@/services/api'
import { toRemoteStreamData, type StremioRawStream } from './StremioStreamAdapter'
import type { TmdbSearchResult, GroupedStreams, RemoteStreamData, CacheStatus } from './remote.types'
import { getYear } from './remote.types'
import { getCachedImageUrl } from '@/services/api'

interface TmdbSearchResponse { results: TmdbSearchResult[]; total_results: number }

interface PlaybackEndedEvent {
  media_id: number
  completed: boolean
  final_position: number | null
  final_duration: number | null
  media_type: 'movie' | 'tv'
  tmdb_id: number
  season_number: number | null
  episode_number: number | null
  title: string
}

type PageState = 'library' | 'search' | 'detail' | 'episodes'

function getMediaIdentifier(item: TmdbSearchResult, season?: number, episode?: number): string {
  const base = `remote-${item.id}`
  if (item.media_type === 'tv' && season != null && episode != null) {
    return `${base}-S${season}E${episode}`
  }
  return base
}

interface RemoteBookmark {
  tmdb_id: string
  media_type: string   // 'movie' or 'tv'
  title: string
  year: number | null
  poster_path: string | null
  created_at: string
}

interface RemoteProgress {
  tmdb_id: string
  media_type: string
  season_number: number | null
  episode_number: number | null
  resume_position_seconds: number
  duration_seconds: number
  last_watched: string
}

interface TmdbEpisode {
  episode_number: number
  name: string
  overview: string
  still_path: string | null
  runtime: number | null
  season_number: number
}

function toSearchResult(item: RemoteBookmark): TmdbSearchResult {
  const tmdbId = parseInt(item.tmdb_id)
  const normalType: 'movie' | 'tv' = item.media_type === 'tv' ? 'tv' : 'movie'
  return {
    id: Number.isFinite(tmdbId) ? tmdbId : 0,
    title: normalType === 'movie' ? item.title : undefined,
    name: normalType === 'tv' ? item.title : undefined,
    media_type: normalType,
    poster_path: item.poster_path ?? undefined,
    release_date: item.year ? String(item.year) : undefined,
    first_air_date: item.year ? String(item.year) : undefined,
    vote_average: undefined,
  }
}

const LibraryPoster = memo(function LibraryPoster({ posterPath, alt }: { posterPath: string | null; alt: string }) {
  const [imgUrl, setImgUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      if (!posterPath) { if (!cancelled) setImgUrl(null); return }
      if (posterPath.startsWith('http://') || posterPath.startsWith('https://') || posterPath.startsWith('asset://')) {
        if (!cancelled) setImgUrl(posterPath)
        return
      }
      if (posterPath.startsWith('/')) {
        if (!cancelled) setImgUrl(`https://image.tmdb.org/t/p/w185${posterPath}`)
        return
      }
      let filename = posterPath
      if (filename.startsWith('image_cache/')) filename = filename.replace('image_cache/', '')
      try {
        const url = await getCachedImageUrl(filename)
        if (!cancelled) setImgUrl(url)
      } catch (e) {
        console.debug('[RemoteSourceView] getCachedImageUrl:', e)
        if (!cancelled) setImgUrl(null)
      }
    }
    load()
    return () => { cancelled = true }
  }, [posterPath])

  const url = imgUrl && !failed ? imgUrl : null
  return url ? (
    <img src={url} alt={alt} className="w-full h-full object-cover" loading="lazy" onError={() => setFailed(true)} />
  ) : (
    <div className="w-full h-full flex items-center justify-center bg-neutral-900">
      <Film className="size-5 text-neutral-700" />
    </div>
  )
})

interface ErrorBoundaryState { hasError: boolean; error: Error | null }

class RemoteSourceErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  state: ErrorBoundaryState = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[RemoteSourceView] Uncaught error:', error, info.componentStack)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="h-full flex items-center justify-center">
          <div className="text-center space-y-4 p-8">
            <div className="size-16 rounded-2xl bg-neutral-900 border border-neutral-800 flex items-center justify-center mx-auto">
              <Film className="size-7 text-red-500/60" />
            </div>
            <div>
              <p className="text-sm font-semibold text-neutral-300">Something went wrong</p>
              <p className="text-[13px] text-neutral-600 mt-1 max-w-xs">
                {this.state.error?.message || 'An unexpected error occurred in the remote library.'}
              </p>
              <button
                onClick={() => this.setState({ hasError: false, error: null })}
                className="mt-4 px-4 py-2 rounded-xl bg-neutral-900 border border-neutral-800 text-xs font-semibold text-neutral-400 hover:text-neutral-200 hover:border-neutral-700 transition-all"
              >
                Try again
              </button>
            </div>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

function RemoteSourceViewInner() {
  const { toast } = useToast()
  const [searchQuery, setSearchQuery] = useState('')
  const [searchResults, setSearchResults] = useState<TmdbSearchResult[]>([])
  const [isSearching, setIsSearching] = useState(false)
  const [remoteBookmarks, setRemoteBookmarks] = useState<RemoteBookmark[]>([])
  const [remoteProgress, setRemoteProgress] = useState<RemoteProgress[]>([])
  const [libraryLimit, setLibraryLimit] = useState(50)
  const [addonUrlConfigured, setAddonUrlConfigured] = useState<boolean | null>(null)
  const [activeSource, setActiveSource] = useState<{ name: string; url: string } | null>(null)
  const [addonVersion, setAddonVersion] = useState<string | null>(null)
  const [addonCrashed, setAddonCrashed] = useState(false)

  const [pageState, setPageState] = useState<PageState>('library')
  const [selectedItem, setSelectedItem] = useState<TmdbSearchResult | null>(null)
  const [selectedShow, setSelectedShow] = useState<RemoteBookmark | null>(null)
  const [showEpisodes, setShowEpisodes] = useState<TmdbEpisode[]>([])
  const [loadingEpisodes, setLoadingEpisodes] = useState(false)

  // Stream fetching
  const [fetching, setFetching] = useState(false)
  const [groupedStreams, setGroupedStreams] = useState<GroupedStreams[]>([])
  const [streamError, setStreamError] = useState<string | null>(null)
  const [qualityOpen, setQualityOpen] = useState(false)
  // ponytail: season stream cache, keyed by "imdbId:season"
  const seasonStreamsCache = useRef<Map<string, Map<number, GroupedStreams[]>>>(new Map())

  // Current episode context (for TV)
  const [currentSeason, setCurrentSeason] = useState<number>(1)
  const [currentEpisode, setCurrentEpisode] = useState<number>(1)
  const [currentEpisodeTitle, setCurrentEpisodeTitle] = useState('')

  // Resume dialog

  // Cache
  const [cacheStatus, setCacheStatus] = useState<CacheStatus | null>(null)
  const [showCleanup, setShowCleanup] = useState(false)
  const [lastPlayedTitle, setLastPlayedTitle] = useState('')
  const [lastCacheKey, setLastCacheKey] = useState('')


  const imdbIdRef = useRef<string>('')
  const detailReqId = useRef(0)
  const searchReqIdRef = useRef(0)

  // Next episode prompt
  const [nextEpisodePrompt, setNextEpisodePrompt] = useState<{ show: boolean; imdbId: string; season: number; episode: number; title: string }>({ show: false, imdbId: '', season: 0, episode: 0, title: '' })

  const HISTORY_KEY = 'remote-search-history'
  const MAX_HISTORY = 20

  const [searchHistory, setSearchHistory] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(HISTORY_KEY) || '[]')
    } catch (e) { console.debug('[RemoteSourceView] load search history:', e); return [] }
  })

  useEffect(() => {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(searchHistory))
  }, [searchHistory])

  const lastSavedRef = useRef<string>('')

  const addToHistory = useCallback((query: string) => {
    const q = query.trim()
    if (!q || q.length < 2 || q.toLowerCase() === lastSavedRef.current) return
    lastSavedRef.current = q.toLowerCase()
    setSearchHistory((prev) => {
      const filtered = prev.filter((s) => s.toLowerCase() !== q.toLowerCase())
      return [q, ...filtered].slice(0, MAX_HISTORY)
    })
  }, [])

  const clearHistory = useCallback(() => {
    setSearchHistory([])
  }, [])

  const removeFromHistory = useCallback((q: string) => {
    setSearchHistory((prev) => prev.filter((s) => s !== q))
  }, [])

  // Load remote library on mount
  const loadRemoteLibrary = useCallback(async () => {
    try {
      const [bookmarks, progress] = await Promise.all([
        invoke<RemoteBookmark[]>('remote_get_library'),
        invoke<RemoteProgress[]>('remote_get_all_progress'),
      ])
      setRemoteBookmarks(bookmarks)
      setRemoteProgress(progress)
    } catch (e) { console.error('[RemoteSourceView] loadRemoteLibrary:', e) }
  }, [])

  // Pre-fetch all streams for a season (tt{imdb}:{season}:full)
  const handleFetchSeasonStreams = useCallback(async (imdbId: string, season: number) => {
    const cacheKey = `${imdbId}:${season}`
    if (seasonStreamsCache.current.has(cacheKey)) return
    try {
      type SeasonEpisodeResponse = { episode: number; groupedStreams: GroupedStreams[] }
      const result = await invoke<SeasonEpisodeResponse[]>('remote_get_season_streams', { imdbId, season })
      const epMap = new Map<number, GroupedStreams[]>()
      for (const ep of result) {
        epMap.set(ep.episode, ep.groupedStreams)
      }
      seasonStreamsCache.current.set(cacheKey, epMap)
    } catch (e) {
      console.debug('[RemoteSourceView] season streams fetch failed:', e)
    }
  }, [])

  const loadShowEpisodes = useCallback(async (tmdbId: number) => {
    setLoadingEpisodes(true)
    try {
      // Fetch progress for this show from the progress table
      const showProgress = await invoke<RemoteProgress[]>('remote_get_show_progress', { tmdbId: String(tmdbId) })
      const progressMap = new Map<string, RemoteProgress>()
      for (const p of showProgress) {
        progressMap.set(`${p.season_number}x${p.episode_number}`, p)
      }

      const details = await invoke<any>('get_tv_details', { tvId: tmdbId })
      const tvSeasons = (details.seasons || []).filter((s: any) => s.season_number > 0)
      const allEpisodes: TmdbEpisode[] = []
      for (const season of tvSeasons) {
        try {
          const seasonData = await invoke<any>('get_tv_season_episodes', { tvId: tmdbId, seasonNumber: season.season_number })
          for (const ep of (seasonData.episodes || [])) {
            allEpisodes.push({
              episode_number: ep.episode_number,
              name: ep.name || '',
              overview: ep.overview ?? '',
              still_path: ep.still_path ?? null,
              runtime: ep.runtime ?? null,
              season_number: season.season_number,
            })
          }
        } catch (e) { console.debug(`[RemoteSourceView] Failed to fetch season ${season.season_number}:`, e) }
      }
      setShowEpisodes(allEpisodes)

      // Pre-fetch season streams in background for instant episode playback
      const imdbId = await invoke<string | null>('resolve_imdb_id', { tmdbId, mediaType: 'tv' }).catch(() => null)
      if (imdbId) {
        for (const season of tvSeasons) {
          handleFetchSeasonStreams(imdbId, season.season_number).catch(() => {})
        }
      }
    } catch (e) { console.error('[RemoteSourceView] loadShowEpisodes:', e) }
    finally { setLoadingEpisodes(false) }
  }, [handleFetchSeasonStreams])

  useEffect(() => {
    loadRemoteLibrary()
  }, [loadRemoteLibrary])

  // Check if addon URL is configured (sources or legacy URL)
  const checkAddonConfig = useCallback(async () => {
    try {
      const config = await invoke<any>('get_config')
      const hasSources = config?.addon_sources?.length > 0
      const hasLegacyUrl = !!config?.addon_url
      let hasStremio = false
      try {
        const stremioAddons = await invoke<any[]>('stremio_list_addons')
        hasStremio = stremioAddons.length > 0
      } catch { /* ignore */ }
      setAddonUrlConfigured(hasSources || hasLegacyUrl || hasStremio)
      if (hasSources) {
        const defaultSrc = config.addon_sources.find((s: any) => s.is_default)
        const src = defaultSrc || config.addon_sources[0]
        setActiveSource({ name: src.name, url: src.url })
        try {
          const ver = await invoke<string | null>('get_addon_version', { url: src.url })
          setAddonVersion(ver)
        } catch { setAddonVersion(null) }
      } else if (hasLegacyUrl) {
        setActiveSource({ name: 'Addon', url: config.addon_url })
        try {
          const ver = await invoke<string | null>('get_addon_version', { url: config.addon_url })
          setAddonVersion(ver)
        } catch { setAddonVersion(null) }
      } else {
        setActiveSource(null)
        setAddonVersion(null)
      }
    } catch {
      setAddonUrlConfigured(false)
      setAddonVersion(null)
    }
  }, [])

  useEffect(() => { checkAddonConfig() }, [checkAddonConfig])

  // Re-check when config changes (e.g. source removed in Settings)
  useEffect(() => {
    const handler = () => checkAddonConfig()
    window.addEventListener('config-saved', handler)
    return () => window.removeEventListener('config-saved', handler)
  }, [checkAddonConfig])

  // Periodically check addon health (version = alive proxy)
  useEffect(() => {
    if (!activeSource) return
    const interval = setInterval(() => {
      invoke<string | null>('get_addon_version', { url: activeSource.url })
        .then((ver) => {
          if (ver) {
            setAddonVersion(ver)
            setAddonCrashed(false)
          } else {
            setAddonVersion(null)
          }
        })
        .catch(() => {
          setAddonVersion(null)
        })
    }, addonVersion ? 30000 : 15000) // 30s when healthy, 15s when waiting for first contact
    return () => clearInterval(interval)
  }, [activeSource, addonVersion])

  // Listen for addon log events and crash notifications
  useEffect(() => {
    let unlistenLog: (() => void) | undefined
    let unlistenCrash: (() => void) | undefined
    const setup = async () => {
      try {
        unlistenLog = await listen<string>('addon-log', (_event) => {
          // If we're getting logs, addon is alive — fetch version if missing
          if (!addonVersion && activeSource) {
            invoke<string | null>('get_addon_version', { url: activeSource.url })
              .then(setAddonVersion)
              .catch(() => {})
          }
        })
        unlistenCrash = await listen<number>('addon-crashed', () => {
          setAddonCrashed(true)
          toast({ title: 'Addon crashed', description: 'The addon binary has crashed too many times. Please restart the app.', variant: 'destructive' })
        })
      } catch {}
    }
    setup()
    return () => { unlistenLog?.(); unlistenCrash?.() }
  }, [toast, addonVersion, activeSource])

  // Search (with race condition guard via searchReqIdRef)
  useEffect(() => {
    if (!searchQuery.trim()) { setSearchResults([]); return }
    const reqId = ++searchReqIdRef.current
    setIsSearching(true)
    setPageState('search')
    invoke<TmdbSearchResponse>('search_tmdb', { query: searchQuery })
      .then((res) => {
        if (reqId !== searchReqIdRef.current) return
        const results = res.results || []
        if (results.length > 0) addToHistory(searchQuery)
        setSearchResults(results)
      })
      .catch((e) => {
        if (reqId !== searchReqIdRef.current) return
        console.error('[RemoteSourceView] search_tmdb:', e)
        setSearchResults([])
      })
      .finally(() => {
        if (reqId !== searchReqIdRef.current) return
        setIsSearching(false)
      })
  }, [searchQuery, addToHistory])

  // Cache progress events
  useEffect(() => {
    const unsub = listen<CacheStatus>('remote-cache-progress', (event) => {
      setCacheStatus(event.payload)
    })
    return () => { unsub.then((fn) => fn()) }
  }, [])

  // Playback complete => cleanup dialog
  useEffect(() => {
    const unsub = listen<any>('remote-cache-complete', (event) => {
      const s = event.payload as CacheStatus
      setLastCacheKey(s.cacheKey)
      setLastPlayedTitle(s.cacheKey.replace(/_.*$/, ''))
      setShowCleanup(true)
    })
    return () => { unsub.then((fn) => fn()) }
  }, [])

  // Netflix-style: listen for mpv-playback-ended for next-episode flow
  useEffect(() => {
    const unsub = listen<PlaybackEndedEvent>('mpv-playback-ended', (event) => {
      const data = event.payload
      // Refresh library to reflect updated progress
      loadRemoteLibrary()
      // Also refresh episode list if viewing episodes
      if (selectedShow) loadShowEpisodes(parseInt(selectedShow.tmdb_id))
      if (data.completed && data.media_type === 'tv' && data.season_number != null && data.episode_number != null) {
        const nextEp = data.episode_number + 1
        setNextEpisodePrompt({
          show: true,
          imdbId: imdbIdRef.current,
          season: data.season_number,
          episode: nextEp,
          title: data.title,
        })
      }
    })
    return () => { unsub.then((fn) => fn()) }
  }, [loadRemoteLibrary, selectedShow, loadShowEpisodes])

  const handleDismissNextEpisode = useCallback(() => {
    setNextEpisodePrompt((prev) => ({ ...prev, show: false }))
  }, [])

  const handleSelectResult = useCallback(async (item: TmdbSearchResult) => {
    setSelectedItem(item)
    setPageState('detail')
  }, [])

  // Stream verification removed — causes rate limiting against addon servers.
  // Pixeldrain URLs are verified separately (direct CDN, safe to HEAD-check).
  const [pixeldrainStatus, setPixeldrainStatus] = useState<Record<string, boolean>>({})
  const [pixeldrainVerifying, setPixeldrainVerifying] = useState(false)

  const isPixeldrainUrl = (url: string) => /pixeldrain\.\w+/i.test(url)

  useEffect(() => {
    if (!qualityOpen || groupedStreams.length === 0) return
    const pdUrls: string[] = []
    for (const g of groupedStreams) {
      for (const s of g.streams) {
        if (isPixeldrainUrl(s.url) && !pixeldrainStatus[s.url]) {
          pdUrls.push(s.url)
        }
      }
    }
    if (pdUrls.length === 0) return
    let cancelled = false
    ;(async () => {
      setPixeldrainVerifying(true)
      try {
        const results = await invoke<Record<string, boolean>>('verify_stream_urls', { urls: pdUrls })
        if (!cancelled) setPixeldrainStatus(prev => ({ ...prev, ...results }))
      } catch { /* ignore */ }
      if (!cancelled) setPixeldrainVerifying(false)
    })()
    return () => { cancelled = true }
  }, [qualityOpen, groupedStreams])

  // Fetch streams from all installed Stremio addons for a given IMDB ID.
  // Returns them merged into GroupedStreams format.
  const fetchStremioAddonStreams = useCallback(async (imdbId: string, mediaType: 'movie' | 'series', season?: number, episode?: number): Promise<GroupedStreams[]> => {
    try {
      const addons = await listStremioAddons()
      if (addons.length === 0) return []

      const stremioType = mediaType === 'tv' ? 'series' : mediaType
      // Stremio stream IDs: movie = tt1234567, series = tt1234567:S1E2
      let stremioId = imdbId
      if (stremioType === 'series' && season != null && episode != null) {
        stremioId = `${imdbId}:S${season}E${episode}`
      }

      const results = await Promise.allSettled(
        addons.map(async (addon) => {
          const resp = await fetchStremioStreams(addon.id, stremioType, stremioId)
          const streams = Array.isArray(resp.streams) ? (resp.streams as StremioRawStream[]) : []
          return streams
            .map((s) => toRemoteStreamData(s, { addonName: addon.name }))
            .filter((s): s is RemoteStreamData => s !== null && s.url !== '')
        })
      )

      const allStremioStreams = results
        .filter((r): r is PromiseFulfilledResult<RemoteStreamData[]> => r.status === 'fulfilled')
        .flatMap((r) => r.value)

      if (allStremioStreams.length === 0) return []

      // Group by quality
      const grouped: GroupedStreams[] = []
      for (const stream of allStremioStreams) {
        const quality = stream.parsedQuality || 'auto'
        const existing = grouped.find((g) => g.quality === quality)
        if (existing) {
          existing.streams.push(stream)
        } else {
          grouped.push({ quality, streams: [stream] })
        }
      }
      return grouped
    } catch {
      return []
    }
  }, [])

  // Movie: fetch streams and open quality selector
  const handleFetchMovieStreams = useCallback(async (imdbId: string, forceRefresh = false) => {
    setFetching(true)
    setStreamError(null)
    setGroupedStreams([])
    setQualityOpen(true)
    setCurrentSeason(1)
    setCurrentEpisode(1)
    setCurrentEpisodeTitle('')
    imdbIdRef.current = imdbId
    try {
      const [addonStreams, stremioStreams] = await Promise.all([
        invoke<GroupedStreams[]>('remote_get_movie_streams', { imdbId, forceRefresh }).catch(() => []),
        fetchStremioAddonStreams(imdbId, 'movie'),
      ])
      // Merge: addon streams first, then Stremio streams
      const merged = [...addonStreams]
      for (const sg of stremioStreams) {
        const existing = merged.find((g) => g.quality === sg.quality)
        if (existing) {
          existing.streams.push(...sg.streams)
        } else {
          merged.push(sg)
        }
      }
      setGroupedStreams(merged)
      if (merged.length === 0) setStreamError('No streams found')
    } catch (e: any) {
      setStreamError(typeof e === 'string' ? e : 'Failed to load streams')
    }
    setFetching(false)
  }, [fetchStremioAddonStreams])

  // Series episode: fetch streams and open quality selector
  const handleFetchEpisodeStreams = useCallback(async (imdbId: string, season: number, episode: number, episodeTitle: string, forceRefresh = false) => {
    setFetching(true)
    setStreamError(null)
    setGroupedStreams([])
    setQualityOpen(true)
    setCurrentSeason(season)
    setCurrentEpisode(episode)
    setCurrentEpisodeTitle(episodeTitle)
    imdbIdRef.current = imdbId

    // Check season cache first
    const cacheKey = `${imdbId}:${season}`
    const cached = seasonStreamsCache.current.get(cacheKey)
    if (cached && !forceRefresh) {
      const epStreams = cached.get(episode) || []
      setGroupedStreams(epStreams)
      setFetching(false)
      return
    }

    try {
      const [addonStreams, stremioStreams] = await Promise.all([
        invoke<GroupedStreams[]>('remote_get_series_streams', { imdbId, season, episode, forceRefresh }).catch(() => []),
        fetchStremioAddonStreams(imdbId, 'series', season, episode),
      ])
      const merged = [...addonStreams]
      for (const sg of stremioStreams) {
        const existing = merged.find((g) => g.quality === sg.quality)
        if (existing) {
          existing.streams.push(...sg.streams)
        } else {
          merged.push(sg)
        }
      }
      setGroupedStreams(merged)
      if (merged.length === 0) setStreamError('No streams found')
    } catch (e: any) {
      setStreamError(typeof e === 'string' ? e : 'Failed to load streams')
    }
    setFetching(false)
  }, [fetchStremioAddonStreams])

  // Season pack: fetch all episode streams for a season and show in quality selector
  const handleFetchSeasonPack = useCallback(async (imdbId: string, season: number) => {
    setFetching(true)
    setStreamError(null)
    setGroupedStreams([])
    setQualityOpen(true)
    setCurrentSeason(season)
    imdbIdRef.current = imdbId
    try {
      type SeasonEpisodeResponse = { episode: number; groupedStreams: GroupedStreams[] }
      const result = await invoke<SeasonEpisodeResponse[]>('remote_get_season_streams', { imdbId, season })
      // Merge all episode streams into a single flat list grouped by quality
      const allStreams: GroupedStreams[] = []
      for (const ep of result) {
        for (const group of ep.groupedStreams) {
          const existing = allStreams.find(g => g.quality === group.quality)
          if (existing) {
            existing.streams.push(...group.streams)
          } else {
            allStreams.push({ ...group, streams: [...group.streams] })
          }
        }
      }
      setGroupedStreams(allStreams)
    } catch (e: any) {
      setStreamError(typeof e === 'string' ? e : 'Failed to load season pack')
    }
    setFetching(false)
  }, [])

  const handlePlayNextEpisode = useCallback(() => {
    const prompt = nextEpisodePrompt
    setNextEpisodePrompt({ show: false, imdbId: '', season: 0, episode: 0, title: '' })
    if (!prompt.imdbId) return
    handleFetchEpisodeStreams(prompt.imdbId, prompt.season, prompt.episode, '')
  }, [nextEpisodePrompt, handleFetchEpisodeStreams])

  const launchPlayback = useCallback(async (
    stream: RemoteStreamData,
    identifier: string,
    startPosition: number,
    item: TmdbSearchResult,
    season: number,
    episode: number,
    episodeTitle: string,
  ) => {
    try {
      const year = item.release_date || item.first_air_date || ''
      const showName = item.title || item.name || stream.name || 'Unknown'
      const displayTitle = item.media_type === 'tv' && episodeTitle
        ? `${showName} - S${String(season).padStart(2, '0')}E${String(episode).padStart(2, '0')} - ${episodeTitle}`
        : showName
      
      await invoke<any>('remote_play_with_mpv', {
        url: stream.url,
        title: displayTitle,
        videoSize: stream.videoSize,
        mediaIdentifier: identifier,
        qualityLabel: stream.parsedQuality,
        mediaType: item.media_type,
        tmdbId: item.id,
        seasonNumber: item.media_type === 'tv' ? season : null,
        episodeNumber: item.media_type === 'tv' ? episode : null,
        episodeTitle: item.media_type === 'tv' ? episodeTitle : null,
        posterPath: item.poster_path || null,
        stillPath: null,
        overview: item.overview || null,
        year: getYear(year) ? parseInt(getYear(year)) : null,
        startPosition,
      })

      // Refresh library to pick up the new record
      loadRemoteLibrary()

      toast({ title: 'Playback started', description: `${episodeTitle || item.title || item.name} -- ${stream.parsedQuality}` })
    } catch (e: any) {
      toast({
        title: 'Playback failed',
        description: typeof e === 'string' ? e : 'Failed to launch player',
        variant: 'destructive',
      })
    }
  }, [toast, loadRemoteLibrary])

  // User selects a quality => check resume, maybe show dialog, then play
  const isInLibrary = useCallback((item: TmdbSearchResult) => {
    const tmdbId = String(item.id)
    const mediaType = item.media_type === 'tv' ? 'tv' : 'movie'
    return remoteBookmarks.some(b => b.tmdb_id === tmdbId && b.media_type === mediaType)
  }, [remoteBookmarks])

  const handleQualitySelect = useCallback(async (stream: RemoteStreamData) => {
    if (!selectedItem) return
    const identifier = getMediaIdentifier(selectedItem, currentSeason, currentEpisode)
    setQualityOpen(false)
    // Auto-add to library if not already there
    if (!isInLibrary(selectedItem)) {
      try {
        await invoke('remote_add_to_library', {
          tmdbId: String(selectedItem.id),
          title: selectedItem.title || selectedItem.name || '',
          mediaType: selectedItem.media_type === 'tv' ? 'tv' : 'movie',
          year: getYear(selectedItem.release_date || selectedItem.first_air_date) ? parseInt(getYear(selectedItem.release_date || selectedItem.first_air_date)) : null,
          posterPath: selectedItem.poster_path || null,
        })
        await loadRemoteLibrary()
      } catch (e) { console.error('[RemoteSourceView] auto-add to library:', e) }
    }
    launchPlayback(stream, identifier, 0, selectedItem, currentSeason, currentEpisode, currentEpisodeTitle)
  }, [selectedItem, currentSeason, currentEpisode, currentEpisodeTitle, launchPlayback, isInLibrary, loadRemoteLibrary])

  const handleCleanup = useCallback(async () => {
    try {
      await invoke('remote_cleanup_cache', { cacheKey: lastCacheKey })
      setCacheStatus(null)
      toast({ title: 'Cache cleaned', description: 'Cached file has been removed.' })
    } catch (e: any) {
      toast({ title: 'Cleanup failed', description: typeof e === 'string' ? e : 'Failed to clean cache', variant: 'destructive' })
    }
  }, [lastCacheKey, toast])

  const handleKeep = useCallback(() => {
    setCacheStatus(null)
    toast({ title: 'Kept', description: 'File will be auto-cleaned based on cache settings.' })
  }, [toast])

  const handleRemoveFromLibrary = useCallback(async (item: RemoteBookmark, e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await invoke('remote_remove_from_library', { tmdbId: item.tmdb_id, mediaType: item.media_type })
      setRemoteBookmarks((prev) => prev.filter((b) => b.tmdb_id !== item.tmdb_id || b.media_type !== item.media_type))
      toast({ title: 'Removed', description: `${item.title} removed from library.` })
    } catch (err: any) {
      toast({ title: 'Remove failed', description: typeof err === 'string' ? err : 'Failed to remove', variant: 'destructive' })
    }
  }, [toast])

  const handleLibraryCardClick = useCallback(async (item: RemoteBookmark) => {
    const reqId = ++detailReqId.current
    const searchItem = toSearchResult(item)

    // Navigate immediately to detail view (same as search flow)
    setSelectedItem(searchItem)
    setPageState('detail')
    setShowCleanup(false)

    // Fetch fresh TMDB details in the background to enrich poster/backdrop/imdb_id
    try {
      if (item.media_type === 'tv') {
        const [details, extIds] = await Promise.all([
          invoke<any>('get_tv_details', { tvId: searchItem.id }),
          invoke<any>('get_imdb_details', { imdbId: null, tmdbId: searchItem.id, mediaType: 'tv' }).catch(() => null),
        ])
        if (reqId !== detailReqId.current) return
        if (details.poster_path) setSelectedItem((prev) => prev ? { ...prev, poster_path: details.poster_path } : prev)
        if (details.backdrop_path) setSelectedItem((prev) => prev ? { ...prev, backdrop_path: details.backdrop_path } as TmdbSearchResult : prev)
        if (extIds?.imdb_id) setSelectedItem((prev) => prev ? { ...prev, imdb_id: extIds.imdb_id } as TmdbSearchResult : prev)
      } else {
        const details = await invoke<any>('get_movie_details', { movieId: searchItem.id })
        if (reqId !== detailReqId.current) return
        if (details.poster_path) setSelectedItem((prev) => prev ? { ...prev, poster_path: details.poster_path } : prev)
        if (details.backdrop_path) setSelectedItem((prev) => prev ? { ...prev, backdrop_path: details.backdrop_path } as TmdbSearchResult : prev)
        if (details.imdb_id) setSelectedItem((prev) => prev ? { ...prev, imdb_id: details.imdb_id } as TmdbSearchResult : prev)
      }
    } catch (e) { console.warn('[RemoteSourceView] handleLibraryCardClick TMDB details:', e) }
  }, [])

  const handleBackFromEpisodes = useCallback(() => {
    setSelectedShow(null)
    setShowEpisodes([])
    setPageState('library')
  }, [])

  const handleEpisodeClick = useCallback(async (episode: TmdbEpisode) => {
    if (!selectedShow) return
    const showSearchItem = toSearchResult(selectedShow)
    // Set episode context
    setCurrentSeason(episode.season_number)
    setCurrentEpisode(episode.episode_number)
    setCurrentEpisodeTitle(episode.name)
    setSelectedItem(showSearchItem)
    setPageState('detail')
    // Fetch streams in background
    const imdbId = showSearchItem.imdb_id
    if (imdbId) {
      try {
        const streams = await invoke<GroupedStreams[]>('remote_get_series_streams', {
          imdbId,
          season: episode.season_number ?? 1,
          episode: episode.episode_number ?? 1,
          forceRefresh: false,
        })
        const flat = streams.flatMap((g) => g.streams)
        if (flat.length > 0) {
        }
      } catch (e) { console.warn('[RemoteSourceView] handleEpisodeClick fetch streams:', e) }
    }
  }, [selectedShow])

  const handleBackToLibrary = useCallback(() => {
    setSelectedItem(null)
    setSelectedShow(null)
    setShowEpisodes([])
    setGroupedStreams([])
    setStreamError(null)
    setQualityOpen(false)
    setPageState(searchQuery ? 'search' : 'library')
  }, [searchQuery])

  // Check if a TMDB item is already in the library

  // Save addon URL from setup wizard (uses new add_addon_source command)
  // (moved into AddonSetupWizard; this component no longer owns the setup flow)

  // Binary install handler (Go binary drag-and-drop)
  // (the dropzone now lives inside AddonSetupWizard; window-level file drops
  // are still received here because the Tauri event is global to the webview)
  const handleBinaryInstall = useCallback(async (filePath: string) => {
    setAddonCrashed(false)
    try {
      const result = await invoke<any>('install_addon_binary', { filePath, name: 'Custom Addon Binary' })
      setAddonUrlConfigured(true)
      loadRemoteLibrary()
      window.dispatchEvent(new CustomEvent('config-saved'))
      // Fetch version after install
      try {
        const ver = await invoke<string | null>('get_addon_version', { url: result.url })
        setAddonVersion(ver)
      } catch {}
      toast({ title: 'Binary installed', description: `Addon running at ${result.url}${addonVersion ? ` (v${addonVersion})` : ''}` })
    } catch (e: any) {
      const msg = String(e?.message || e)
      let description = msg
      if (msg.includes('--version')) description = 'The dropped file is not a valid addon binary. Please drop a vault-addon compatible .exe file.'
      else if (msg.includes('too large')) description = 'File is too large. Addon binaries should be under 50MB.'
      else if (msg.includes('.exe')) description = 'On Windows, the file must have an .exe extension.'
      else if (msg.includes('Failed to start')) description = 'The binary failed to start. Check that no other instance is running and the port is not in use.'
      toast({ title: 'Installation failed', description, variant: 'destructive' })
    }
  }, [loadRemoteLibrary, toast])

  // Listen for window-level file drops (Tauri tauri://file-drop event)
  useEffect(() => {
    let unlisten: (() => void) | undefined
    const setup = async () => {
      try {
        unlisten = await listen<{ paths: string[] }>('tauri://file-drop', (event) => {
          const paths = event.payload.paths
          if (paths.length > 0) {
            const filePath = paths[0]
            if (filePath.endsWith('.exe')) {
              handleBinaryInstall(filePath)
            } else {
              toast({ title: 'Invalid file', description: 'Please drop an .exe addon binary file.', variant: 'destructive' })
            }
          }
        })
      } catch {}
    }
    setup()
    return () => { unlisten?.() }
  }, [handleBinaryInstall, toast])

  // Show setup wizard if addon URL is not configured
  if (addonUrlConfigured === false) {
    return (
      <AddonSetupWizard
        crashed={addonCrashed}
        onInstalled={({ url: installedUrl, version }) => {
          setAddonUrlConfigured(true)
          loadRemoteLibrary()
          window.dispatchEvent(new CustomEvent('config-saved'))
          toast({
            title: 'Binary installed',
            description: `Addon running at ${installedUrl}${version ? ` (v${version})` : ''}`,
          })
        }}
        onSaved={() => {
          setAddonUrlConfigured(true)
          loadRemoteLibrary()
          window.dispatchEvent(new CustomEvent('config-saved'))
          toast({ title: 'Source added', description: 'You can now stream content from the External tab.' })
        }}
        onStremioInstalled={(addon) => {
          setAddonUrlConfigured(true)
          loadRemoteLibrary()
          toast({ title: 'Stremio addon added', description: `${addon.name} v${addon.version} — streams will appear when you pick content.` })
        }}
        onError={(message) => toast({ title: 'Could not add addon', description: message, variant: 'destructive' })}
        onRetryRestart={async () => {
          try {
            await invoke('restart_addon')
            setAddonCrashed(false)
            setAddonVersion(null)
            toast({ title: 'Restarting addon', description: 'Attempting to restart the addon binary...' })
          } catch (e: any) {
            toast({ title: 'Restart failed', description: String(e?.message || e), variant: 'destructive' })
            throw e
          }
        }}
      />
    )
  }

  // Loading state while checking config
  if (addonUrlConfigured === null) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="size-10 rounded-full border-2 border-neutral-800 border-t-amber-700/40 animate-spin" />
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col relative">
      {/* Ambient background glow */}
      <div className="pointer-events-none absolute -top-40 -right-40 size-[600px] rounded-full bg-amber-500/3 blur-[120px]" />
      <div className="pointer-events-none absolute -bottom-40 -left-40 size-[500px] rounded-full bg-sky-500/2 blur-[120px]" />

      {pageState === 'episodes' && selectedShow ? (
        /* ── Episode browser ── */
        <ScrollArea className="flex-1 px-8 pb-8 pt-10 relative z-10">
          <div className="max-w-4xl mx-auto">
            <button
              onClick={handleBackFromEpisodes}
              className="flex items-center gap-2 text-xs text-neutral-500 hover:text-neutral-200 transition-colors mb-6"
            >
              <ArrowLeft className="size-3.5" />
              Back to Library
            </button>
            <div className="flex gap-6 mb-8">
              <div className="w-32 shrink-0">
                <div className="aspect-[2/3] rounded-lg overflow-hidden bg-neutral-900 border border-neutral-800/60">
                  <LibraryPoster posterPath={selectedShow.poster_path} alt={selectedShow.title} />
                </div>
              </div>
              <div className="flex-1 min-w-0">
                <h2 className="text-2xl font-black text-white tracking-tight">{selectedShow.title}</h2>
                {selectedShow.year && <p className="text-sm text-neutral-500 mt-1">{selectedShow.year}</p>}
                <p className="text-xs text-neutral-600 mt-2">{loadingEpisodes ? 'Loading episodes...' : `${showEpisodes.length} episode${showEpisodes.length !== 1 ? 's' : ''}`}</p>
              </div>
            </div>
            {showEpisodes.length === 0 ? (
              <p className="text-sm text-neutral-600 text-center py-12">No episodes yet</p>
            ) : loadingEpisodes ? (
              <div className="flex items-center justify-center py-12">
                <div className="size-8 rounded-full border-2 border-neutral-800 border-t-amber-700/40 animate-spin" />
              </div>
            ) : (
              (() => {
                const grouped: Record<number, TmdbEpisode[]> = {}
                for (const ep of showEpisodes) {
                  const s = ep.season_number ?? 0
                  if (!grouped[s]) grouped[s] = []
                  grouped[s].push(ep)
                }
                // Load progress for this show
                const showTmdbId = selectedShow?.tmdb_id || ''
                const progressForShow = remoteProgress.filter(p => p.tmdb_id === showTmdbId && p.media_type === 'tv')
                const progressMap = new Map<string, RemoteProgress>()
                for (const p of progressForShow) {
                  progressMap.set(`${p.season_number}x${p.episode_number}`, p)
                }
                const seasons = Object.keys(grouped).map(Number).sort((a, b) => a - b)
                return seasons.map((seasonNum) => (
                  <div key={seasonNum} className="mb-6">
                    <h3 className="text-[11px] font-bold uppercase tracking-widest text-neutral-500 mb-3">
                      Season {seasonNum}
                    </h3>
                    <div className="space-y-2">
                      {grouped[seasonNum].map((ep) => {
                        const epProgress = progressMap.get(`${ep.season_number}x${ep.episode_number}`)
                        const durSec = epProgress?.duration_seconds ?? (ep.runtime ? ep.runtime * 60 : 0)
                        const posSec = epProgress?.resume_position_seconds ?? 0
                        const progress = durSec > 0 ? Math.min(100, (posSec / durSec) * 100) : 0
                        const isCompleted = progress >= 90
                        const inProgress = posSec > 0 && !isCompleted
                        const stillUrl = ep.still_path
                          ? ep.still_path.startsWith('http') ? ep.still_path
                          : ep.still_path.startsWith('/') ? `https://image.tmdb.org/t/p/w185${ep.still_path}`
                          : null : null
                        return (
                          <button
                            key={`${ep.season_number}x${ep.episode_number}`}
                            onClick={() => handleEpisodeClick(ep)}
                            className="w-full flex flex-col sm:flex-row gap-4 p-4 rounded-2xl bg-[#0A0A0A] border border-neutral-800/80 hover:bg-[#0D0D0D] hover:border-neutral-700/50 transition-all text-left group"
                          >
                            <div className="shrink-0 w-full sm:w-44 aspect-video rounded-xl overflow-hidden bg-neutral-900 border border-neutral-800">
                              {stillUrl ? (
                                <img src={stillUrl} alt={ep.name || ''} className="w-full h-full object-cover transition-transform duration-500 group-hover:scale-105" onError={(e) => { (e.target as HTMLImageElement).style.display = 'none' }} />
                              ) : (
                                <div className="w-full h-full flex items-center justify-center"><Film className="size-5 text-neutral-400" /></div>
                              )}
                            </div>
                            <div className="flex-1 min-w-0 flex flex-col justify-center gap-1.5">
                              <div className="flex items-center gap-2">
                                <span className="text-[11px] font-bold text-neutral-400 tabular-nums shrink-0">
                                  S{String(seasonNum).padStart(2, '0')} &middot; E{String(ep.episode_number ?? 0).padStart(2, '0')}
                                </span>
                                <h3 className={`text-sm font-semibold truncate ${isCompleted ? 'text-neutral-500' : 'text-neutral-200'}`}>
                                  {ep.name || `Episode ${ep.episode_number}`}
                                </h3>
                                {isCompleted && <Check className="size-3.5 text-emerald-500 shrink-0" />}
                                {inProgress && <Clock className="size-3.5 text-amber-500 shrink-0" />}
                              </div>
                              {ep.overview && (
                                <p className="text-xs text-neutral-300 leading-relaxed line-clamp-2">{ep.overview}</p>
                              )}
                              <div className="flex items-center gap-3 mt-0.5">
                                {durSec > 0 && (
                                  <span className="text-[10px] text-neutral-600">{Math.floor(durSec / 60)}m</span>
                                )}
                                {inProgress && (
                                  <div className="flex-1 max-w-32 h-1 rounded-full bg-neutral-800 overflow-hidden">
                                    <div className="h-full bg-amber-500/70 rounded-full" style={{ width: `${progress}%` }} />
                                  </div>
                                )}
                              </div>
                            </div>
                            <div className="shrink-0 flex items-center">
                              <div className="size-10 flex items-center justify-center rounded-xl bg-white/10 border border-white/15 text-neutral-200 hover:bg-white/20 hover:text-white hover:border-white/25 transition-all opacity-0 group-hover:opacity-100">
                                <Play className="size-4 fill-current" />
                              </div>
                            </div>
                          </button>
                        )
                      })}
                    </div>
                  </div>
                ))
              })()
            )}
          </div>
        </ScrollArea>
      ) : pageState === 'detail' ? (
        /* ── Detail view ── */
        <ScrollArea className="flex-1 px-8 pb-8 pt-10 relative z-10">
          <div className="max-w-4xl mx-auto">
            <RemoteMediaDetail
              item={selectedItem!}
              imdbId={(selectedItem as any).imdb_id}
              onBack={handleBackToLibrary}
              onFetchMovieStreams={handleFetchMovieStreams}
              onFetchEpisodeStreams={handleFetchEpisodeStreams}
              onFetchSeasonStreams={handleFetchSeasonStreams}
              onFetchSeasonPack={handleFetchSeasonPack}
              fetching={fetching}
            />
          </div>
        </ScrollArea>
      ) : (
        /* ── Library + Search view ── */
        <div className="flex-1 flex flex-col relative z-10">
          {/* Header */}
          <div className="shrink-0 pt-16 pb-6 px-8 text-center">
            <div className="max-w-lg mx-auto space-y-5">
              <div className="space-y-2">
                <div className="flex items-center justify-center gap-3 text-[10px] font-semibold text-neutral-600 uppercase tracking-[0.15em]">
                  <span className="h-px w-6 bg-neutral-800" />
                  <span>External Sources</span>
                  <span className="h-px w-6 bg-neutral-800" />
                </div>
                {addonCrashed && (
                  <div className="mx-auto max-w-md rounded-lg bg-red-950/40 border border-red-900/40 px-3 py-2 flex items-center justify-between gap-2">
                    <span className="text-xs text-red-400">Addon binary crashed too many times.</span>
                    <button
                      onClick={async () => {
                        try {
                          await invoke('restart_addon')
                          setAddonCrashed(false)
                          setAddonVersion(null)
                          toast({ title: 'Restarting addon', description: 'Attempting to restart the addon binary...' })
                        } catch (e) {
                          toast({ title: 'Restart failed', description: String(e), variant: 'destructive' })
                        }
                      }}
                      className="shrink-0 rounded-md bg-red-900/60 px-2.5 py-1 text-xs text-red-300 hover:bg-red-800/60 transition-colors"
                    >
                      Retry
                    </button>
                  </div>
                )}
                {activeSource && (
                  <div className="flex items-center justify-center gap-2">
                    <div className={`size-1.5 rounded-full ${addonCrashed ? 'bg-red-500' : 'bg-emerald-500 animate-pulse'}`} />
                    <span className="text-xs text-neutral-400">{activeSource.name}</span>
                    {addonVersion && <span className="text-[10px] text-neutral-600">v{addonVersion}</span>}
                    <span className="text-[10px] text-neutral-600 truncate max-w-[200px]">{activeSource.url}</span>
                  </div>
                )}
                <h1 className="text-3xl font-black tracking-tight text-white leading-none">Stream fuckin anything.</h1>
                <p className="text-[10px] text-neutral-700 leading-relaxed max-w-md mx-auto">
                  All media sources are third-party. We do not host, store, or control any content.
                </p>
              </div>
              <RemoteSearchBar value={searchQuery} onChange={setSearchQuery} />
              {!searchQuery && searchHistory.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {searchHistory.slice(0, 10).map((q) => (
                    <span
                      key={q}
                      className="group/chip inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-neutral-900/80 border border-neutral-800/60 text-[11px] text-neutral-500"
                    >
                      <button onClick={() => setSearchQuery(q)} className="hover:text-neutral-200 transition-colors">
                        {q}
                      </button>
                      <button
                        onClick={() => removeFromHistory(q)}
                        className="text-neutral-700 hover:text-red-400 transition-colors leading-none"
                      >
                        ×
                      </button>
                    </span>
                  ))}
                  <button
                    onClick={clearHistory}
                    className="px-2 py-0.5 rounded-md text-[11px] text-neutral-600 hover:text-red-400 transition-colors"
                  >
                    Clear
                  </button>
                </div>
              )}
            </div>
          </div>

          {/* Library cards - shown when no search query */}
          {!searchQuery && remoteBookmarks.length > 0 && (
            <div className="px-8 pb-4">
              <h2 className="text-[11px] font-bold uppercase tracking-widest text-neutral-600 mb-4">My Library</h2>
              <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8 gap-2.5">
                {remoteBookmarks.slice(0, libraryLimit).map((item) => {
                  // Find latest in-progress episode/movie for the resume bar
                  const itemProgress = remoteProgress.filter(p =>
                    p.tmdb_id === item.tmdb_id &&
                    (item.media_type === 'tv' ? p.media_type === 'tv' : p.media_type === 'movie') &&
                    p.duration_seconds > 0 && p.resume_position_seconds > 0
                  ).sort((a, b) => new Date(b.last_watched).getTime() - new Date(a.last_watched).getTime())[0]
                  const resumePercent = itemProgress ? Math.min(100, (itemProgress.resume_position_seconds / itemProgress.duration_seconds) * 100) : 0
                  return (
                  <button
                    key={`${item.tmdb_id}-${item.media_type}`}
                    onClick={() => handleLibraryCardClick(item)}
                    className="group text-left focus:outline-none"
                  >
                    <div className="aspect-[2/3] rounded-lg overflow-hidden bg-neutral-900 border border-neutral-800/60 group-hover:border-amber-700/40 transition-all duration-300 relative">
                      <LibraryPoster posterPath={item.poster_path} alt={item.title} />
                      {/* Media type badge */}
                      <div className="absolute top-1 left-1 px-1.5 py-0.5 rounded text-[8px] font-bold uppercase tracking-wider bg-black/60 text-neutral-400">
                        {item.media_type === 'movie' ? 'Movie' : 'Series'}
                      </div>
                      {/* Resume progress bar */}
                      {resumePercent > 0 && (
                        <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-neutral-800">
                          <div
                            className="h-full bg-amber-500/70 transition-all"
                            style={{ width: `${resumePercent}%` }}
                          />
                        </div>
                      )}
                      {/* Delete button on hover */}
                      <button
                        onClick={(e) => handleRemoveFromLibrary(item, e)}
                        className="absolute top-1 right-1 size-5 rounded-full bg-black/70 text-neutral-400 hover:text-red-400 hover:bg-black/90 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all z-10"
                        title="Remove from library"
                      >
                        <X className="size-3" />
                      </button>
                      {/* Play/Browse overlay on hover */}
                      <div className="absolute inset-0 bg-black/0 group-hover:bg-black/50 transition-all duration-300 flex items-center justify-center">
                        <div className="size-8 rounded-full bg-amber-600/80 text-white flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all duration-300 translate-y-1 group-hover:translate-y-0">
                          {item.media_type === 'tv' ? (
                            <Film className="size-3.5" />
                          ) : (
                            <Play className="size-3.5 fill-white ml-0.5" />
                          )}
                        </div>
                      </div>
                    </div>
                    <p className="mt-1 text-[11px] font-semibold text-neutral-300 truncate leading-tight">
                      {item.title}
                    </p>
                    {item.year && (
                      <p className="text-[10px] text-neutral-600">{item.year}</p>
                    )}
                  </button>
                )})}
              </div>
              {remoteBookmarks.length > libraryLimit && (
                <div className="flex justify-center mt-4">
                  <button
                    onClick={() => setLibraryLimit((prev) => prev + 50)}
                    className="px-4 py-2 rounded-xl bg-neutral-900 border border-neutral-800 text-xs font-semibold text-neutral-400 hover:text-neutral-200 hover:border-neutral-700 transition-all"
                  >
                    Load More ({remoteBookmarks.length - libraryLimit} remaining)
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Empty library */}
          {!searchQuery && remoteBookmarks.length === 0 && (
            <div className="flex-1 flex items-center justify-center px-8">
              <div className="text-center space-y-4">
                <div className="size-16 rounded-2xl bg-neutral-900 border border-neutral-800 flex items-center justify-center mx-auto">
                  <Film className="size-7 text-neutral-600" />
                </div>
                <div>
                  <p className="text-sm font-semibold text-neutral-300">No content yet</p>
                  <p className="text-[13px] text-neutral-600 mt-1 max-w-xs">
                    Search for a movie or TV show above to start streaming
                  </p>
                </div>
              </div>
            </div>
          )}

          {/* Search results */}
          {searchQuery && (
            <ScrollArea className="flex-1 min-h-0 px-8 pb-8">
              <div className="max-w-lg mx-auto">
                {/* Merge library items that match search into results */}
                <RemoteSearchResults results={searchResults} isLoading={isSearching} onSelect={handleSelectResult} />
              </div>
            </ScrollArea>
          )}
        </div>
      )}

      <RemoteQualitySelector
        open={qualityOpen}
        onOpenChange={setQualityOpen}
        title={selectedItem?.title || selectedItem?.name || 'Unknown'}
        groupedStreams={groupedStreams}
        onSelect={handleQualitySelect}
        onOpenUrl={(url) => window.open(url, '_blank')}
        loading={fetching}
        error={streamError}
        verifying={pixeldrainVerifying}
        streamStatus={pixeldrainStatus}
        verifyingUrls={new Set(Object.keys(pixeldrainStatus))}
        addonContext={imdbIdRef.current && currentSeason ? { imdbId: imdbIdRef.current, season: currentSeason } : null}
      />


      {/* Next Episode Prompt */}
      {nextEpisodePrompt.show && selectedItem && (
        <div className="fixed bottom-8 right-8 z-50 bg-[#0A0A0A] border border-neutral-800 rounded-2xl p-5 shadow-2xl max-w-sm">
          <p className="text-[11px] font-bold uppercase tracking-wider text-neutral-500 mb-1">Up Next</p>
          <p className="text-sm font-semibold text-neutral-200 mb-3">
            {selectedItem.title || selectedItem.name}
            {' '}
            <span className="text-amber-400">S{nextEpisodePrompt.season}E{nextEpisodePrompt.episode}</span>
          </p>
          <div className="flex gap-2">
            <button
              onClick={handlePlayNextEpisode}
              className="px-4 py-2 rounded-xl bg-amber-600/15 text-amber-400 border border-amber-700/30 text-xs font-semibold uppercase tracking-wider hover:bg-amber-600/25 transition-all"
            >
              Play Next Episode
            </button>
            <button
              onClick={handleDismissNextEpisode}
              className="px-4 py-2 rounded-xl bg-[#0D0D0D] text-neutral-500 border border-neutral-800 text-xs font-semibold uppercase tracking-wider hover:bg-neutral-900 hover:text-neutral-300 transition-all"
            >
              Dismiss
            </button>
          </div>
        </div>
      )}

      <RemoteCacheStatusBar status={cacheStatus} />
      <RemoteCleanupDialog open={showCleanup} onOpenChange={setShowCleanup} title={lastPlayedTitle} onCleanup={handleCleanup} onKeep={handleKeep} />
    </div>
  )
}

export default function RemoteSourceViewExport() {
  return (
    <RemoteSourceErrorBoundary>
      <RemoteSourceViewInner />
    </RemoteSourceErrorBoundary>
  )
}

export { RemoteSourceViewExport as RemoteSourceView }
