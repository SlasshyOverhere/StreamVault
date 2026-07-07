import { useState, useEffect, useRef, lazy, Suspense, useMemo, useCallback, startTransition } from 'react'
import { listen, emit, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/tauri'
import { appWindow } from '@tauri-apps/api/window'
import {
  Sidebar,
  MovieCard,
  ContinueCard,
  ResumeDialog,
  PlayConfirmDialog,
  DeleteEpisodesModal,

  MarkCompleteDialog,
  WatchTogetherBanner,
  LoginScreen,
  ReauthBanner,
  ContentDetailsModal,
  ZipPlaybackLoadingOverlay,
  NotificationCenter,
  RemindersView,
  DownloadsView,
  SmartCollections,
  Clock,
} from '@/components'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Toaster } from '@/components/ui/toaster'
import {
  getLibraryFiltered,
  searchAllLibraries,
  getLibraryStats,
  getWatchHistory,
  getWatchHistoryEvents,
  getRecentlyAdded,
  deleteMediaFiles,
  MediaItem,
  WatchRoom,
  playMedia,
  getResumeInfo,
  getMediaInfo,
  ResumeInfo,
  getCachedImageUrl,
  getTabVisibility,
  setTabVisibility,
  TabVisibility,
  markAsComplete,
  clearProgress,
  isBetaEnabled,
  setBetaEnabled,
  isTmdbKeyNoticeDismissed,
  setTmdbKeyNoticeDismissed,
  checkForUpdates,
  downloadUpdate,
  installUpdate,
  syncWatchHistory,
  UpdateInfo,
  MpvAudioTracksDetectedPayload,
  MpvSubtitleTracksDetectedPayload,
  mergeCachedSeriesAudioTracks,
  mergeCachedSeriesSubtitleTracks,
  resolveSeriesAudioPreferenceForPlayback,
  resolveSeriesSubtitlePreferenceForPlayback,
  DownloadJob,
  getDownloadJobs,
  startMediaDownload,
  cancelDownloadJob,
  deleteDownloadJob,
  clearDownloadHistory,
  openDownloadJobTarget,
  getAnalyticsData,
  AnalyticsData,
  playMediaNative,
  getConfig,
  consumePendingSubtitlePath,
} from '@/services/api'
import {
  Search, Loader2, Film, Tv,
  ChevronRight, LayoutGrid, List,
  TrendingUp, Sparkles, X, Cloud, RefreshCw, Minus, Download, Bell,
  Maximize2, Minimize2, Archive, AlertCircle, FolderOpen
} from 'lucide-react'
import { useToast } from '@/components/ui/use-toast'
import { motion, AnimatePresence } from 'framer-motion'
import { useAuth } from '@/hooks/useAuth'
import { useAuthGuard, AuthCancelledError } from '@/hooks/useAuthGuard'
import { sortMediaItems } from '@/utils/sorting'
import { sortPinnedFirst } from '@/utils/pins'
import {
  getMediaProgressPercent,
  isProgressPastAutoCompleteThreshold,
  shouldPromptToMarkComplete,
} from '@/utils/playbackProgress'
import {
  buildZipPlaybackLoadingState,
  type ZipPlaybackLoadingState,
  waitForMinimumZipOverlayVisibility,
  waitForMpvPlaybackStart,
  waitForZipLoadingOverlayPaint,
} from '@/utils/zipPlayback'
import slasshyvaultIcon from '@/assets/slasshyvault-icon-ui.png'
import { CommandPalette } from '@/components/CommandPalette'
import { TmdbKeyNoticeBanner } from '@/components/TmdbKeyNoticeBanner'
const FullHistoryView = lazy(() => import('@/components/FullHistoryView').then(m => ({ default: m.FullHistoryView })))
const DirectLinksView = lazy(() => import('@/components/DirectLinksView').then(m => ({ default: m.default })))
const RemoteSourceView = lazy(() => import('@/components/RemoteSource/RemoteSourceView').then(m => ({ default: m.RemoteSourceView })))
const CalendarView = lazy(() => import('@/components/CalendarView').then(m => ({ default: m.CalendarView })))
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { OptimizationContext } from '@/lib/OptimizationContext'
import { useAutoOptimize, saveCachedLibrarySize } from '@/lib/optimization'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

// Lazy load heavy components
const loadSettingsModal = () => import('@/components/SettingsModal')
const loadEpisodeBrowser = () => import('@/components/EpisodeBrowser')
const loadWatchTogetherModal = () => import('@/components/WatchTogether/WatchTogetherModal')
const loadFixMatchModal = () => import('@/components/FixMatchModal')
const loadSyncValidatorModal = () => import('@/components/SyncValidatorModal')

const SettingsModal = lazy(() => loadSettingsModal().then(module => ({ default: module.SettingsModal })))
const EpisodeBrowser = lazy(() => loadEpisodeBrowser().then(module => ({ default: module.EpisodeBrowser })))
const WatchTogetherModal = lazy(() => loadWatchTogetherModal().then(module => ({ default: module.WatchTogetherModal })))
const FixMatchModal = lazy(() => loadFixMatchModal().then(module => ({ default: module.FixMatchModal })))
const SyncValidatorModal = lazy(() => loadSyncValidatorModal().then(module => ({ default: module.SyncValidatorModal })))
const DuplicateDetector = lazy(() => import('@/components/DuplicateDetector').then(module => ({ default: module.DuplicateDetector })))
const DeveloperConsole = lazy(() => import('@/components/DeveloperConsole').then(m => ({ default: m.DeveloperConsole })))



interface ScanProgressPayload {
  title: string
  media_type: string
  current: number
  total: number
}

interface ScanCompletePayload {
  movies_count: number
  tv_count: number
}

interface MpvPlaybackEndedPayload {
  media_id: number
  title: string
  episode_title?: string
  season_number?: number
  episode_number?: number
  media_type?: string
  final_position?: number
  final_duration?: number
  completed: boolean
}

interface ReminderFiredPayload {
  movie_id: number
  title: string
}

interface WatchlistReminderFiredPayload {
  id: number
  title: string
  tmdb_id: string
  media_type: string
  notification_mode: string
}

const AUTO_MARK_WATCHED_THRESHOLD_PERCENT = 93

interface ZipProcessingStatusPayload {
  phase: 'detected' | 'complete' | 'error'
  archiveCount: number
  archiveName?: string | null
  episodesIndexed?: number | null
  message: string
}

interface ZipProcessingPopupState {
  phase: 'detected' | 'complete' | 'error'
  archiveCount: number
  archiveName?: string | null
  episodesIndexed?: number | null
  message: string
}

type NotificationCategory = 'movie_add' | 'show_add' | 'reminder' | 'other'
type NotificationFilter = 'all' | NotificationCategory

interface AppNotificationItem {
  id: string
  category: NotificationCategory
  title: string
  message: string
  createdAt: string
  read: boolean
}

const NOTIFICATION_CENTER_STORAGE_KEY = 'slasshyvault.notification-center.v1'
const MAX_NOTIFICATION_CENTER_ITEMS = 200
const TV_EPISODE_NOTIFICATION_PATTERN = /\bS\d{1,2}E\d{1,3}\b/i

const loadStoredNotifications = (): AppNotificationItem[] => {
  try {
    const raw = localStorage.getItem(NOTIFICATION_CENTER_STORAGE_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []

    return parsed.filter((item): item is AppNotificationItem =>
      item
      && typeof item.id === 'string'
      && typeof item.category === 'string'
      && typeof item.title === 'string'
      && typeof item.message === 'string'
      && typeof item.createdAt === 'string'
      && typeof item.read === 'boolean'
    )
  } catch {
    return []
  }
}

const classifyNotificationCategory = (title: string, message: string): NotificationCategory => {
  if (title.toLowerCase().includes('reminder')) return 'reminder'
  if (message.includes('added to your library')) {
    return TV_EPISODE_NOTIFICATION_PATTERN.test(message) ? 'show_add' : 'movie_add'
  }
  return 'other'
}

type ViewMode = 'grid' | 'list'
type SortOption = 'title' | 'year' | 'recent' | 'progress'
type MediaSubTab = 'movies' | 'tv' | 'collections'
const LARGE_LIBRARY_THRESHOLD = 120
const CLOUD_INITIAL_RENDER_COUNT = 48
const CLOUD_CHUNK_RENDER_COUNT = 96
const VIEW_MODE_STORAGE_KEY = 'slasshyvault.view_mode'

const LoadingFallback = () => (
  <div className="flex h-full w-full items-center justify-center min-h-[50vh]">
    <Loader2 className="size-8 animate-spin text-muted-foreground" />
  </div>
)

const formatTime = (seconds: number): string => {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  return `${m}:${String(s).padStart(2, '0')}`
}

const formatUpdateError = (error: unknown) => {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  if (error && typeof error === 'object') {
    const record = error as Record<string, unknown>
    return typeof record.message === 'string' ? record.message : typeof record.error === 'string' ? record.error : JSON.stringify(error)
  }
  return 'Unknown update error.'
}

/** Get display title for a MediaItem — prefers episode_title for episodes */
function displayTitleFor(item: MediaItem): string {
  return item.episode_title || item.title
}

function App() {
  // Migrate old localStorage keys
  useEffect(() => {
    if (localStorage.getItem('slasshyvault_migration_done')) return;

    const keyMap: Record<string, string> = {
      'streamvault.notification-center.v1': 'slasshyvault.notification-center.v1',
      'streamvault.view_mode': 'slasshyvault.view_mode',
      'streamvault_streaming_progress': 'slasshyvault_streaming_progress',
      'streamvault_profile_cache': 'slasshyvault_profile_cache',
      'streamvault_dev_settings': 'slasshyvault_dev_settings',
      'streamvault_detected_audio_tracks_v2': 'slasshyvault_detected_audio_tracks_v2',
      'streamvault_detected_subtitle_tracks_v1': 'slasshyvault_detected_subtitle_tracks_v1',
      'streamvault_onboarding_completed': 'slasshyvault_onboarding_completed',
      'streamvault_tab_visibility': 'slasshyvault_tab_visibility',
      'streamvault_beta_features': 'slasshyvault_beta_features',

    };

    for (const [oldKey, newKey] of Object.entries(keyMap)) {
      const value = localStorage.getItem(oldKey);
      if (value !== null) {
        localStorage.setItem(newKey, value);
        localStorage.removeItem(oldKey);
      }
    }

    localStorage.setItem('slasshyvault_migration_done', 'true');
  }, [])

  // Navigate to Direct Links tab when season pack is indexed
  useEffect(() => {
    const handler = () => setView('directlinks')
    window.addEventListener('navigate-to-ddl', handler)
    return () => window.removeEventListener('navigate-to-ddl', handler)
  }, [])

  // Search and View state
  const searchInputRef = useRef<HTMLInputElement>(null)
  const [view, setView] = useState<string>('home')
  const [items, setItems] = useState<MediaItem[]>([])
  const [downloadJobs, setDownloadJobs] = useState<DownloadJob[]>([])
  const downloadJobCount = useMemo(() => downloadJobs.filter((j) => j.status !== 'completed' && j.status !== 'failed' && j.status !== 'cancelled').length, [downloadJobs])
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedShow, setSelectedShow] = useState<MediaItem | null>(null)
  const [isMaximized, setIsMaximized] = useState(false)
  const [analyticsData, setAnalyticsData] = useState<AnalyticsData | null>(null)

  // Sub-tabs for Cloud view
  const [cloudSubTab, setCloudSubTab] = useState<MediaSubTab>('movies')

  // View mode and sort
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    try {
      const saved = localStorage.getItem(VIEW_MODE_STORAGE_KEY)
      return saved === 'list' || saved === 'grid' ? saved : 'grid'
    } catch {
      return 'grid'
    }
  })
  const [sortBy] = useState<SortOption>('title')
  const [isNativePlaying, setIsNativePlaying] = useState(false)

  // Native player state
  const [nativePos, setNativePos] = useState(0)
  const [nativeDuration, setNativeDuration] = useState(0)
  const [nativePaused, setNativePaused] = useState(false)
  const [nativeVolume, setNativeVolume] = useState(100)
  const [nativeMuted, setNativeMuted] = useState(false)
  const [nativeAudioTracks, setNativeAudioTracks] = useState<Array<{id: number, lang: string}>>([])
  const [nativeSubTracks, setNativeSubTracks] = useState<Array<{id: number, lang: string}>>([])
  const [nativeAid, setNativeAid] = useState<number | null>(null)
  const [nativeSid, setNativeSid] = useState<number | null>(null)
  const [nativeSubScale, setNativeSubScale] = useState(1.0)
  const [controlsVisible, setControlsVisible] = useState(true)
  const controlsTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const controlsEl = useRef<HTMLDivElement>(null)

  useEffect(() => {
    document.body.classList.toggle('native-player-active', isNativePlaying)
    return () => document.body.classList.remove('native-player-active')
  }, [isNativePlaying])

  // Auto-hide controls during playback
  useEffect(() => {
    if (!isNativePlaying) {
      setControlsVisible(true)
      return
    }
    const show = () => {
      setControlsVisible(true)
      if (controlsTimer.current) clearTimeout(controlsTimer.current)
      controlsTimer.current = setTimeout(() => {
        if (!nativePaused) setControlsVisible(false)
      }, 3000)
    }
    show()
    window.addEventListener('mousemove', show)
    window.addEventListener('keydown', show)
    return () => {
      window.removeEventListener('mousemove', show)
      window.removeEventListener('keydown', show)
      if (controlsTimer.current) clearTimeout(controlsTimer.current)
    }
  }, [isNativePlaying, nativePaused])

  // Keyboard controls for native player
  useEffect(() => {
    if (!isNativePlaying) return
    const handler = (e: KeyboardEvent) => {
      const key = e.key
      const tag = (e.target as HTMLElement)?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

      switch (key) {
        case 'Escape':
        case 'q':
        case 'Q':
          invoke('native_mpv_stop').catch(() => {})
          setIsNativePlaying(false)
          break
        case ' ':
          e.preventDefault()
          invoke('native_mpv_pause', { paused: !nativePaused })
          break
        case 'ArrowLeft':
          invoke('native_mpv_seek', { position: Math.max(0, nativePos - 5) })
          break
        case 'ArrowRight':
          invoke('native_mpv_seek', { position: Math.min(nativeDuration, nativePos + 5) })
          break
        case 'ArrowUp':
          invoke('native_mpv_set_volume', { volume: Math.min(100, nativeVolume + 5) })
          break
        case 'ArrowDown':
          invoke('native_mpv_set_volume', { volume: Math.max(0, nativeVolume - 5) })
          break
        case 'f':
        case 'F':
          appWindow.toggleMaximize()
          break
        case '=':
        case '+':
          setNativeSubScale(s => {
            const v = Math.min(3, s + 0.1)
            invoke('native_mpv_set_property', { name: 'sub-scale', value: v })
            return v
          })
          break
        case '-':
          setNativeSubScale(s => {
            const v = Math.max(0.3, s - 0.1)
            invoke('native_mpv_set_property', { name: 'sub-scale', value: v })
            return v
          })
          break
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [isNativePlaying, nativePaused, nativePos, nativeDuration, nativeVolume])

  // Memoized sorted items to prevent re-sorting on every render
  const sortedItems = useMemo(() => {
    return sortMediaItems(items, sortBy)
  }, [items, sortBy])

  const refreshWindowState = useCallback(async () => {
    setIsMaximized(await appWindow.isMaximized())
  }, [])

  // Incremental rendering for very large cloud libraries to avoid view-switch stutter
  const [visibleCloudItemsCount, setVisibleCloudItemsCount] = useState(CLOUD_INITIAL_RENDER_COUNT)
  const cloudLoadMoreRef = useRef<HTMLDivElement | null>(null)
  const isChunkedCloudRender = view === 'cloud' && !searchQuery.trim() && sortedItems.length > LARGE_LIBRARY_THRESHOLD
  const cloudItemsToRender = useMemo(() => {
    if (!isChunkedCloudRender) return sortedItems
    return sortedItems.slice(0, visibleCloudItemsCount)
  }, [sortedItems, isChunkedCloudRender, visibleCloudItemsCount])
  useEffect(() => {
    let unlisten: UnlistenFn | null = null
    const setup = async () => {
      await refreshWindowState()
      unlisten = await appWindow.onResized(async () => {
        await refreshWindowState()
      })
    }
    setup()
    return () => {
      unlisten?.()
    }
  }, [refreshWindowState])

  useEffect(() => {
    document.body.classList.toggle('app-window-maximized', isMaximized)

    return () => {
      document.body.classList.remove('app-window-maximized')
    }
  }, [isMaximized])

  useEffect(() => {
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode)
    } catch {
      console.warn('[App] Failed to save view mode')
    }
  }, [viewMode])

  useEffect(() => {
    return () => {
      if (zipProcessingPopupTimeoutRef.current) {
        window.clearTimeout(zipProcessingPopupTimeoutRef.current)
      }
    }
  }, [])

  // Home search state
  const [homeSearchQuery, setHomeSearchQuery] = useState('')
  const [homeSearchResults, setHomeSearchResults] = useState<MediaItem[]>([])
  const [isHomeSearching, setIsHomeSearching] = useState(false)

  // Continue watching
  const [recentlyAdded, setRecentlyAdded] = useState<MediaItem[]>([])
  const [continueWatching, setContinueWatching] = useState<MediaItem[]>([])

  // Library stats
  const [libraryStats, setLibraryStats] = useState({ movies: 0, shows: 0, episodes: 0 })

  // Scanning state
  const [isScanning, setIsScanning] = useState(false)
  const [scanProgress, setScanProgress] = useState<{ current: number; total: number; title: string } | null>(null)

  // Cloud indexing state
  const [isCloudIndexing, setIsCloudIndexing] = useState(false)
  const [cloudIndexingStatus, setCloudIndexingStatus] = useState<string>('')
  const [cloudIndexingProgress, setCloudIndexingProgress] = useState<{
    currentFolder: number
    totalFolders: number
    currentFolderName: string
    filesFound: number
    moviesFound: number
    tvFound: number
  } | null>(null)
  const [zipPlaybackLoading, setZipPlaybackLoading] = useState<ZipPlaybackLoadingState | null>(null)
  const [zipProcessingPopup, setZipProcessingPopup] = useState<ZipProcessingPopupState | null>(null)
  const zipProcessingPopupTimeoutRef = useRef<number | null>(null)

  // Modals
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [showSyncValidator, setShowSyncValidator] = useState(false)
  const [showDuplicateDetector, setShowDuplicateDetector] = useState(false)
  const [settingsInitialTab, setSettingsInitialTab] = useState<'general' | 'beta' | 'updates' | 'cloud' | 'api' | 'danger' | 'dev'>('general')
  const [fixMatchOpen, setFixMatchOpen] = useState(false)
  const [itemToFix, setItemToFix] = useState<MediaItem | null>(null)
  const [showTmdbKeyNotice, setShowTmdbKeyNotice] = useState(false)
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false)
  const [theme] = useState<'dark' | 'light'>('dark')
  const { toast } = useToast()
  const optim = useAutoOptimize()

  // Confirm dialog state for delete confirmations
  const [confirmDeleteOpen, setConfirmDeleteOpen] = useState(false)
  const confirmDeleteActionRef = useRef<(() => void) | null>(null)
  const [confirmDeleteDesc, setConfirmDeleteDesc] = useState('')

  useEffect(() => {
    const preloadTimer = window.setTimeout(() => {
      // Preload ALL lazy components so they don't suspend during user interaction
      void loadSettingsModal()
      void loadEpisodeBrowser()
      void loadWatchTogetherModal()
      void loadFixMatchModal()
      void loadSyncValidatorModal()
      import('@/components/DuplicateDetector')
      import('@/components/CalendarView')
      import('@/components/DeveloperConsole')
      import('@/components/FullHistoryView')
      import('@/components/DirectLinksView')
      import('@/components/RemoteSource/RemoteSourceView')
    }, 500)

    return () => window.clearTimeout(preloadTimer)
  }, [])

  // On startup, if no TMDB API key is configured and the user hasn't explicitly
  // dismissed the notice, show a persistent in-page banner with two actions:
  //   • "Open Settings" — opens the API tab where the key can be entered
  //   • "Don't show again" — persists dismissal in localStorage
  useEffect(() => {
    let cancelled = false
    const bootstrapTimer = window.setTimeout(async () => {
      if (isTmdbKeyNoticeDismissed()) return
      try {
        const config = await getConfig()
        if (cancelled) return
        const hasKey = !!(config.tmdb_api_key && config.tmdb_api_key.trim())
        if (hasKey) return
        if (cancelled) return
        setShowTmdbKeyNotice(true)
      } catch (error) {
        console.warn("[TMDB-KEY-NOTICE] failed to check config:", error)
      }
    }, 1500)

    return () => {
      cancelled = true
      window.clearTimeout(bootstrapTimer)
    }
  }, [])

  // Resume dialog state
  const [resumeDialogOpen, setResumeDialogOpen] = useState(false)
  const [resumeDialogData, setResumeDialogData] = useState<{
    item: MediaItem
    resumeInfo: ResumeInfo
    posterUrl?: string
  } | null>(null)
  const [contentDetailsOpen, setContentDetailsOpen] = useState(false)
  const [contentDetailsItem, setContentDetailsItem] = useState<MediaItem | null>(null)
  const [playConfirmOpen, setPlayConfirmOpen] = useState(false)
  const [playConfirmData, setPlayConfirmData] = useState<MediaItem | null>(null)

  // Delete modal state
  const [deleteModalOpen, setDeleteModalOpen] = useState(false)
  const [deleteModalData, setDeleteModalData] = useState<{
    seriesId: number
    seriesTitle: string
  } | null>(null)

  // Watch Together state
  const [watchTogetherOpen, setWatchTogetherOpen] = useState(false)
  const [watchTogetherMedia, setWatchTogetherMedia] = useState<MediaItem | null>(null)

  // Watch Together session state (persists across modal open/close)
  const [wtActiveRoom, setWtActiveRoom] = useState<WatchRoom | null>(null)
  const [wtSessionId, setWtSessionId] = useState('')
  const [wtIsPlaying, setWtIsPlaying] = useState(false)
  const [wtSessionMedia, setWtSessionMedia] = useState<MediaItem | null>(null) // Media for the session

  // Tab visibility state - cloud-only mode
  const [tabVisibility, setTabVisibilityState] = useState<TabVisibility>(getTabVisibility)

  // Cloud connection state for contextual empty states
  const [isGDriveConnected, setIsGDriveConnected] = useState(false)

  // Mark complete dialog state
  const [markCompleteDialogOpen, setMarkCompleteDialogOpen] = useState(false)
  const [markCompleteData, setMarkCompleteData] = useState<{
    mediaId: number
    title: string
    seasonEpisode?: string
    progressPercent: number
    isCompletionConfirmation?: boolean // True when MPV detected end chapter
  } | null>(null)

  // Expired DDL link dialog state
  const [ddlExpiredDialogOpen, setDdlExpiredDialogOpen] = useState(false)
  const [ddlExpiredSourceId, setDdlExpiredSourceId] = useState<string | null>(null)
  const [ddlExpiredItem, setDdlExpiredItem] = useState<MediaItem | null>(null)
  const [ddlExpiredNewUrl, setDdlExpiredNewUrl] = useState('')
  const [ddlExpiredError, setDdlExpiredError] = useState('')
  const [ddlExpiredRefreshing, setDdlExpiredRefreshing] = useState(false)

  // Authentication state
  const { state, isAuthenticated, isAuthLoading, isLoggingIn, login: handleLogin, logout: handleLogout, reauth: _handleReauth, needsReauth: _needsReauth, showIndexingPrompt, isIndexing, confirmIndexing, declineIndexing } = useAuth()

  // Write-gate: wraps Drive-mutating operations so that, when the refresh
  // token is soft_lost, the user is prompted to re-authenticate and the
  // write only runs after re-auth completes. Read-only reads stay ungated.
  const { guard: guardDriveWrite } = useAuthGuard()

  const mergeDownloadJob = useCallback((job: DownloadJob) => {
    setDownloadJobs((current) => {
      const existingIndex = current.findIndex((entry) => entry.id === job.id)
      if (existingIndex === -1) {
        return [job, ...current].sort((left, right) =>
          new Date(right.createdAt).getTime() - new Date(left.createdAt).getTime()
        )
      }

      const next = [...current]
      next[existingIndex] = job
      next.sort((left, right) =>
        new Date(right.createdAt).getTime() - new Date(left.createdAt).getTime()
      )
      return next
    })
  }, [])

  const loadDownloadQueue = useCallback(async () => {
    try {
      const jobs = await getDownloadJobs()
      setDownloadJobs(jobs)
    } catch (error) {
      console.error('[Downloads] Failed to load jobs:', error)
    }
  }, [])

  const [notificationCenterOpen, setNotificationCenterOpen] = useState(false)
  const [notificationFilter, setNotificationFilter] = useState<NotificationFilter>('all')
  const [notifications, setNotifications] = useState<AppNotificationItem[]>(() => loadStoredNotifications())

  const notificationIdCounter = useRef(0)
  const pushNotification = useCallback((input: Omit<AppNotificationItem, 'id' | 'createdAt' | 'read'> & { createdAt?: string }) => {
    setNotifications((current) => [
      {
        id: `notif-${++notificationIdCounter.current}`,
        createdAt: input.createdAt ?? new Date().toISOString(),
        read: false,
        ...input,
      },
      ...current,
    ].slice(0, MAX_NOTIFICATION_CENTER_ITEMS))
  }, [])

  const clearNotifications = useCallback(() => {
    setNotifications([])
  }, [])

  const unreadNotificationCount = useMemo(
    () => notifications.filter((item) => !item.read).length,
    [notifications],
  )

  // Persist notification center state to localStorage (debounced)
  const notificationsRef = useRef(notifications)
  notificationsRef.current = notifications

  useEffect(() => {
    const id = setTimeout(() => {
      try {
        localStorage.setItem(NOTIFICATION_CENTER_STORAGE_KEY, JSON.stringify(notifications))
      } catch {
        console.warn('[App] Failed to persist notifications')
      }
    }, 2000)
    return () => clearTimeout(id)
  }, [notifications])

  // Flush pending notification writes on tab/window close
  useEffect(() => {
    const flush = () => {
      try {
        localStorage.setItem(NOTIFICATION_CENTER_STORAGE_KEY, JSON.stringify(notificationsRef.current))
      } catch { /* ignore */ }
    }
    window.addEventListener('beforeunload', flush)
    return () => {
      window.removeEventListener('beforeunload', flush)
      // Also flush on unmount in React's case
      flush()
    }
  }, [])

  const handleOpenNotificationCenter = useCallback(() => {
    setNotificationCenterOpen(true)
    setNotifications((current) => current.map((item) => (
      item.read ? item : { ...item, read: true }
    )))
  }, [])

  // Beta features state
  const [betaEnabled, setBetaEnabledState] = useState(isBetaEnabled)

  // Update notification state
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null)
  const [updateGateStatus, setUpdateGateStatus] = useState<'checking' | 'downloading' | 'installing' | 'error' | 'idle'>('idle')
  const [updateGateMessage, setUpdateGateMessage] = useState('Checking for updates...')
  const [updateGateError, setUpdateGateError] = useState<string | null>(null)
  const [updateProgress, setUpdateProgress] = useState(0)
  const [isUpdateNoticeVisible, setIsUpdateNoticeVisible] = useState(false)
  // Initialize beta features

  const checkForAvailableUpdate = useCallback(async (showCheckErrors = false) => {
    setUpdateGateStatus('checking')
    setUpdateGateMessage('Checking for updates...')
    setUpdateGateError(null)

    try {
      const info = await checkForUpdates()
      if (!info.available) {
        setUpdateInfo(null)
        setIsUpdateNoticeVisible(false)
        setUpdateGateStatus('idle')
        return
      }

      setUpdateInfo(info)
      setIsUpdateNoticeVisible(true)
      setUpdateGateStatus('idle')
      setUpdateGateMessage(`Update available: v${info.latest_version}`)
    } catch (error) {
      console.error('[Update] Update check failed:', error)
      if (showCheckErrors) {
        setUpdateGateError(formatUpdateError(error))
        setUpdateGateStatus('error')
        setUpdateGateMessage('Unable to check for updates.')
        setIsUpdateNoticeVisible(true)
      } else {
        setUpdateGateStatus('idle')
      }
    }
  }, [])

  const startUpdateInstall = useCallback(async () => {
    if (!updateInfo?.download_url) {
      setUpdateGateStatus('error')
      setUpdateGateMessage('Update failed.')
      setUpdateGateError('Missing update download URL.')
      setIsUpdateNoticeVisible(true)
      return
    }

    let unlistenProgress: UnlistenFn | null = null

    setUpdateGateError(null)
    setUpdateProgress(0)
    setIsUpdateNoticeVisible(true)

    try {
      setUpdateGateStatus('downloading')
      setUpdateGateMessage(`Downloading update v${updateInfo.latest_version}...`)

      unlistenProgress = await listen<{ progress: number }>('update-download-progress', (event) => {
        const value = Math.max(0, Math.min(100, Math.round(event.payload.progress)))
        setUpdateProgress(value)
      })

      await invoke('plugin:autostart|enable')

      const installerPath = await downloadUpdate(updateInfo.download_url, updateInfo.published_at ?? undefined)

      setUpdateGateStatus('installing')
      setUpdateGateMessage('Installing update and restarting...')
      await installUpdate(installerPath)
    } catch (error) {
      console.error('[Update] Update install failed:', error)
      setUpdateGateStatus('error')
      setUpdateGateMessage('Auto update failed. You can keep using the app and retry later.')
      setUpdateGateError(formatUpdateError(error))
      setIsUpdateNoticeVisible(true)
    } finally {
      if (unlistenProgress) {
        unlistenProgress()
      }
    }
  }, [updateInfo])

  // Background update check on app start
  useEffect(() => {
    void checkForAvailableUpdate()
  }, [checkForAvailableUpdate])

  // Handle beta toggle
  const handleBetaToggle = (enabled: boolean) => {
    setBetaEnabled(enabled)
    setBetaEnabledState(enabled)
    if (!enabled) {
      setView('home')
    }
    toast({
      title: enabled ? "Beta Features Enabled" : "Beta Features Disabled",
      description: enabled
        ? "Watch Together features are now available"
        : "Watch Together features are now hidden"
    })
  }

  // Check GDrive connection status for contextual empty states
  const checkGDriveStatus = async () => {
    try {
      const { isGDriveConnected: checkConnected } = await import('@/services/gdrive')
      const connected = await checkConnected()
      setIsGDriveConnected(connected)
    } catch (e) {
      console.warn('[App] GDrive status check failed:', e)
      setIsGDriveConnected(false)
    }
  }

  // Check GDrive status when switching to cloud view or on mount
  useEffect(() => {
    if (view === 'cloud') {
      checkGDriveStatus()
    }
  }, [view])

  useEffect(() => {
    void loadDownloadQueue()
  }, [loadDownloadQueue])

  useEffect(() => {
    let unlistenDownloadUpdates: UnlistenFn | undefined
    let unlistenDownloadCleared: UnlistenFn | undefined

    const setupDownloadListener = async () => {
      unlistenDownloadUpdates = await listen<DownloadJob>('download-job-updated', (event) => {
        mergeDownloadJob(event.payload)
      })

      unlistenDownloadCleared = await listen<DownloadJob[]>('download-queue-cleared', (event) => {
        setDownloadJobs(event.payload)
      })
    }

    void setupDownloadListener()
    return () => {
      unlistenDownloadUpdates?.()
      unlistenDownloadCleared?.()
    }
  }, [mergeDownloadJob])

  // Handler for tab visibility changes from settings
  const handleTabVisibilityChange = (visibility: TabVisibility) => {
    setTabVisibility(visibility)
    setTabVisibilityState(visibility)
    // If user hides the cloud view, navigate to home
    if (view === 'cloud' && !visibility.showCloud) {
      setView('home')
    }
  }

  // Listen for Tauri events - depends on view to properly refresh data
  // Global shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // CTRL+F or CMD+F to search
      if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
        e.preventDefault()
        if (view !== 'home') {
          setView('home')
          // Small delay to allow view transition before focusing
          setTimeout(() => searchInputRef.current?.focus(), 150)
        } else {
          searchInputRef.current?.focus()
        }
      }

      // CTRL+K or CMD+K to open command palette
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault()
        setCommandPaletteOpen((prev) => !prev)
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [view])

  useEffect(() => {
    document.documentElement.classList.add('dark')

    // Disable right-click context menu in production
    if (import.meta.env.PROD) {
      const handleContextMenu = (e: MouseEvent) => {
        e.preventDefault()
      }
      document.addEventListener('contextmenu', handleContextMenu)
      return () => document.removeEventListener('contextmenu', handleContextMenu)
    }
  }, [])

  // Cloud change detection is now handled by the Rust backend
  // The backend polls every 60 seconds and emits 'library-updated' events
  // which are already handled elsewhere in the app

  // Load library stats - cloud only
  const loadLibraryStats = useCallback(async () => {
    try {
      const stats = await getLibraryStats(true)
      setLibraryStats(stats)
      saveCachedLibrarySize(stats.movies + stats.shows + stats.episodes)
    } catch (error) {
      console.error('Failed to load stats', error)
    }
  }, [])

  // Load continue watching
  const loadRecentlyAdded = useCallback(async () => {
    try {
      const isCloud = view === 'cloud' ? true : (view === 'home' ? undefined : false)
      const items = await getRecentlyAdded(10, isCloud)
      setRecentlyAdded(items)
    } catch (error) {
      console.error('Failed to load recently added', error)
    }
  }, [view])

  const loadRecentlyAddedForHome = useCallback(async () => {
    try {
      const items = await getRecentlyAdded(10)
      setRecentlyAdded(items)
      return items
    } catch (error) {
      console.error('Failed to load recently added', error)
      return [] as MediaItem[]
    }
  }, [])

  const loadContinueWatching = useCallback(async () => {
    try {
      const history = await getWatchHistory()
      // Filter to items that are still meaningfully in progress.
      const inProgress = history
        .filter(item => {
          const progress = getMediaProgressPercent(item)
          return progress > 0 && !isProgressPastAutoCompleteThreshold(progress)
        })
        .slice(0, 10)
      setContinueWatching(inProgress)
    } catch (error) {
      console.error('Failed to load continue watching', error)
    }
  }, [])

  const loadHistoryEvents = useCallback(async () => {
    try {
      await getWatchHistoryEvents()
    } catch (error) {
      console.error('Failed to load history events', error)
    }
  }, [])

  const loadAnalytics = useCallback(async () => {
    try {
      const data = await getAnalyticsData()
      setAnalyticsData(data)
    } catch (error) {
      console.error('Failed to load analytics data', error)
    }
  }, [])

  const runWatchHistorySync = useCallback(async () => {
    try {
      // syncWatchHistory is the only path that mutates the user's watch
      // history state on Google Drive. Soft_lost guard prompts a re-auth
      // modal before letting this run.
      await guardDriveWrite(() => syncWatchHistory())
    } catch (error) {
      // AuthCancelledError: user dismissed the re-auth modal — silently skip
      // the sync (local DB write in callers already succeeded; the Drive
      // sync will just retry later via runWatchHistorySync()).
      if (error instanceof AuthCancelledError) {
        return
      }
      console.error('[History] Sync failed:', error)
    }
  }, [guardDriveWrite])

  const handleHomeSearch = useCallback(async () => {
    if (!homeSearchQuery.trim()) {
      setHomeSearchResults([])
      return
    }

    setIsHomeSearching(true)
    try {
      // Single IPC call instead of 4 parallel calls
      const combined = await searchAllLibraries(homeSearchQuery)
      const query = homeSearchQuery.toLowerCase()

      // Use Schwartzian transform to pre-calculate lowercase titles for faster sorting
      const sorted = combined
        .map(item => ({ item, titleLower: item.title.toLowerCase() }))
        .sort((a, b) => {
          const aTitle = a.titleLower
          const bTitle = b.titleLower
          if (aTitle === query && bTitle !== query) return -1
          if (bTitle === query && aTitle !== query) return 1
          if (aTitle.startsWith(query) && !bTitle.startsWith(query)) return -1
          if (bTitle.startsWith(query) && !aTitle.startsWith(query)) return 1
          return aTitle.localeCompare(bTitle)
        })
        .map(({ item }) => item)

      setHomeSearchResults(sorted)
    } catch (error) {
      console.error("Failed to search", error)
    } finally {
      setIsHomeSearching(false)
    }
  }, [homeSearchQuery])

  const handleDdlIndexComplete = useCallback(async ({ contentName }: { mediaIds: number[]; contentName: string }) => {
    setView('home')
    await loadRecentlyAddedForHome()
    toast({
      title: 'Content Indexed',
      description: `${contentName} is now available in Newly Added.`
    })
  }, [loadRecentlyAddedForHome, toast])

  const fetchData = useCallback(async () => {
    try {
      let data: MediaItem[] = []
      if (view === 'cloud' && cloudSubTab !== 'collections') {
        // Cloud view - filter by is_cloud = true
        const mediaType = cloudSubTab === 'movies' ? 'movie' : 'tv'
        data = await getLibraryFiltered(mediaType, searchQuery, true)
      }

      // Sorting is now handled by useMemo (sortedItems)
      setItems(data)
    } catch (error) {
      console.error("Failed to fetch data", error)
    }
  }, [view, cloudSubTab, searchQuery])

  // Stable ref for fetchData to avoid callback identity churn in downstream handlers.
  const fetchDataRef = useRef(fetchData)
  fetchDataRef.current = fetchData

  // Load initial data
  useEffect(() => {
    loadContinueWatching()
    loadRecentlyAdded()
    loadLibraryStats()
  }, [tabVisibility, loadContinueWatching, loadRecentlyAdded, loadLibraryStats])

  useEffect(() => {
    if (!isAuthenticated) {
      return
    }

    let cancelled = false

    const syncAndRefresh = async () => {
      await runWatchHistorySync()
      if (cancelled) return
      await loadContinueWatching()
      await loadRecentlyAdded()
      if (cancelled) return
      await loadHistoryEvents()
    }

    void syncAndRefresh()

    return () => {
      cancelled = true
    }
  }, [isAuthenticated, loadContinueWatching, loadRecentlyAdded, loadHistoryEvents, runWatchHistorySync])

  useEffect(() => {
    let unlistenProgress: UnlistenFn | undefined
    let unlistenComplete: UnlistenFn | undefined
    let unlistenMpvEnded: UnlistenFn | undefined
    let unlistenMpvAudioTracks: UnlistenFn | undefined
    let unlistenMpvSubtitleTracks: UnlistenFn | undefined
    let unlistenLibraryUpdated: UnlistenFn | undefined
    let unlistenNotification: UnlistenFn | undefined
    let unlistenCloudIndexingStarted: UnlistenFn | undefined
    let unlistenZipProcessing: UnlistenFn | undefined
    let unlistenReminderFired: UnlistenFn | undefined
    let unlistenWatchlistReminderFired: UnlistenFn | undefined
    let unlistenMpvEvent: UnlistenFn | undefined

    const setupListeners = async () => {
      unlistenProgress = await listen<ScanProgressPayload>('scan-progress', (event) => {
        const payload = event.payload
        setScanProgress({
          current: payload.current,
          total: payload.total,
          title: payload.title
        })
      })

      unlistenCloudIndexingStarted = await listen<{ count: number }>('cloud-indexing-started', () => {
        setIsCloudIndexing(true)
      })

      unlistenReminderFired = await listen<ReminderFiredPayload>('movie-reminder-fired', (event) => {
        const reminder = event.payload
        pushNotification({
          category: 'reminder',
          title: 'Reminder',
          message: `It's time for ${reminder.title}!`,
        })
        toast({
          title: 'Reminder',
          description: `It's time for ${reminder.title}!`,
        })
        emit('refresh-reminders')
      })

      unlistenWatchlistReminderFired = await listen<WatchlistReminderFiredPayload>('watchlist-reminder-fired', (event) => {
        const item = event.payload
        const isSpam = item.notification_mode === 'spam'
        const title = isSpam ? 'Spam Reminder' : 'Watchlist Reminder'
        const message = isSpam
          ? `${item.title} is still waiting in your watchlist.`
          : `${item.title} is on your watchlist.`

        pushNotification({
          category: 'reminder',
          title,
          message,
        })
        toast({
          title,
          description: message,
        })
        emit('refresh-watchlist')
      })

      unlistenZipProcessing = await listen<ZipProcessingStatusPayload>('zip-processing-status', (event) => {
        const payload = event.payload

        if (zipProcessingPopupTimeoutRef.current) {
          window.clearTimeout(zipProcessingPopupTimeoutRef.current)
          zipProcessingPopupTimeoutRef.current = null
        }

        setZipProcessingPopup({
          phase: payload.phase,
          archiveCount: payload.archiveCount,
          archiveName: payload.archiveName ?? null,
          episodesIndexed: payload.episodesIndexed ?? null,
          message: payload.message,
        })

        if (payload.phase === 'detected') {
          setIsCloudIndexing(true)
          return
        }

        zipProcessingPopupTimeoutRef.current = window.setTimeout(() => {
          setZipProcessingPopup(null)
          zipProcessingPopupTimeoutRef.current = null
        }, payload.phase === 'error' ? 6500 : 5000)
      })

      unlistenComplete = await listen<ScanCompletePayload>('scan-complete', async () => {
        setIsScanning(false)
        setScanProgress(null)
        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          await loadHistoryEvents()
        }
        await Promise.all([loadLibraryStats(), loadContinueWatching(), loadRecentlyAdded()])

        toast({ title: "Scan Complete", description: "Library has been updated." })
      })

      unlistenMpvEnded = await listen<MpvPlaybackEndedPayload>('mpv-playback-ended', async (event) => {
        const { media_id, title, episode_title, season_number, episode_number, media_type, completed, final_position, final_duration } = event.payload

        const seasonEpisode = media_type === 'tvepisode' && season_number && episode_number
          ? `S${String(season_number).padStart(2, '0')}E${String(episode_number).padStart(2, '0')}`
          : undefined
        const resolvedTitle = episode_title || title
        const displayTitle = seasonEpisode ? `${resolvedTitle} (${seasonEpisode})` : resolvedTitle
        const autoMarkAsWatched = async () => {
          await markAsComplete(media_id)
          await emit('media-marked-complete', { media_id })
          toast({ title: "Marked Complete", description: `${displayTitle} marked as watched` })
        }

        if (completed) {
          try {
            await autoMarkAsWatched()
          } catch (e) {
            console.warn('[App] autoMarkAsWatched failed:', e)
            toast({ title: "Progress Saved", description: `${displayTitle} - 100% watched` })
          }
        } else if (final_position && final_duration && final_position > 30) {
          const progressPercent = (final_position / final_duration) * 100

          if (isProgressPastAutoCompleteThreshold(progressPercent)) {
            try {
              await autoMarkAsWatched()
            } catch (e) {
              console.warn('[App] autoMarkAsWatched failed:', e)
              toast({ title: "Progress Saved", description: `${displayTitle} - ${progressPercent.toFixed(0)}% watched` })
            }
          } else if (shouldPromptToMarkComplete(progressPercent)) {
            setMarkCompleteData({
              mediaId: media_id,
              title: resolvedTitle,
              seasonEpisode,
              progressPercent,
              isCompletionConfirmation: false
            })
            setMarkCompleteDialogOpen(true)
          } else {
            toast({ title: "Progress Saved", description: `${displayTitle} - ${progressPercent.toFixed(0)}% watched` })
          }
        }

        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          await loadHistoryEvents()
        }
        await Promise.all([loadContinueWatching(), loadRecentlyAdded(), runWatchHistorySync()])
      })

      unlistenMpvAudioTracks = await listen<MpvAudioTracksDetectedPayload>('mpv-audio-tracks-detected', (event) => {
        const { series_id, tracks } = event.payload
        if (!series_id || !Array.isArray(tracks)) {
          return
        }

        const nextTracks = tracks.toSorted((left, right) =>
          left.label.localeCompare(right.label),
        )
        mergeCachedSeriesAudioTracks(series_id, nextTracks)
      })

      unlistenMpvSubtitleTracks = await listen<MpvSubtitleTracksDetectedPayload>('mpv-subtitle-tracks-detected', (event) => {
        const { series_id, tracks } = event.payload
        if (!series_id || !Array.isArray(tracks)) {
          return
        }

        const nextTracks = tracks.toSorted((left, right) =>
          left.label.localeCompare(right.label),
        )
        mergeCachedSeriesSubtitleTracks(series_id, nextTracks)
      })

      // Native libmpv player events
      unlistenMpvEvent = await listen<[string, unknown]>('mpv-event', async (event) => {
        const [eventType, raw] = event.payload
        if (!eventType) return

        if (eventType === 'mpv-event-ended') {
          const reason = (raw as Record<string, string>)?.reason
          console.debug('[MPV-NATIVE] Playback ended:', reason)
          setIsNativePlaying(false)
          await Promise.all([loadContinueWatching(), loadRecentlyAdded(), runWatchHistorySync()])
          return
        }

        if (eventType === 'mpv-prop-change') {
          const data = raw as Record<string, unknown>
          const name = data?.name as string
          const val = data?.data
          if (!name) return

          switch (name) {
            case 'time-pos':
              if (typeof val === 'number') setNativePos(val)
              break
            case 'duration':
              if (typeof val === 'number') setNativeDuration(val)
              break
            case 'pause':
              if (typeof val === 'boolean') setNativePaused(val)
              break
            case 'volume':
              if (typeof val === 'number') setNativeVolume(val)
              break
            case 'mute':
              if (typeof val === 'boolean') setNativeMuted(val)
              break
            case 'aid':
              if (typeof val === 'number') setNativeAid(val)
              break
            case 'sid':
              if (typeof val === 'number') setNativeSid(val)
              break
            case 'sub-scale':
              if (typeof val === 'number') setNativeSubScale(val)
              break
            case 'track-list':
              if (Array.isArray(val)) {
                const audioTracks = val
                  .filter(t => (t as Record<string, unknown>)?.type === 'audio')
                  .map((t, idx) => {
                    const track = t as Record<string, unknown>
                    return { id: track.id as number, lang: (track.lang as string) || `Track ${idx + 1}` }
                  })
                if (audioTracks.length > 0) setNativeAudioTracks(audioTracks)

                const subTracks = val
                  .filter(t => (t as Record<string, unknown>)?.type === 'sub')
                  .map((t, idx) => {
                    const track = t as Record<string, unknown>
                    return { id: track.id as number, lang: (track.lang as string) || `Track ${idx + 1}` }
                  })
                if (subTracks.length > 0) setNativeSubTracks(subTracks)
              }
              break
          }
        }
      })

      unlistenLibraryUpdated = await listen<{ type?: string; title?: string; media_id?: number; parent_id?: number }>('library-updated', async (event) => {
        const payload = event.payload || {}

        setIsCloudIndexing(false)
        await loadDownloadQueue()

        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          await loadHistoryEvents()
        } else if (view === 'episodes' && selectedShow) {
          try {
            const selectedId = selectedShow.id
            const changedMediaId = typeof payload.media_id === 'number' ? payload.media_id : null
            const changedParentId = typeof payload.parent_id === 'number' ? payload.parent_id : null

            if (
              changedMediaId === null
              || changedParentId === selectedId
              || changedMediaId === selectedId
            ) {
              const refreshedShow = await getMediaInfo(selectedId)
              setSelectedShow(refreshedShow)
            }
          } catch (error) {
            console.warn('[App] Failed to refresh selected show after library update:', error)
          }
        }
        await Promise.all([loadLibraryStats(), loadContinueWatching(), loadRecentlyAdded()])
      })

      unlistenNotification = await listen<{ type: string; title: string; message: string }>('notification', (event) => {
        const { type, title, message } = event.payload
        pushNotification({
          category: classifyNotificationCategory(title, message),
          title,
          message,
        })
        toast({
          title,
          description: message,
          variant: type === 'success' ? 'default' : type === 'info' ? 'info' : 'destructive'
        })
      })
    }

    void setupListeners()
    return () => {
      unlistenProgress?.()
      unlistenComplete?.()
      unlistenMpvEnded?.()
      unlistenMpvAudioTracks?.()
      unlistenMpvSubtitleTracks?.()
      unlistenMpvEvent?.()
      unlistenLibraryUpdated?.()
      unlistenNotification?.()
      unlistenCloudIndexingStarted?.()
      unlistenZipProcessing?.()
      unlistenReminderFired?.()
      unlistenWatchlistReminderFired?.()
    }
  }, [view, selectedShow, fetchData, loadContinueWatching, loadRecentlyAdded, loadHistoryEvents, loadLibraryStats, loadDownloadQueue, runWatchHistorySync, pushNotification, toast])

  useEffect(() => {
    if (view !== 'episodes' && view !== 'home' && view !== 'reminders' && view !== 'downloads') {
      // Fetch immediately on tab switch; only debounce active typing.
      const delayMs = searchQuery.trim() ? 180 : 0
      const timer = window.setTimeout(() => {
        if (view === 'history') {
          loadHistoryEvents()
          return
        }
        if (view === 'analytics') {
          loadAnalytics()
          return
        }
        fetchData()
      }, delayMs)
      return () => window.clearTimeout(timer)
    }
  }, [view, searchQuery, cloudSubTab, fetchData, loadHistoryEvents, loadAnalytics])

  // Dev-only test trigger listener
  useEffect(() => {
    if (!import.meta.env.DEV) return

    let lastTriggerTime = 0
    const interval = setInterval(() => {
      fetch('/test-trigger.json')
        .then((res) => {
          if (!res.ok) throw new Error('Not found')
          return res.json() as Promise<{ title: string; message: string; timestamp: number; type?: 'info' | 'success' | 'error' }>
        })
        .then((data) => {
          if (data && typeof data.timestamp === 'number' && data.timestamp > lastTriggerTime) {
            lastTriggerTime = data.timestamp
            pushNotification({
              category: classifyNotificationCategory(data.title, data.message),
              title: data.title,
              message: data.message,
            })
            toast({
              title: data.title,
              description: data.message,
              variant: data.type === 'success' ? 'default' : data.type === 'error' ? 'destructive' : 'info',
            })
          }
        })
        .catch(() => {
          // Fail silently
        })
    }, 1000)

    return () => clearInterval(interval)
  }, [pushNotification, toast])

  useEffect(() => {
    if (view !== 'downloads') return
    void loadDownloadQueue()
  }, [view, loadDownloadQueue])

  useEffect(() => {
    if (view !== 'cloud') return

    if (!isChunkedCloudRender) {
      setVisibleCloudItemsCount(sortedItems.length)
      return
    }

    setVisibleCloudItemsCount(Math.min(CLOUD_INITIAL_RENDER_COUNT, sortedItems.length))
  }, [view, cloudSubTab, searchQuery, sortedItems.length, isChunkedCloudRender])

  useEffect(() => {
    if (view !== 'cloud' || !isChunkedCloudRender) return

    const sentinel = cloudLoadMoreRef.current
    if (!sentinel) return

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          setVisibleCloudItemsCount((prev) => {
            if (prev >= sortedItems.length) return prev
            return Math.min(prev + CLOUD_CHUNK_RENDER_COUNT, sortedItems.length)
          })
        }
      },
      {
        root: null,
        rootMargin: '260px 0px',
        threshold: 0.01,
      }
    )

    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [view, isChunkedCloudRender, sortedItems.length])

  useEffect(() => {
    if (view !== 'home') return
    if (!homeSearchQuery.trim()) {
      setHomeSearchResults([])
      return
    }

    const delayDebounceFn = setTimeout(() => {
      handleHomeSearch()
    }, 300)
    return () => clearTimeout(delayDebounceFn)
  }, [homeSearchQuery, view, handleHomeSearch])


  // Handle cloud-only indexing - scans the entire Google Drive for NEW files only
  const handleCloudScan = async () => {
    if (isScanning || isCloudIndexing) {
      toast({ title: "Update In Progress", description: "Library update is already running." })
      return
    }

    try {
      const { isGDriveConnected: checkConnected } = await import('@/services/gdrive')
      const connected = await checkConnected()

      if (!connected) {
        toast({
          title: "Not Connected",
          description: "Connect to Google Drive in Settings first"
        })
        return
      }

      setIsCloudIndexing(true)
      setCloudIndexingStatus('Scanning your entire Google Drive...')

      // Always scan the entire Google Drive (root recursively covers everything)
      const gdrive = await import('@/services/gdrive')
      // Both calls mutate Drive state (folder tracking + index write).
      // Soft_lost guard prompts re-auth before letting either run.
      const result = await guardDriveWrite(async () => {
        await gdrive.addCloudFolder('root', 'My Drive')
        return gdrive.scanCloudFolder('root', 'My Drive')
      })

      if (result.indexed_count > 0) {
        setCloudIndexingStatus(`✓ Added ${result.indexed_count} new files!`)
        toast({
          title: "Library Updated",
          description: `Added ${result.movies_count} movies and ${result.tv_count} TV shows`
        })
        // Refresh the view and stats
        await fetchData()
        await loadLibraryStats()
      } else {
        setCloudIndexingStatus('✓ ' + (result.message || 'Library is up to date'))
        toast({
          title: "Library Up to Date",
          description: result.message || "No new movies or TV shows found in your Drive"
        })
      }

      // Keep the success state visible briefly
      setTimeout(() => {
        setIsCloudIndexing(false)
        setCloudIndexingStatus('')
        setCloudIndexingProgress(null)
      }, 2500)

    } catch (error) {
      console.error('[CloudScan] Failed:', error)
      setIsCloudIndexing(false)
      setCloudIndexingStatus('')
      setCloudIndexingProgress(null)
      toast({
        title: "Update Failed",
        description: String(error) || "Failed to update library",
        variant: "destructive"
      })
    }
  }

  const handleContentDetailsOpenChange = useCallback((open: boolean) => {
    setContentDetailsOpen(open)
    if (!open) {
      setContentDetailsItem(null)
    }
  }, [])

  const launchPlaybackWithZipLoading = useCallback(async (
    item: MediaItem,
    resume: boolean,
    audioPreference?: string | null,
    subtitlePreference?: string | null,
  ) => {
    const loadingState = item.parent_zip_id ? buildZipPlaybackLoadingState(item, resume) : null
    let overlayVisibleSince = 0
    if (loadingState) {
      setZipPlaybackLoading(loadingState)
      await waitForZipLoadingOverlayPaint()
      overlayVisibleSince = Date.now()
    }

    const effectiveDuration = item.duration_seconds && item.duration_seconds > 0 ? item.duration_seconds : null
    const effectiveSize = item.zip_uncompressed_size ?? item.zip_compressed_size ?? item.file_size_bytes ?? null

    try {
      // Check player mode config — use native libmpv if enabled
      const config = await getConfig()
      const subtitleFilePath = consumePendingSubtitlePath(item.id)
      if (config.player_mode === 'native') {
        await playMediaNative(item.id, resume, audioPreference, subtitlePreference)
        setIsNativePlaying(true)
      } else {
        await playMedia(item.id, resume, audioPreference, subtitlePreference, effectiveDuration, effectiveSize, subtitleFilePath)
      }
      if (loadingState) {
        await waitForMpvPlaybackStart(item.id)
        await waitForMinimumZipOverlayVisibility(overlayVisibleSince)
      }
    } finally {
      if (loadingState) {
        setZipPlaybackLoading(null)
      }
    }
  }, [])

  const startPlaybackFlow = useCallback(async (item: MediaItem) => {
    try {
      const resumeInfo = await getResumeInfo(item.id)

      if (resumeInfo.has_progress && resumeInfo.progress_percent <= AUTO_MARK_WATCHED_THRESHOLD_PERCENT) {
        let posterUrl: string | undefined
        if (item.poster_path) {
          try {
            posterUrl = await getCachedImageUrl(item.poster_path.replace('image_cache/', '')) || undefined
          } catch (e) {
            console.debug('[App] Cache lookup failed:', e)
          }
        }

        setResumeDialogData({ item, resumeInfo, posterUrl })
        setResumeDialogOpen(true)
      } else {
        await launchPlaybackWithZipLoading(
          item,
          false,
          resolveSeriesAudioPreferenceForPlayback(item.parent_id, item.season_number),
          resolveSeriesSubtitlePreferenceForPlayback(item.parent_id, item.season_number),
        )
        toast({ title: "Playing", description: `Now playing: ${item.title}` })
      }
    } catch (e) {
      const msg = String(e)
      if (msg.includes("Link expired") && item.ddl_source_id) {
        setDdlExpiredSourceId(item.ddl_source_id)
        setDdlExpiredItem(item)
        setDdlExpiredNewUrl('')
        setDdlExpiredError('')
        setDdlExpiredDialogOpen(true)
      } else {
        toast({ title: "Error", description: msg || "Failed to start playback", variant: "destructive" })
      }
    }
  }, [launchPlaybackWithZipLoading, toast])

  const handleItemClick = useCallback((item: MediaItem) => {
    setContentDetailsItem(item)
    setContentDetailsOpen(true)
  }, [])

  const handleDetailsPrimaryAction = useCallback(async (item: MediaItem) => {
    setContentDetailsOpen(false)
    setContentDetailsItem(null)

    if (item.media_type === 'tvshow') {
      setSelectedShow(item)
      setView('episodes')
      return
    }

    if (item.media_type === 'movie') {
      await startPlaybackFlow(item)
      return
    }

    try {
      const resumeInfo = await getResumeInfo(item.id)
      if (resumeInfo.has_progress && resumeInfo.progress_percent <= AUTO_MARK_WATCHED_THRESHOLD_PERCENT) {
        await startPlaybackFlow(item)
      } else {
        setPlayConfirmData(item)
        setPlayConfirmOpen(true)
      }
    } catch (e) {
      console.error('[App] Failed to check resume info', e)
      setPlayConfirmData(item)
      setPlayConfirmOpen(true)
    }
  }, [startPlaybackFlow])

  const handlePlayConfirm = useCallback(async () => {
    if (!playConfirmData) return
    await startPlaybackFlow(playConfirmData)
    setPlayConfirmData(null)
  }, [playConfirmData, startPlaybackFlow])

  const handleDetailsMarkWatched = useCallback(async (item: MediaItem) => {
    try {
      await markAsComplete(item.id)
      await emit('media-marked-complete', { media_id: item.id })
      toast({
        title: "Marked as watched",
        description: item.media_type === 'tvepisode'
          ? `${item.title} saved as watched.`
          : `${item.title} marked as watched.`,
      })
      await Promise.all([
        loadContinueWatching(),
        loadRecentlyAdded(),
        loadHistoryEvents(),
        runWatchHistorySync(),
        fetchData(),
      ])
    } catch (e) {
      console.error('[App] Failed to mark as watched:', e)
      toast({ title: "Error", description: "Failed to mark as watched", variant: "destructive" })
    }
  }, [fetchData, loadContinueWatching, loadRecentlyAdded, loadHistoryEvents, runWatchHistorySync, toast])

  const handleDetailsUnwatch = useCallback(async (item: MediaItem) => {
    try {
      await clearProgress(item.id)
      toast({
        title: "Removed from watched",
        description: `${item.title} marked as unwatched.`,
      })
      await Promise.all([
        loadContinueWatching(),
        loadRecentlyAdded(),
        loadHistoryEvents(),
        runWatchHistorySync(),
        fetchData(),
      ])
    } catch (e) {
      console.error('[App] Failed to remove watched status:', e)
      toast({ title: "Error", description: "Failed to remove watched status", variant: "destructive" })
    }
  }, [fetchData, loadContinueWatching, loadRecentlyAdded, loadHistoryEvents, runWatchHistorySync, toast])

  const handleResumeChoice = async (resume: boolean) => {
    if (!resumeDialogData) return
    const { item, resumeInfo } = resumeDialogData
    const resumeTime = resume ? resumeInfo.position : 0
    try {
      await launchPlaybackWithZipLoading(
        item,
        resumeTime > 0,
        resolveSeriesAudioPreferenceForPlayback(item.parent_id, item.season_number),
        resolveSeriesSubtitlePreferenceForPlayback(item.parent_id, item.season_number),
      )
      toast({ title: "Playing", description: `Now playing: ${item.title}` })
      setResumeDialogOpen(false)
      setResumeDialogData(null)
    } catch (e) {
      const msg = String(e)
      if (msg.includes("Link expired") && item.ddl_source_id) {
        // Try auto-refresh from addon before showing manual dialog
        try {
          const newUrl = await invoke<string | null>('auto_refresh_ddl_from_addon', { sourceId: item.ddl_source_id })
          if (newUrl) {
            toast({ title: "Link refreshed", description: "Auto-refreshed from addon. Retrying playback..." })
            await startPlaybackFlow(item)
            return
          }
        } catch { /* auto-refresh failed, fall through to manual dialog */ }
        setDdlExpiredSourceId(item.ddl_source_id)
        setDdlExpiredItem(item)
        setDdlExpiredNewUrl('')
        setDdlExpiredError('')
        setDdlExpiredDialogOpen(true)
      } else {
        toast({ title: "Error", description: msg || "Failed to start playback", variant: "destructive" })
      }
    }
  }

  const handleFixMatch = useCallback((item: MediaItem) => {
    startTransition(() => {
      setItemToFix(item)
      setFixMatchOpen(true)
    })
  }, [])

  const handleStartDownload = useCallback(async (item: MediaItem) => {
    if (!item.is_cloud) {
      toast({
        title: 'Download unavailable',
        description: 'Direct downloads are available for SlasshyVault cloud items.',
        variant: 'destructive',
      })
      return
    }

    if (item.media_type === 'tvshow') {
      setContentDetailsItem(item)
      setContentDetailsOpen(true)
      return
    }

    try {
      const job = await startMediaDownload(item.id)
      mergeDownloadJob(job)
      toast({
        title: 'Download queued',
        description: `${item.title} was added to the Downloads tab.`,
      })
    } catch (error) {
      toast({
        title: 'Download failed',
        description: String(error) || 'Unable to start download.',
        variant: 'destructive',
      })
    }
  }, [mergeDownloadJob, toast])

  const handleCancelDownload = useCallback(async (job: DownloadJob) => {
    try {
      const updated = await cancelDownloadJob(job.id)
      mergeDownloadJob(updated)
      toast({
        title: 'Download cancelled',
        description: `${job.title} has been stopped.`,
      })
    } catch (error) {
      toast({
        title: 'Cancel failed',
        description: String(error) || 'Unable to cancel this download.',
        variant: 'destructive',
      })
    }
  }, [mergeDownloadJob, toast])

  const handleOpenDownload = useCallback(async (job: DownloadJob) => {
    try {
      await openDownloadJobTarget(job.id)
    } catch (error) {
      toast({
        title: 'Open failed',
        description: String(error) || 'Unable to open the downloaded file.',
        variant: 'destructive',
      })
    }
  }, [toast])

  const handleClearDownloadHistory = useCallback(async () => {
    try {
      await clearDownloadHistory()
    } catch (error) {
      toast({
        title: 'Clear failed',
        description: String(error) || 'Unable to clear download history.',
        variant: 'destructive',
      })
    }
  }, [toast])

  const handleDeleteDownload = useCallback(async (jobId: string) => {
    try {
      await deleteDownloadJob(jobId)
      setDownloadJobs(prev => prev.filter(j => j.id !== jobId))
    } catch (error) {
      toast({
        title: 'Delete failed',
        description: String(error) || 'Unable to delete download job.',
        variant: 'destructive',
      })
    }
  }, [toast])

  const handleFixMatchSuccess = useCallback(async () => {
    const fixedItem = itemToFix

    await Promise.all([
      fetchData(),
      loadLibraryStats(),
      loadContinueWatching(),
        loadRecentlyAdded(),
    ])

    // Emit update event so current view can hot-refresh in-place without manual reload.
    try {
      await emit('library-updated', {
        type: 'metadata-updated',
        title: fixedItem?.title || 'Metadata updated',
        media_id: fixedItem?.id || null,
        parent_id: fixedItem?.parent_id || null,
      })
    } catch (error) {
      console.warn('[FixMatch] Failed to emit library-updated event:', error)
    }

    if (selectedShow) {
      const shouldRefreshSelectedShow =
        view === 'episodes'
        || !fixedItem
        || selectedShow.id === fixedItem.id
        || selectedShow.id === (fixedItem.parent_id || -1)

      if (shouldRefreshSelectedShow) {
        try {
          const refreshedShow = await getMediaInfo(selectedShow.id)
          setSelectedShow(refreshedShow)
        } catch (error) {
          console.warn('[FixMatch] Failed to refresh selected show metadata:', error)
        }
      }
    }

    if (contentDetailsItem) {
      const shouldRefreshContentDetails =
        !fixedItem
        || contentDetailsItem.id === fixedItem.id
        || contentDetailsItem.id === (fixedItem.parent_id || -1)

      if (shouldRefreshContentDetails) {
        try {
          const refreshedItem = await getMediaInfo(contentDetailsItem.id)
          setContentDetailsItem(refreshedItem)
        } catch (error) {
          console.warn('[ContentDetails] Failed to refresh content details item:', error)
        }
      }
    }
  }, [contentDetailsItem, itemToFix, fetchData, loadLibraryStats, loadContinueWatching, loadRecentlyAdded, selectedShow, view])

  const handleContentDetailsMetadataRefresh = useCallback(async (itemId: number) => {
    try {
      const refreshedItem = await getMediaInfo(itemId)

      // Guard: don't overwrite if user navigated to different content
      setContentDetailsItem(prev => prev?.id === itemId ? refreshedItem : prev)

      if (selectedShow?.id === itemId) {
        setSelectedShow(refreshedItem)
      }

      await Promise.allSettled([
        fetchData(),
        loadLibraryStats(),
        loadContinueWatching(),
        loadRecentlyAdded(),
        loadHistoryEvents(),
      ])

      return refreshedItem
    } catch (error) {
      console.warn('[ContentDetails] Failed to refresh item after metadata update:', error)
      return null
    }
  }, [fetchData, loadContinueWatching, loadRecentlyAdded, loadHistoryEvents, loadLibraryStats, selectedShow])

  const handleDelete = useCallback(async (item: MediaItem) => {
    if (item.media_type === 'tvshow') {
      setDeleteModalData({ seriesId: item.id, seriesTitle: item.title })
      setDeleteModalOpen(true)
    } else {
      const desc = item.ddl_source_id
        ? `"${item.title}" is a direct-link item. Deleting it will remove it from your library. Continue?`
        : item.parent_zip_id
          ? `"${item.title}" comes from a ZIP archive. Deleting it will remove the ZIP archive from Google Drive and all indexed episodes from that archive. Continue?`
          : `Are you sure you want to permanently delete "${item.title}"?`
      setConfirmDeleteDesc(desc)
      confirmDeleteActionRef.current = async () => {
        try {
          const result = await deleteMediaFiles([item.id])
          if (result.success) {
            toast({ title: "Deleted", description: result.message })
            await fetchDataRef.current()
          } else {
            toast({ title: "Partial Delete", description: result.message, variant: "destructive" })
            await fetchDataRef.current()
          }
        } catch (e) {
          console.error('[App] Failed to delete file:', e)
          toast({ title: "Error", description: "Failed to delete file", variant: "destructive" })
        }
      }
      setConfirmDeleteOpen(true)
    }
  }, [toast])

  const handleDdlRefreshAndRetry = useCallback(async () => {
    if (!ddlExpiredSourceId || !ddlExpiredItem) return
    setDdlExpiredError('')
    setDdlExpiredRefreshing(true)
    try {
      const result = await invoke<{ accepted: boolean; message: string }>('ddl_refresh_link', {
        sourceId: ddlExpiredSourceId,
        newUrl: ddlExpiredNewUrl.trim(),
      })
      if (result.accepted) {
        setDdlExpiredDialogOpen(false)
        setDdlExpiredSourceId(null)
        const item = ddlExpiredItem
        setDdlExpiredItem(null)
        setDdlExpiredNewUrl('')
        toast({ title: "Link refreshed", description: "Retrying playback..." })
        await startPlaybackFlow(item)
      } else {
        setDdlExpiredError(result.message)
      }
    } catch (err: unknown) {
      setDdlExpiredError(err instanceof Error ? err.message : String(err))
    } finally {
      setDdlExpiredRefreshing(false)
    }
  }, [ddlExpiredSourceId, ddlExpiredItem, ddlExpiredNewUrl, toast, startPlaybackFlow])

  const handleDeleteComplete = useCallback(async (message?: string) => {
    await fetchDataRef.current()
    toast({ title: "Deleted", description: message || "Selected content has been permanently deleted." })
  }, [toast])

  const handleMarkComplete = async () => {
    if (!markCompleteData) return
    const data = markCompleteData
    setMarkCompleteData(null)
    try {
      await markAsComplete(data.mediaId)
      toast({ title: "Marked Complete", description: `${data.title} marked as watched` })
      // Emit event so EpisodeBrowser and other components can refresh
      await emit('media-marked-complete', { media_id: data.mediaId })
      await Promise.all([loadContinueWatching(), loadRecentlyAdded(), loadHistoryEvents(), runWatchHistorySync()])
      // Refresh library items to update progress display on cards
      await fetchData()
    } catch (e) {
      console.error('[App] Failed to mark as complete:', e)
      toast({ title: "Error", description: "Failed to mark as complete", variant: "destructive" })
    }
  }

  // Watch Together handler
  const handleWatchTogether = useCallback((item: MediaItem) => {
    // Only allow if beta is enabled
    if (!betaEnabled) return
    startTransition(() => {
      setWatchTogetherMedia(item)
      setWatchTogetherOpen(true)
    })
  }, [betaEnabled])

  // Watch Together session change handler
  const handleWtSessionChange = useCallback((room: WatchRoom | null, sessionId: string, isPlaying: boolean, media?: MediaItem) => {
    setWtActiveRoom(room)
    setWtSessionId(sessionId)
    setWtIsPlaying(isPlaying)
    if (media) {
      setWtSessionMedia(media)
    }
    if (!room) {
      setWtSessionMedia(null)
      setWatchTogetherMedia(null)
    }
  }, [])

  const handleWtLeave = () => {
    setWtActiveRoom(null)
    setWtSessionId('')
    setWtIsPlaying(false)
    setWtSessionMedia(null)
  }

  const toggleTheme = () => {
    toast({ title: "Theme Locked", description: "Dark mode is optimized for this interface." })
  }

  const handleToggleSidebarPin = useCallback(() => {
    const current = localStorage.getItem('slasshyvault_sidebar_pinned') === 'true'
    localStorage.setItem('slasshyvault_sidebar_pinned', String(!current))
    window.dispatchEvent(new CustomEvent('sidebar-pin-toggle'))
    toast({ title: current ? 'Sidebar Unpinned' : 'Sidebar Pinned' })
  }, [toast])

  const isUpdateGateActive = updateGateStatus === 'downloading' || updateGateStatus === 'installing'
  const showUpdateNotice = isUpdateNoticeVisible && (Boolean(updateInfo) || updateGateStatus === 'error')

  return (
    <OptimizationContext.Provider value={optim}>
    <div className={`flex h-screen text-foreground overflow-hidden ${isNativePlaying ? 'bg-transparent' : 'bg-background bg-gradient-mesh'}`}>
      {/* Indexing confirmation dialog for first-time users */}
      <Dialog open={showIndexingPrompt} onOpenChange={declineIndexing}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Index Your Google Drive?</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-4">
            <p className="text-sm text-muted-foreground">
              Would you like to scan your Google Drive and index all media files
              (movies, TV shows)? This will add them to your library so you can
              browse and stream them directly.
            </p>
            <p className="text-xs text-muted-foreground/70">
              You can always index your Drive later from Settings &rarr; Cloud Storage.
            </p>
            <div className="flex gap-2 justify-end">
              <Button
                variant="outline"
                onClick={declineIndexing}
                disabled={isIndexing}
              >
                Skip
              </Button>
              <Button
                onClick={confirmIndexing}
                disabled={isIndexing}
                className="gap-2"
              >
                {isIndexing ? (
                  <>
                    <Loader2 className="size-4 animate-spin" />
                    Indexing…
                  </>
                ) : (
                  "Yes, Index My Drive"
                )}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {isUpdateGateActive && (
        <div className="fixed inset-0 z-[400] flex items-center justify-center bg-black/70 backdrop-blur-sm">
          <div className="w-full max-w-lg mx-4 rounded-2xl border border-white/10 bg-[#121212]/95 shadow-2xl shadow-black/50 p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="p-2.5 rounded-xl bg-white/10">
                <Download className="size-5 text-neutral-200" />
              </div>
              <div>
                <h2 className="text-lg font-semibold text-white">Updating SlasshyVault</h2>
                <p className="text-sm text-neutral-400">
                  {updateInfo?.latest_version ? `v${updateInfo.latest_version}` : 'Checking version...'}
                </p>
              </div>
            </div>

            <p className="text-sm text-neutral-300 mb-4">
              {updateGateMessage}
            </p>

            {updateGateStatus === 'downloading' && (
              <div className="space-y-2">
                <div className="h-2 w-full rounded-full bg-white/10 overflow-hidden">
                  <div
                    className="h-full bg-white/60 transition-all"
                    style={{ width: `${updateProgress}%` }}
                  />
                </div>
                <div className="text-xs text-neutral-500">{updateProgress}%</div>
              </div>
            )}

            {updateGateStatus === 'installing' && (
              <div className="flex items-center gap-2 text-xs text-neutral-500">
                <Loader2 className="size-3.5 animate-spin" />
                Installing and restarting…
              </div>
            )}
          </div>
        </div>
      )}
      {showUpdateNotice && !isUpdateGateActive && (
        <div className="fixed top-12 right-4 z-[260] w-[min(420px,calc(100vw-2rem))] rounded-2xl border border-white/10 bg-[#121212]/95 shadow-2xl shadow-black/50 p-4 backdrop-blur-xl">
          <div className="flex items-start gap-3">
            <div className={`mt-0.5 p-2 rounded-xl ${updateGateStatus === 'error' ? 'bg-red-500/15' : 'bg-white/10'}`}>
              <Download className={`size-4 ${updateGateStatus === 'error' ? 'text-red-300' : 'text-neutral-200'}`} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-sm font-semibold text-white">
                    {updateGateStatus === 'error' ? 'Update Failed' : 'Update Available'}
                  </h2>
                  <p className="text-xs text-neutral-400">
                    {updateInfo?.latest_version ? `v${updateInfo.latest_version}` : 'SlasshyVault update'}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setIsUpdateNoticeVisible(false)}
                  className="rounded-md p-1 text-neutral-400 hover:bg-white/10 hover:text-white transition-colors"
                  aria-label="Close update notification"
                >
                  <X className="size-4" />
                </button>
              </div>

              <p className="mt-3 text-sm text-neutral-300">
                {updateGateStatus === 'error'
                  ? updateGateMessage
                  : 'A new version is available. You can update now or dismiss this notice and keep using the app.'}
              </p>

              {updateGateError && (
                <div className="mt-3 text-xs text-red-300/90 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 whitespace-pre-wrap break-words">
                  {updateGateError}
                </div>
              )}

              <div className="mt-4 flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void startUpdateInstall()}
                  disabled={!updateInfo?.download_url}
                  className="rounded-lg bg-white/10 hover:bg-white/15 disabled:opacity-50 disabled:cursor-not-allowed text-neutral-100 text-sm font-medium transition-colors border border-white/10 px-4 py-2"
                >
                  {updateGateStatus === 'error' ? 'Retry Update' : 'Update Now'}
                </button>
                <button
                  type="button"
                  onClick={() => setIsUpdateNoticeVisible(false)}
                  className="rounded-lg text-neutral-400 hover:text-white hover:bg-white/5 text-sm transition-colors px-3 py-2"
                >
                  Later
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
      {/* Show login screen if not authenticated */}
      {state === 'unauthenticated' && !isAuthLoading && (
        <LoginScreen onLogin={handleLogin} isLoading={isLoggingIn} />
      )}
      <ReauthBanner />

      {/* Show loading state while checking auth */}
      {isAuthLoading && (
        <div className="fixed inset-0 bg-[#0a0a0a] flex items-center justify-center z-[300]">
          <div className="flex flex-col items-center gap-4">
            <Loader2 className="size-8 animate-spin text-white" />
            <span className="text-neutral-400 text-sm">Loading…</span>
          </div>
        </div>
      )}

      {/* Main app content - only show when authenticated */}
      {isAuthenticated && (
        <>
          <ZipPlaybackLoadingOverlay loadingState={zipPlaybackLoading} />
          {isNativePlaying && (
            <div
              ref={controlsEl}
              className="fixed inset-0 z-[999] flex flex-col justify-between pointer-events-none"
            >
              {/* Top gradient */}
              <div className={`absolute top-0 left-0 right-0 h-32 bg-gradient-to-b from-black/70 to-transparent transition-opacity duration-300 ${controlsVisible ? 'opacity-100' : 'opacity-0 pointer-events-none'}`} />

              {/* Bottom gradient */}
              <div className={`absolute bottom-0 left-0 right-0 h-40 bg-gradient-to-t from-black/80 via-black/40 to-transparent transition-opacity duration-300 ${controlsVisible ? 'opacity-100' : 'opacity-0 pointer-events-none'}`} />

              {/* Top bar */}
              <div className={`relative z-10 flex items-center gap-3 px-4 pt-3 transition-opacity duration-300 ${controlsVisible ? 'opacity-100' : 'opacity-0 pointer-events-none'}`}>
                <button
                  onClick={() => { invoke('native_mpv_stop'); setIsNativePlaying(false) }}
                  className="pointer-events-auto flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm font-medium backdrop-blur-sm transition-colors"
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><path d="M19 12H5"/><path d="M12 19l-7-7 7-7"/></svg>
                  Back
                </button>
              </div>

              {/* Bottom bar */}
              <div className={`relative z-10 px-4 pb-4 transition-opacity duration-300 ${controlsVisible ? 'opacity-100' : 'opacity-0 pointer-events-none'}`}>
                {/* Timeline */}
                <div className="pointer-events-auto mb-3">
                  <input
                    type="range"
                    min={0}
                    max={nativeDuration || 1}
                    step={0.1}
                    value={nativePos}
                    onChange={e => {
                      const v = parseFloat(e.target.value)
                      invoke('native_mpv_seek', { position: v })
                    }}
                    className="w-full h-1 appearance-none bg-white/20 rounded-full cursor-pointer accent-white [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-lg"
                  />
                  <div className="flex justify-between mt-1 px-0.5">
                    <span className="text-white/60 text-xs tabular-nums">{formatTime(nativePos)}</span>
                    <span className="text-white/60 text-xs tabular-nums">{formatTime(nativeDuration)}</span>
                  </div>
                </div>

                {/* Controls row */}
                <div className="pointer-events-auto flex items-center gap-3">
                  {/* Play/Pause */}
                  <button
                    onClick={() => invoke('native_mpv_pause', { paused: !nativePaused })}
                    className="flex items-center justify-center w-10 h-10 rounded-full bg-white/10 hover:bg-white/20 backdrop-blur-sm transition-colors"
                  >
                    {nativePaused ? (
                      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" className="text-white"><polygon points="5,3 19,12 5,21"/></svg>
                    ) : (
                      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" className="text-white"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
                    )}
                  </button>

                  {/* Volume */}
                  <div className="flex items-center gap-2 group">
                    <button
                      onClick={() => invoke('native_mpv_set_property', { name: 'mute', value: !nativeMuted })}
                      className="text-white/70 hover:text-white transition-colors"
                    >
                      {nativeMuted || nativeVolume === 0 ? (
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>
                      ) : nativeVolume < 50 ? (
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>
                      ) : (
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>
                      )}
                    </button>
                    <div className="w-0 group-hover:w-20 overflow-hidden transition-all duration-200">
                      <input
                        type="range"
                        min={0}
                        max={100}
                        value={nativeVolume}
                        onChange={e => invoke('native_mpv_set_volume', { volume: parseInt(e.target.value) })}
                        className="w-20 h-1 appearance-none bg-white/20 rounded-full cursor-pointer accent-white [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white"
                      />
                    </div>
                    <span className="text-white/50 text-xs tabular-nums w-8">{nativeVolume}</span>
                  </div>

                  <div className="flex-1" />

                  {/* Subtitle track */}
                  <select
                    value={nativeSid ?? ''}
                    onChange={e => {
                      const v = e.target.value
                      invoke('native_mpv_set_property', { name: 'sid', value: v === 'off' ? 'no' : parseInt(v) })
                    }}
                    className="pointer-events-auto bg-white/10 hover:bg-white/20 backdrop-blur-sm text-white text-xs px-2 py-1.5 rounded-md border-0 outline-none cursor-pointer transition-colors"
                  >
                    <option value="off" className="bg-neutral-900 text-white">Sub: Off</option>
                    {nativeSubTracks.map(t => (
                      <option key={t.id} value={t.id} className="bg-neutral-900 text-white">Sub: {t.lang}</option>
                    ))}
                  </select>

                  {/* Subtitle size */}
                  <div className="flex items-center gap-1">
                    <button
                      onClick={() => setNativeSubScale(s => {
                        const v = Math.max(0.3, s - 0.1)
                        invoke('native_mpv_set_property', { name: 'sub-scale', value: v })
                        return v
                      })}
                      className="pointer-events-auto text-white/60 hover:text-white text-xs px-1.5 py-1 rounded hover:bg-white/10 transition-colors"
                    >A⁻</button>
                    <span className="text-white/40 text-[10px] tabular-nums w-6 text-center">{nativeSubScale.toFixed(1)}</span>
                    <button
                      onClick={() => setNativeSubScale(s => {
                        const v = Math.min(3, s + 0.1)
                        invoke('native_mpv_set_property', { name: 'sub-scale', value: v })
                        return v
                      })}
                      className="pointer-events-auto text-white/60 hover:text-white text-xs px-1.5 py-1 rounded hover:bg-white/10 transition-colors"
                    >A⁺</button>
                  </div>

                  {/* Audio track */}
                  <select
                    value={nativeAid ?? ''}
                    onChange={e => {
                      const v = e.target.value
                      invoke('native_mpv_set_property', { name: 'aid', value: v === 'off' ? 'no' : parseInt(v) })
                    }}
                    className="pointer-events-auto bg-white/10 hover:bg-white/20 backdrop-blur-sm text-white text-xs px-2 py-1.5 rounded-md border-0 outline-none cursor-pointer transition-colors"
                  >
                    {nativeAudioTracks.map(t => (
                      <option key={t.id} value={t.id} className="bg-neutral-900 text-white">Audio: {t.lang}</option>
                    ))}
                  </select>

                  {/* Fullscreen */}
                  <button
                    onClick={() => appWindow.toggleMaximize()}
                    className="pointer-events-auto text-white/70 hover:text-white transition-colors"
                  >
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3m18 0V5a2 2 0 0 0-2-2h-3m0 18h3a2 2 0 0 0 2-2v-3M3 16v3a2 2 0 0 0 2 2h3"/></svg>
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Custom Title Bar */}
          {!isNativePlaying && (
          <header className="fixed top-0 left-0 right-0 h-9 z-[220] border-b border-white/10 bg-background transition-colors duration-300">
            <div className="relative h-full w-full flex items-center justify-between">
              <div className="absolute top-0 left-0 right-0 h-1.5" />
              <div
                data-tauri-drag-region
                onDoubleClick={async () => {
                  await appWindow.toggleMaximize()
                  await refreshWindowState()
                }}
                className="absolute left-0 top-1.5 bottom-0 right-[120px]"
              />
              <div className="flex items-center gap-2 pl-3 select-none">
                <img
                  data-tauri-drag-region
                  src={slasshyvaultIcon}
                  alt=""
                  draggable={false}
                  className="pointer-events-none size-4 object-contain"
                />
                <span data-tauri-drag-region className="pointer-events-none text-[10px] font-semibold uppercase tracking-[0.2em] text-neutral-400">
                  SlasshyVault{import.meta.env.DEV ? ' dev' : ''}
                </span>
              </div>
              <div className="flex items-center gap-1 pr-1.5">
                <button
                  type="button"
                  onClick={() => appWindow.minimize()}
                  onDoubleClick={(event) => event.stopPropagation()}
                  className="h-7 w-8 rounded-md border border-transparent text-neutral-400 transition-colors hover:border-white/10 hover:bg-white/10 hover:text-white"
                  title="Minimize"
                  aria-label="Minimize window"
                >
                  <Minus className="mx-auto size-3.5" />
                </button>
                <button
                  type="button"
                  onClick={async () => {
                    await appWindow.toggleMaximize()
                    await refreshWindowState()
                  }}
                  onDoubleClick={(event) => event.stopPropagation()}
                  className="h-7 w-8 rounded-md border border-transparent text-neutral-400 transition-colors hover:border-white/10 hover:bg-white/10 hover:text-white"
                  title={isMaximized ? "Restore" : "Maximize"}
                  aria-label={isMaximized ? "Restore window" : "Maximize window"}
                >
                  {isMaximized ? <Minimize2 className="mx-auto size-3.5" /> : <Maximize2 className="mx-auto size-3.5" />}
                </button>
                <button
                  type="button"
                  onClick={async () => {
                    await appWindow.hide()
                  }}
                  onDoubleClick={(event) => event.stopPropagation()}
                  className="h-7 w-8 rounded-md border border-transparent text-neutral-400 transition-colors hover:border-rose-500/40 hover:bg-rose-500/20 hover:text-rose-200"
                  title="Close"
                  aria-label="Hide window"
                >
                  <X className="mx-auto size-3.5" />
                </button>
              </div>
            </div>
          </header>
          )}
          {/* Background decorative orbs */}
          {!isNativePlaying && (
          <div className="fixed inset-0 pointer-events-none overflow-hidden z-0">
            <div className="bg-orb bg-orb-1" />
            <div className="bg-orb bg-orb-2" />
            <div className="bg-orb bg-orb-3" />
          </div>
          )}

          <AnimatePresence>
            {zipProcessingPopup && (
              <motion.div
                initial={{ opacity: 0, y: 12, scale: 0.9 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: 6, scale: 0.9 }}
                className="fixed right-4 top-14 z-[230] flex items-center gap-2 rounded-full border border-white/10 bg-black/90 px-3 py-1.5 shadow-lg shadow-black/50 backdrop-blur-xl"
              >
                {zipProcessingPopup.phase === 'complete' ? (
                  <span className="text-[10px] text-white/60">✓</span>
                ) : zipProcessingPopup.phase === 'error' ? (
                  <span className="text-[10px] text-white/60">!</span>
                ) : (
                  <motion.div
                    animate={{ rotate: 360 }}
                    transition={{ duration: 2.2, repeat: Infinity, ease: 'linear' }}
                    className="flex"
                  >
                    <Archive className="size-3 text-white/50" />
                  </motion.div>
                )}
                <span className="text-[11px] font-medium text-white/70">
                  {zipProcessingPopup.phase === 'complete'
                    ? 'ZIP Indexed'
                    : zipProcessingPopup.phase === 'error'
                      ? 'ZIP Error'
                      : 'ZIP Detected'}
                </span>
                <span className="text-[10px] text-white/40">·</span>
                <span className="truncate text-[10px] text-white/50 max-w-[140px]">
                  {zipProcessingPopup.archiveName || zipProcessingPopup.message}
                </span>
                {typeof zipProcessingPopup.episodesIndexed === 'number' && zipProcessingPopup.phase === 'complete' && (
                  <>
                    <span className="text-[10px] text-white/40">·</span>
                    <span className="text-[10px] text-white/50">
                      {zipProcessingPopup.episodesIndexed}ep
                    </span>
                  </>
                )}
              </motion.div>
            )}
          </AnimatePresence>

          {!isNativePlaying && (
          <>
          <Sidebar
            currentView={view === 'episodes' ? 'cloud' : view}
            setView={(v) => {
              setView(v)
              setSelectedShow(null)
              setSearchQuery('')
              setHomeSearchQuery('')
              setHomeSearchResults([])
            }}
            onOpenSettings={() => startTransition(() => setSettingsOpen(true))}
            onCloudScan={handleCloudScan}
            onSyncValidator={() => startTransition(() => setShowSyncValidator(true))}
            onDuplicateDetector={() => startTransition(() => setShowDuplicateDetector(true))}
            theme={theme}
            toggleTheme={toggleTheme}
            isScanning={isScanning}
            isCloudIndexing={isCloudIndexing}
            scanProgress={scanProgress}
            showCloudTab={tabVisibility.showCloud}
            betaEnabled={betaEnabled}
            downloadJobCount={downloadJobCount}
            className="flex-shrink-0 z-50 h-screen sticky top-0"
          />

          <main className="flex-1 flex flex-col min-w-0 relative z-10 overflow-hidden">
            {/* Floating Scan Progress Indicator */}
            <AnimatePresence>
              {isScanning && scanProgress && (
                <motion.div
                  initial={{ opacity: 0, y: -20, scale: 0.9 }}
                  animate={{ opacity: 1, y: 0, scale: 1 }}
                  exit={{ opacity: 0, y: -20, scale: 0.9 }}
                  className="fixed top-12 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-3 px-4 py-2.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/30 shadow-lg"
                >
                  <div className="relative">
                    <Loader2 className="size-4 animate-spin text-white" />
                    <div className="absolute inset-0 rounded-full bg-white/40 blur-md animate-pulse" />
                  </div>
                  <span className="text-white text-sm font-semibold">
                    Scanning {scanProgress.current}/{scanProgress.total}
                  </span>
                </motion.div>
              )}
            </AnimatePresence>

            {/* Floating Controls for Cloud View */}
            <AnimatePresence>
              {view === 'cloud' && (
                <motion.div
                  initial={{ opacity: 0, y: -15 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -15 }}
                  transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
                  className="fixed top-12 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-4"
                >
                  {/* Sub-tabs for Movies/TV */}
                  <div className="flex p-0.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/10 shadow-md">
                    <motion.button
                      onClick={() => setCloudSubTab('movies')}
                      whileTap={{ scale: 0.95 }}
                      className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${cloudSubTab === 'movies'
                        ? 'bg-white text-black shadow-md'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      <Film className="size-3.5" />
                      <span>Movies</span>
                    </motion.button>
                    <motion.button
                      onClick={() => setCloudSubTab('tv')}
                      whileTap={{ scale: 0.95 }}
                      className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${cloudSubTab === 'tv'
                        ? 'bg-white text-black shadow-md'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      <Tv className="size-3.5" />
                      <span>TV Shows</span>
                    </motion.button>
                    <motion.button
                      onClick={() => setCloudSubTab('collections')}
                      whileTap={{ scale: 0.95 }}
                      className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${cloudSubTab === 'collections'
                        ? 'bg-white text-black shadow-md'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      <FolderOpen className="size-3.5" />
                      <span>Collections</span>
                    </motion.button>
                  </div>

                  {/* Search Input */}
                  <div className="relative flex items-center bg-card/90 backdrop-blur-xl border border-white/10 rounded-lg shadow-md overflow-hidden">
                    <Search className="size-3.5 text-muted-foreground ml-2.5" />
                    <input
                      type="text"
                      aria-label={`Search ${cloudSubTab === 'movies' ? 'movies' : 'TV shows'}`}
                      placeholder={`Search ${cloudSubTab === 'movies' ? 'movies' : 'TV shows'}...`}
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                      className="w-32 bg-transparent border-none text-xs px-2 py-1.5 focus:outline-none text-white placeholder:text-muted-foreground/60 font-medium"
                    />
                    {searchQuery && (
                      <button
                        type="button"
                        onClick={() => setSearchQuery('')}
                        className="p-1 hover:bg-white/10 rounded-full transition-colors mr-1.5"
                      >
                        <X className="size-3 text-muted-foreground" />
                      </button>
                    )}
                  </div>

                  {/* View Mode Toggle */}
                  <div className="relative flex p-[3px] rounded-xl bg-card/90 backdrop-blur-xl border border-white/10 shadow-md">
                    {/* Sliding indicator */}
                    <motion.div
                      className="absolute top-[3px] bottom-[3px] rounded-[9px] bg-white/15 border border-white/20 shadow-sm"
                      animate={{
                        left: viewMode === 'grid' ? '3px' : '50%',
                        width: 'calc(50% - 3px)',
                      }}
                      transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                    />
                    <motion.button
                      onClick={() => setViewMode('grid')}
                      whileTap={{ scale: 0.95 }}
                      className={`relative z-10 flex items-center gap-1.5 px-3 py-1.5 rounded-[9px] text-xs font-medium transition-colors duration-200 ${viewMode === 'grid'
                        ? 'text-white'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      <LayoutGrid className="size-3.5" />
                      <span>Grid</span>
                    </motion.button>
                    <motion.button
                      onClick={() => setViewMode('list')}
                      whileTap={{ scale: 0.95 }}
                      className={`relative z-10 flex items-center gap-1.5 px-3 py-1.5 rounded-[9px] text-xs font-medium transition-colors duration-200 ${viewMode === 'list'
                        ? 'text-white'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      <List className="size-3.5" />
                      <span>List</span>
                    </motion.button>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>

            {/* Content - Episodes and AI chat have their own fixed layout/scroll behavior */}
            {view === 'episodes' && selectedShow ? (
              <div className="flex-1 overflow-hidden px-3 pb-3 pt-12">
                <AnimatePresence mode="wait">
                  <motion.div
                    key="episodes"
                    initial={{ opacity: 0, x: 20 }}
                    animate={{ opacity: 1, x: 0 }}
                    exit={{ opacity: 0, x: -20 }}
                    className="h-full"
                  >
                    <Suspense fallback={<LoadingFallback />}>
                      <EpisodeBrowser
                        show={selectedShow}
                        onBack={() => {
                          // Navigate back to cloud view (all shows are cloud-based now)
                          setView('cloud')
                          setCloudSubTab('tv')
                          setSelectedShow(null)
                        }}
                        onWatchTogether={betaEnabled ? handleWatchTogether : undefined}
                        onDownload={handleStartDownload}
                      />
                    </Suspense>
                  </motion.div>
                </AnimatePresence>
              </div>
            ) : view === 'reminders' ? (
              <div className="flex-1 overflow-hidden">
                <div className="h-full min-h-0">
                  <AnimatePresence mode="wait">
                    <motion.div
                      key="reminders"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="h-full"
                    >
                      <RemindersView />
                    </motion.div>
                  </AnimatePresence>
                </div>
              </div>
            ) : view === 'remote' ? (
              <div className="flex-1 overflow-hidden">
                <div className="h-full min-h-0">
                  <AnimatePresence mode="wait">
                    <motion.div
                      key="remote"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="h-full"
                    >
                      <Suspense fallback={<LoadingFallback />}>
                        <RemoteSourceView />
                      </Suspense>
                    </motion.div>
                  </AnimatePresence>
                </div>
              </div>
            ) : view === 'calendar' ? (
              <div className="flex-1 overflow-hidden">
                <div className="h-full min-h-0">
                  <AnimatePresence mode="wait">
                    <motion.div
                      key="calendar"
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="h-full"
                    >
                      <Suspense fallback={<LoadingFallback />}>
                        <CalendarView />
                      </Suspense>
                    </motion.div>
                  </AnimatePresence>
                </div>
              </div>
            ) : (
              <ScrollArea className="flex-1">
                <div className={`content-container ${view === 'home' ? '!py-0' : ''}`}>
                  <AnimatePresence mode="wait">
                    {/* Home View */}
                    {view === 'home' && (
                      <motion.div
                        key="home"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="h-[calc(100vh-80px)] flex flex-col overflow-hidden relative px-8"
                      >
                        {/* Background Decorative Layer */}
                        <div className="absolute inset-0 bg-gradient-mesh opacity-20 pointer-events-none" />
                        <div className="absolute inset-0 bg-sheen opacity-10 pointer-events-none" />
                        <div className="absolute right-6 top-16 z-20">
                          <button
                            type="button"
                            onClick={handleOpenNotificationCenter}
                            className="group relative flex size-12 items-center justify-center rounded-2xl bg-gradient-to-br from-white/10 to-transparent border border-white/10 backdrop-blur-xl shadow-2xl transition-all duration-500 hover:scale-105 active:scale-95"
                          >
                            <div className="absolute inset-0 rounded-2xl bg-white/0 group-hover:bg-white/5 transition-colors duration-500" />
                            <Bell className="relative z-10 size-5 text-white/40 group-hover:text-white transition-colors duration-300" />
                            {unreadNotificationCount > 0 && (
                              <span className="absolute -top-0.5 -right-0.5 z-20 flex size-4">
                                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-white/40 opacity-75"></span>
                                <span className="relative inline-flex rounded-full size-4 bg-white items-center justify-center text-[8px] font-black text-black">
                                  {unreadNotificationCount > 99 ? '99+' : unreadNotificationCount}
                                </span>
                              </span>
                            )}
                          </button>
                        </div>

                        <div className="relative flex-1 flex flex-col min-h-0">
                          {/* 1. Header Row: Clock + Branding + Date */}
                           <div className="pt-16 pb-6 flex flex-col items-center justify-center flex-shrink-0 w-full gap-2 relative">
                            <Clock />
                          </div>

                          {/* 2. Centered Sleek Search Bar */}
                          <div className="flex justify-center px-6 pb-12 flex-shrink-0 w-full">
                            <div className="relative group w-full max-w-xl">
                              <div className="relative flex items-center bg-white/[0.03] hover:bg-white/[0.06] border border-white/10 rounded-full transition-all duration-500 group-focus-within:border-white/40 group-focus-within:bg-white/[0.08] group-focus-within:shadow-glow-sm overflow-hidden">
                                <Search className="size-5 text-white/30 ml-6 group-focus-within:text-white/60 transition-colors" />
                                <input
                                  ref={searchInputRef}
                                  type="text"
                                  aria-label="Search in your library"
                                  className="flex-1 bg-transparent border-none text-base p-4 focus:outline-none text-white placeholder:text-white/20 font-medium tracking-tight"
                                  placeholder="Search in your library..."
                                  value={homeSearchQuery}
                                  onChange={(e) => setHomeSearchQuery(e.target.value)}
                                />
                                {homeSearchQuery && (
                                  <button
                                    type="button"
                                    onClick={() => setHomeSearchQuery('')}
                                    className="p-2 hover:bg-white/10 rounded-full transition-colors mr-3"
                                  >
                                    <X className="size-4 text-white/40" />
                                  </button>
                                )}
                                {!homeSearchQuery && (
                                  <div className="mr-6 flex items-center gap-2 opacity-20 group-focus-within:opacity-0 transition-opacity">
                                    <span className="text-[10px] font-bold text-white tracking-[0.2em]">CTRL F</span>
                                  </div>
                                )}
                              </div>
                            </div>
                          </div>

                          {/* 3. Main Content Sections - FIXED HEIGHT / NO SCROLL */}
                          <div className="flex-1 flex flex-col justify-between px-0 pb-8 min-h-0">
                            {homeSearchQuery ? (
                              <section className="flex-1 min-h-0 overflow-hidden animate-in fade-in slide-in-from-bottom-4 duration-500">
                                <div className="flex items-center gap-3 mb-4">
                                  <div className="p-2 rounded-xl bg-white/10">
                                    <Search className="size-5 text-white" />
                                  </div>
                                  <div>
                                    <h3 className="text-xl font-bold text-white tracking-tight">
                                      {isHomeSearching ? 'Searching...' : `Search Results (${homeSearchResults.length})`}
                                    </h3>
                                    <p className="text-xs text-muted-foreground">Showing matches from your library</p>
                                  </div>
                                </div>

                                {homeSearchResults.length > 0 ? (
                                  <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8 gap-5 overflow-y-auto max-h-full pb-4 no-scrollbar">
                                    {sortPinnedFirst(homeSearchResults).slice(0, 16).map((item, index) => (
                                      <MovieCard
                                        key={item.id}
                                        item={item}
                                        index={index}
                                        onClick={handleItemClick}
                                        onFixMatch={handleFixMatch}
                                        onDownload={handleStartDownload}
                                        onDelete={handleDelete}
                                        onWatchTogether={betaEnabled ? handleWatchTogether : undefined}
                                      />
                                    ))}
                                  </div>
                                ) : !isHomeSearching && (
                                  <div className="text-center py-12 glass-light rounded-3xl border border-white/5 shadow-2xl">
                                    <div className="mb-4 inline-flex p-4 rounded-full bg-white/5">
                                      <Search className="size-8 text-muted-foreground/40" />
                                    </div>
                                    <h4 className="text-lg font-bold text-white mb-2">No matches found</h4>
                                    <p className="text-sm text-muted-foreground max-w-xs mx-auto">We couldn't find anything matching "{homeSearchQuery}" in your collection.</p>
                                  </div>
                                )}
                              </section>
                            ) : (
                              <div className="flex-1 flex flex-col justify-around gap-4 min-h-0">
                                {/* Continue Watching - Horizontal Cards */}
                                {continueWatching.length > 0 && (
                                  <motion.section
                                    initial={{ opacity: 0, y: 20 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    transition={{ delay: 0.1 }}
                                    className="flex-shrink-0"
                                  >
                                    <div className="flex items-center justify-between mb-5 px-1">
                                      <div className="flex items-center gap-4">
                                        <div className="w-1 h-4 bg-white/20 rounded-full" />
                                        <h3 className="text-[11px] font-black text-white/50 uppercase tracking-[0.3em]">Continue Watching</h3>
                                      </div>
                                      <button
                                        type="button"
                                        onClick={() => setView('history')}
                                        className="text-[10px] font-bold text-white/20 hover:text-white uppercase tracking-widest transition-colors flex items-center gap-2 group"
                                      >
                                        View All
                                        <ChevronRight className="size-3 opacity-50 group-hover:translate-x-1 transition-transform" />
                                      </button>
                                    </div>
                                    <div className="flex gap-4 pb-1 overflow-x-auto no-scrollbar">
                                      {continueWatching.slice(0, 3).map((item, index) => (
                                        <ContinueCard
                                          key={item.id}
                                          item={item}
                                          index={index}
                                          onClick={handleItemClick}
                                        />
                                      ))}
                                    </div>
                                  </motion.section>
                                )}

                                {/* Newly Added - Single Row Grid */}
                                {recentlyAdded.length > 0 && (
                                  <motion.section
                                    initial={{ opacity: 0, y: 20 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    transition={{ delay: 0.2 }}
                                    className="min-h-0 overflow-hidden"
                                  >
                                    <div className="flex items-center mb-5 px-1">
                                      <div className="flex items-center gap-4">
                                        <div className="w-1 h-4 bg-white/20 rounded-full" />
                                        <h3 className="text-[11px] font-black text-white/50 uppercase tracking-[0.3em]">Newly Added</h3>
                                      </div>
                                    </div>
                                    <div className="grid grid-cols-4 sm:grid-cols-5 md:grid-cols-6 lg:grid-cols-7 xl:grid-cols-8 gap-5">
                                      {/* One row of larger items */}
                                      {sortPinnedFirst(recentlyAdded).slice(0, 8).map((item, index) => (
                                        <motion.div
                                          key={item.id}
                                          initial={{ opacity: 0, scale: 0.9 }}
                                          animate={{ opacity: 1, scale: 1, y: 0 }}
                                          transition={{ delay: index * 0.04 }}
                                          className="aspect-[2/3] rounded-[1.4rem]"
                                        >
                                          <MovieCard
                                            item={item}
                                            index={index}
                                            onClick={handleItemClick}
                                            onFixMatch={handleFixMatch}
                                            onDownload={handleStartDownload}
                                            onDelete={handleDelete}
                                            onWatchTogether={betaEnabled ? handleWatchTogether : undefined}
                                            showNewBadge={false}
                                            className="h-full"
                                          />
                                        </motion.div>
                                      ))}
                                    </div>
                                  </motion.section>
                                )}

                                {/* Stats bar */}
                                {tabVisibility.showCloud && (libraryStats.movies > 0 || libraryStats.shows > 0) && (
                                  <motion.div
                                    initial={{ opacity: 0, y: 10 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    transition={{ delay: 0.3 }}
                                    className="flex items-center justify-center gap-5 py-3 flex-shrink-0"
                                  >
                                    <button
                                      type="button"
                                      onClick={() => { setView('cloud'); setCloudSubTab('movies'); }}
                                      className="flex items-center gap-2 text-[15px] text-white/40 hover:text-white transition-colors"
                                    >
                                      <Film className="size-4" />
                                      <span className="font-bold tabular-nums">{libraryStats.movies}</span>
                                      <span className="text-white/25">Movies</span>
                                    </button>
                                    <span className="w-px h-5 bg-white/10" />
                                    <button
                                      type="button"
                                      onClick={() => { setView('cloud'); setCloudSubTab('tv'); }}
                                      className="flex items-center gap-2 text-[15px] text-white/40 hover:text-white transition-colors"
                                    >
                                      <Tv className="size-4" />
                                      <span className="font-bold tabular-nums">{libraryStats.shows}</span>
                                      <span className="text-white/25">Shows</span>
                                    </button>
                                    <span className="w-px h-5 bg-white/10" />
                                    <button
                                      type="button"
                                      onClick={() => setView('history')}
                                      className="flex items-center gap-2 text-[15px] text-white/40 hover:text-white transition-colors"
                                    >
                                      <TrendingUp className="size-4" />
                                      <span className="font-bold tabular-nums">{continueWatching.length}</span>
                                      <span className="text-white/25">Watching</span>
                                    </button>
                                  </motion.div>
                                )}

                                {/* Empty state */}
                                {continueWatching.length === 0 && libraryStats.movies === 0 && libraryStats.shows === 0 && (
                                  <motion.div
                                    className="flex flex-col items-center justify-center flex-1 py-16"
                                    initial={{ opacity: 0, y: 12 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    transition={{ duration: 0.5, ease: [0.16, 1, 0.3, 1] }}
                                  >
                                    {/* Subtle radial glow */}
                                    <div className="absolute w-[400px] h-[200px] bg-white/[0.02] rounded-full blur-[80px] pointer-events-none" />

                                    {/* Icon */}
                                    <div className="mb-8 p-6 rounded-[28px] bg-white/[0.03] border border-white/[0.06]">
                                      <Film className="size-14 text-white/30" strokeWidth={1.5} />
                                    </div>

                                    {/* Copy */}
                                    <h3 className="text-2xl font-bold text-white tracking-tight mb-2">Your library is waiting</h3>
                                    <p className="text-sm text-white/20 mb-8 max-w-xs">
                                      Connect Google Drive to start building your collection.
                                    </p>

                                    {/* CTA */}
                                    <button
                                      type="button"
                                      onClick={() => startTransition(() => setSettingsOpen(true))}
                                      className="btn-primary py-3 px-8 rounded-xl inline-flex items-center gap-2.5 text-sm font-semibold shadow-glow-sm hover:shadow-glow transition-all"
                                    >
                                      <Sparkles className="size-4 text-black" />
                                      <span>Start Your Collection</span>
                                    </button>
                                  </motion.div>
                                )}
                              </div>
                            )}
                          </div>
                        </div>
                      </motion.div>
                    )}




                    {/* History & Analytics View */}
                    {(view === 'history' || view === 'analytics') && (
                      <motion.div
                        key="history"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                      >
                      <Suspense fallback={<LoadingFallback />}>
                        <FullHistoryView
                          analyticsData={analyticsData}
                          onAnalyticsTabActive={loadAnalytics}
                        />
                      </Suspense>
                      </motion.div>
                    )}

                    {view === 'downloads' && (
                      <motion.div
                        key="downloads"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                      >
                        <DownloadsView
                          jobs={downloadJobs}
                          onCancel={handleCancelDownload}
                          onOpen={handleOpenDownload}
                          onClearHistory={handleClearDownloadHistory}
                          onDeleteJob={handleDeleteDownload}
                        />
                      </motion.div>
                    )}

                    {/* Direct Links View */}
                    {view === 'directlinks' && (
                      <motion.div
                        key="directlinks"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="relative h-[calc(100vh-80px)]"
                      >
                        {/* Background Decorative Layer - Matching Home View Aesthetic */}
                        <div className="absolute inset-0 bg-gradient-mesh opacity-20 pointer-events-none" />
                        <div className="absolute inset-0 bg-sheen opacity-10 pointer-events-none" />
                      <Suspense fallback={<LoadingFallback />}>
                        <DirectLinksView
                          onIndexComplete={handleDdlIndexComplete}
                          viewMode={viewMode}
                          onItemClick={handleItemClick}
                          onFixMatch={handleFixMatch}
                          onDownload={handleStartDownload}
                          onDelete={handleDelete}
                          onWatchTogether={betaEnabled ? handleWatchTogether : undefined}
                        />
                      </Suspense>
                      </motion.div>
                    )}

                    {/* Cloud Media Grid */}
                    {view === 'cloud' && cloudSubTab === 'collections' && (
                      <motion.div
                        key="cloud-collections"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                      >
                        <SmartCollections
                          onItemClick={handleItemClick}
                          onFixMatch={handleFixMatch}
                          onDownload={handleStartDownload}
                          onDelete={handleDelete}
                          viewMode={viewMode}
                        />
                      </motion.div>
                    )}

                    {view === 'cloud' && cloudSubTab !== 'collections' && (
                      <motion.div
                        key={`cloud-${cloudSubTab}`}
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="pt-24"
                      >
                        <div className={viewMode === 'grid' ? 'grid-media' : 'list-media'}>
                          {sortPinnedFirst(cloudItemsToRender).map((item, index) => (
                            <MovieCard
                              key={item.id}
                              item={item}
                              index={index}
                              layout={viewMode}
                              onClick={handleItemClick}
                              onFixMatch={handleFixMatch}
                              onDownload={handleStartDownload}
                              onDelete={handleDelete}
                              onWatchTogether={betaEnabled ? handleWatchTogether : undefined}
                            />
                          ))}
                          {isChunkedCloudRender && cloudItemsToRender.length < sortedItems.length && (
                            <div
                              ref={cloudLoadMoreRef}
                              className={viewMode === 'grid'
                                ? 'col-span-full h-16 flex items-center justify-center text-xs text-muted-foreground/70'
                                : 'h-16 flex items-center justify-center text-xs text-muted-foreground/70'}
                            >
                              Loading more…
                            </div>
                          )}
                          {sortedItems.length === 0 && (
                            <div className="col-span-full flex items-center justify-center min-h-[60vh]">
                              <motion.div
                                className="empty-state-enhanced flex flex-col items-center text-center"
                                initial={{ opacity: 0, scale: 0.9 }}
                                animate={{ opacity: 1, scale: 1 }}
                              >
                                {/* Cloud Indexing Progress - Shows when indexing */}
                                {view === 'cloud' && isCloudIndexing ? (
                                    <motion.div
                                      initial={{ opacity: 0, y: 10 }}
                                      animate={{ opacity: 1, y: 0 }}
                                      className="flex flex-col items-center w-full max-w-xl"
                                    >
                                      <div className="relative mb-8">
                                        {/* Animated rings */}
                                        <motion.div
                                          className="absolute inset-0 rounded-full border-2 border-gray-500/30"
                                          animate={{ scale: [1, 1.5, 1.5], opacity: [0.5, 0, 0] }}
                                          transition={{ duration: 2, repeat: Infinity, ease: "easeOut" }}
                                          style={{ width: 96, height: 96 }}
                                        />
                                        <motion.div
                                          className="absolute inset-0 rounded-full border-2 border-gray-500/30"
                                          animate={{ scale: [1, 1.5, 1.5], opacity: [0.5, 0, 0] }}
                                          transition={{ duration: 2, repeat: Infinity, ease: "easeOut", delay: 0.5 }}
                                          style={{ width: 96, height: 96 }}
                                        />
                                        {/* Center icon */}
                                        <div className="size-24 rounded-full bg-gradient-to-br from-gray-500/20 to-gray-400/20 border border-gray-500/30 flex items-center justify-center">
                                          <motion.div
                                            animate={cloudIndexingStatus.includes('complete') ? {} : { rotate: 360 }}
                                            transition={{ duration: 2, repeat: Infinity, ease: "linear" }}
                                          >
                                            <Cloud className={`size-12 ${cloudIndexingStatus.includes('complete') ? 'text-white' : 'text-gray-400'}`} />
                                          </motion.div>
                                        </div>
                                      </div>

                                      {/* Status Title */}
                                      <h3 className="text-2xl font-semibold text-foreground mb-2">
                                        {cloudIndexingStatus.includes('complete') ? '✓ Indexing Complete!' : cloudIndexingStatus || 'Indexing your cloud files...'}
                                      </h3>

                                    {/* Current Folder */}
                                    {cloudIndexingProgress && cloudIndexingProgress.currentFolderName && !cloudIndexingStatus.includes('complete') && (
                                      <p className="text-gray-400 text-sm font-medium mb-3">
                                        📁 {cloudIndexingProgress.currentFolderName}
                                      </p>
                                    )}

                                    {/* Stats Cards */}
                                    {cloudIndexingProgress && (
                                      <div className="flex items-center gap-4 mb-4">
                                        <div className="flex flex-col items-center px-4 py-2 rounded-lg bg-card/50 border border-border/50">
                                          <span className="text-2xl font-bold text-foreground">{cloudIndexingProgress.filesFound}</span>
                                          <span className="text-xs text-muted-foreground">Files Found</span>
                                        </div>
                                        <div className="flex flex-col items-center px-4 py-2 rounded-lg bg-card/50 border border-border/50">
                                          <span className="text-2xl font-bold text-white">{cloudIndexingProgress.moviesFound}</span>
                                          <span className="text-xs text-muted-foreground">Movies</span>
                                        </div>
                                        <div className="flex flex-col items-center px-4 py-2 rounded-lg bg-card/50 border border-border/50">
                                          <span className="text-2xl font-bold text-white">{cloudIndexingProgress.tvFound}</span>
                                          <span className="text-xs text-muted-foreground">TV Shows</span>
                                        </div>
                                      </div>
                                    )}

                                    {/* Progress bar with folder count */}
                                    {cloudIndexingProgress && (
                                      <div className="w-full max-w-xs">
                                        <div className="flex justify-between text-xs text-muted-foreground mb-1.5">
                                          <span>Folder {cloudIndexingProgress.currentFolder} of {cloudIndexingProgress.totalFolders}</span>
                                          <span>{Math.round((cloudIndexingProgress.currentFolder / cloudIndexingProgress.totalFolders) * 100)}%</span>
                                        </div>
                                        <div className="w-full h-2 bg-muted/30 rounded-full overflow-hidden">
                                          <motion.div
                                            className={`h-full rounded-full ${cloudIndexingStatus.includes('complete') ? 'bg-gradient-to-r from-gray-500 to-gray-400' : 'bg-gradient-to-r from-gray-500 to-gray-400'}`}
                                            initial={{ width: "0%" }}
                                            animate={{ width: `${(cloudIndexingProgress.currentFolder / cloudIndexingProgress.totalFolders) * 100}%` }}
                                            transition={{ duration: 0.3 }}
                                          />
                                        </div>
                                      </div>
                                    )}
                                  </motion.div>
                                ) : (
                                  <>
                                    <div className="mb-8">
                                      <div className="size-24 rounded-full bg-gradient-to-br from-gray-500/10 to-gray-400/10 border border-gray-500/20 flex items-center justify-center mx-auto">
                                        <Cloud className="size-12 text-muted-foreground/60" />
                                      </div>
                                    </div>
                                    <h3 className="text-2xl font-semibold text-foreground mb-3 text-center">
                                      {`No cloud ${(cloudSubTab === 'movies' ? 'movies' : 'TV shows')} found`}
                                    </h3>
                                    <p className="text-muted-foreground text-lg max-w-md mb-8 text-center mx-auto leading-relaxed">
                                      {isGDriveConnected
                                        ? 'Click Update Library to scan your Google Drive for movies and TV shows'
                                        : 'Connect your Google Drive account to stream your cloud media'
                                      }
                                    </p>
                                    <div className="flex items-center gap-3">
                                      {isGDriveConnected ? (
                                        <button
                                          type="button"
                                          onClick={handleCloudScan}
                                          disabled={isScanning || isCloudIndexing}
                                          className="btn-primary inline-flex items-center gap-3 px-8 py-4 text-base rounded-2xl"
                                        >
                                          <RefreshCw className={`size-5 ${isCloudIndexing ? 'animate-spin' : ''}`} />
                                          {isCloudIndexing ? 'Updating...' : 'Update Library'}
                                        </button>
                                      ) : (
                                        <button
                                          type="button"
                                          onClick={() => startTransition(() => {
                                            setSettingsInitialTab('cloud')
                                            setSettingsOpen(true)
                                          })}
                                          className="btn-primary inline-flex items-center gap-3 px-8 py-4 text-base rounded-2xl"
                                        >
                                          <Sparkles className="size-5" />
                                          {view === 'cloud' ? 'Setup Google Drive' : 'Add Media Folders'}
                                        </button>
                                      )}
                                    </div>
                                  </>
                                )}
                              </motion.div>
                            </div>
                          )}
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>
                </div>
              </ScrollArea>
            )}
          </main>
          </>
          )}

          <Suspense fallback={null}>
            <SettingsModal
              open={settingsOpen}
              onOpenChange={(open) => {
                setSettingsOpen(open)
                if (!open) {
                  setSettingsInitialTab('general')
                }
              }}
              initialTab={settingsInitialTab}
              tabVisibility={tabVisibility}
              onTabVisibilityChange={handleTabVisibilityChange}
              onLogout={handleLogout}
            betaEnabled={betaEnabled}
            onBetaToggle={handleBetaToggle}
          />
          </Suspense>

          <Suspense fallback={null}>
            <FixMatchModal
              open={fixMatchOpen}
              onOpenChange={setFixMatchOpen}
              item={itemToFix}
              onSuccess={handleFixMatchSuccess}
            />
          </Suspense>

          <Suspense fallback={null}>
            <SyncValidatorModal
              isOpen={showSyncValidator}
              onClose={() => setShowSyncValidator(false)}
            />
          </Suspense>

          <Suspense fallback={null}>
            <DuplicateDetector
              isOpen={showDuplicateDetector}
              onClose={() => setShowDuplicateDetector(false)}
            />
          </Suspense>

          <ContentDetailsModal
            open={contentDetailsOpen}
            onOpenChange={handleContentDetailsOpenChange}
            item={contentDetailsItem}
            onPrimaryAction={handleDetailsPrimaryAction}
            onDownloadAction={handleStartDownload}
            downloadActionLabel="Download"
            onEpisodeSecondaryAction={handleDetailsMarkWatched}
            episodeSecondaryActionLabel="Mark as watched"
            onEpisodeUnwatchAction={handleDetailsUnwatch}
            onMetadataRefresh={handleContentDetailsMetadataRefresh}
          />

          {resumeDialogData && (
            <ResumeDialog
              open={resumeDialogOpen}
              onOpenChange={setResumeDialogOpen}
              title={displayTitleFor(resumeDialogData.item)}
              mediaType={resumeDialogData.item.media_type}
              seasonEpisode={
                resumeDialogData.item.season_number !== undefined && resumeDialogData.item.episode_number !== undefined
                  ? `S${String(resumeDialogData.item.season_number).padStart(2, '0')}E${String(resumeDialogData.item.episode_number).padStart(2, '0')}`
                  : undefined
              }
              currentPosition={resumeDialogData.resumeInfo.position}
              duration={resumeDialogData.resumeInfo.duration}
              posterUrl={resumeDialogData.posterUrl}
              onResume={() => handleResumeChoice(true)}
              onStartOver={() => handleResumeChoice(false)}
            />
          )}

          {playConfirmData && (
            <PlayConfirmDialog
              open={playConfirmOpen}
              onOpenChange={(open) => { setPlayConfirmOpen(open); if (!open) setPlayConfirmData(null) }}
              title={displayTitleFor(playConfirmData)}
              mediaType={playConfirmData.media_type}
              seasonEpisode={
                playConfirmData.season_number !== undefined && playConfirmData.episode_number !== undefined
                  ? `S${String(playConfirmData.season_number).padStart(2, '0')}E${String(playConfirmData.episode_number).padStart(2, '0')}`
                  : undefined
              }
              onConfirm={handlePlayConfirm}
            />
          )}

          {deleteModalData && (
            <DeleteEpisodesModal
              isOpen={deleteModalOpen}
              onClose={() => { setDeleteModalOpen(false); setDeleteModalData(null) }}
              seriesId={deleteModalData.seriesId}
              seriesTitle={deleteModalData.seriesTitle}
              onDeleteComplete={handleDeleteComplete}
            />
          )}

          {/* Expired DDL link dialog */}
          <Dialog open={ddlExpiredDialogOpen} onOpenChange={(open) => { if (!open && !ddlExpiredRefreshing) { setDdlExpiredDialogOpen(false); setDdlExpiredSourceId(null); setDdlExpiredItem(null); setDdlExpiredNewUrl(''); setDdlExpiredError('') } }}>
            <DialogContent className="sm:max-w-lg">
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <RefreshCw className="size-5" />
                  Link Expired
                </DialogTitle>
                <DialogDescription>
                  The direct download link for this content has expired. Provide a fresh URL for the exact same archive to restore streaming.
                </DialogDescription>
              </DialogHeader>

              <div className="space-y-4">
                <div className="flex items-start gap-3 p-4 rounded-lg bg-amber-500/5 border border-amber-500/20">
                  <AlertCircle className="size-5 text-amber-400 flex-shrink-0 mt-0.5" />
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    The previous link has expired. Please provide a fresh URL for the <span className="text-foreground font-medium">exact same archive</span>.
                  </p>
                </div>

                <div className="space-y-2">
                  <label htmlFor="ddl-expired-url" className="text-xs font-medium text-muted-foreground">New Session URL</label>
                  <Input
                    id="ddl-expired-url"
                    type="url"
                    placeholder="https://server.com/new_session_url.zip"
                    value={ddlExpiredNewUrl}
                    onChange={e => setDdlExpiredNewUrl(e.target.value)}
                    onKeyDown={e => { if (e.key === 'Enter') handleDdlRefreshAndRetry() }}
                    disabled={ddlExpiredRefreshing}
                    autoFocus
                  />
                </div>

                {ddlExpiredError && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: "auto", opacity: 1 }}
                    className="p-3 bg-destructive/10 border border-destructive/20 rounded-lg text-xs text-destructive"
                  >
                    {ddlExpiredError}
                  </motion.div>
                )}
              </div>

              <DialogFooter className="gap-2">
                <Button variant="ghost" onClick={() => { setDdlExpiredDialogOpen(false); setDdlExpiredSourceId(null); setDdlExpiredItem(null); setDdlExpiredNewUrl(''); setDdlExpiredError('') }} disabled={ddlExpiredRefreshing}>
                  Skip
                </Button>
                <Button
                  onClick={handleDdlRefreshAndRetry}
                  disabled={!ddlExpiredNewUrl.trim() || ddlExpiredRefreshing}
                  className="min-w-[160px]"
                >
                  {ddlExpiredRefreshing && <Loader2 className="size-4 mr-2 animate-spin" />}
                  Refresh & Retry
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>

          {/* Mark Complete Dialog */}
          {markCompleteData && (
            <MarkCompleteDialog
              open={markCompleteDialogOpen}
              onOpenChange={setMarkCompleteDialogOpen}
              title={markCompleteData.title}
              seasonEpisode={markCompleteData.seasonEpisode}
              progressPercent={markCompleteData.progressPercent}
              onMarkComplete={handleMarkComplete}
              onKeepProgress={() => {
                setMarkCompleteData(null)
                toast({ title: "Progress Saved", description: `${markCompleteData.title} - ${markCompleteData.progressPercent.toFixed(0)}% watched` })
              }}
            />
          )}

          {/* Watch Together Modal */}
          <Suspense fallback={<div className="flex items-center justify-center h-full"><span className="text-zinc-400">Loading…</span></div>}>
            <WatchTogetherModal
              isOpen={watchTogetherOpen}
              onClose={() => {
                setWatchTogetherOpen(false)
                // Don't clear watchTogetherMedia if still in a room
                if (!wtActiveRoom) {
                  setWatchTogetherMedia(null)
                }
              }}
              selectedMedia={wtSessionMedia || watchTogetherMedia || undefined}
              activeRoom={wtActiveRoom}
              sessionId={wtSessionId}
              isPlaying={wtIsPlaying}
              onSessionChange={handleWtSessionChange}
            />
          </Suspense>

          {/* Watch Together Banner - shows when in a room but modal is closed */}
          {wtActiveRoom && !watchTogetherOpen && (
            <WatchTogetherBanner
              room={wtActiveRoom}
              isPlaying={wtIsPlaying}
              onOpenModal={() => startTransition(() => setWatchTogetherOpen(true))}
              onLeave={handleWtLeave}
            />
          )}

          <NotificationCenter
            open={notificationCenterOpen}
            onOpenChange={setNotificationCenterOpen}
            items={notifications}
            activeFilter={notificationFilter}
            onFilterChange={setNotificationFilter}
            onClearAll={clearNotifications}
          />

          <CommandPalette
            open={commandPaletteOpen}
            onClose={() => setCommandPaletteOpen(false)}
            setView={(v) => {
              setView(v)
              setSelectedShow(null)
              setSearchQuery('')
              setHomeSearchQuery('')
              setHomeSearchResults([])
            }}
            onOpenSettings={() => startTransition(() => setSettingsOpen(true))}
            onCloudScan={handleCloudScan}
            onSyncValidator={() => startTransition(() => setShowSyncValidator(true))}
            onToggleSidebarPin={handleToggleSidebarPin}
            continueWatching={continueWatching}
            onPlayItem={startPlaybackFlow}
          />

          <Toaster />

          {showTmdbKeyNotice && (
            <TmdbKeyNoticeBanner
              onOpenSettings={() => {
                setSettingsInitialTab("api")
                setSettingsOpen(true)
                setShowTmdbKeyNotice(false)
              }}
              onDismiss={() => {
                setTmdbKeyNoticeDismissed(true)
                setShowTmdbKeyNotice(false)
              }}
            />
          )}

          <ConfirmDialog
            open={confirmDeleteOpen}
            onOpenChange={setConfirmDeleteOpen}
            title="Delete Media"
            description={confirmDeleteDesc}
            confirmLabel="Delete"
            variant="destructive"
            onConfirm={() => confirmDeleteActionRef.current?.()}
          />

          {import.meta.env.DEV && <DeveloperConsole />}
        </>
      )}
    </div>
    </OptimizationContext.Provider>
  )
}

export default App
