import { useState, useEffect } from 'react'
import { listen, emit, UnlistenFn } from '@tauri-apps/api/event'
import { appWindow } from '@tauri-apps/api/window'
import { Sidebar } from '@/components/Sidebar'
import { MovieCard, ContinueCard } from '@/components/MovieCard'
import { EpisodeBrowser } from '@/components/EpisodeBrowser'
import { StreamView } from '@/components/StreamView'
import { SettingsModal } from '@/components/SettingsModal'
import { FixMatchModal } from '@/components/FixMatchModal'
import { PlayerModal } from '@/components/PlayerModal'
import { ResumeDialog } from '@/components/ResumeDialog'
import { DeleteEpisodesModal } from '@/components/DeleteEpisodesModal'
import { OnboardingModal } from '@/components/OnboardingModal'
import { MainAppTour } from '@/components/MainAppTour'
import { UpdateNotesModal, shouldShowUpdateNotes } from '@/components/UpdateNotesModal'
import { MarkCompleteDialog } from '@/components/MarkCompleteDialog'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Toaster } from '@/components/ui/toaster'
import {
  getLibraryFiltered,
  getWatchHistory,
  removeFromWatchHistory,
  clearAllWatchHistory,
  deleteMediaFiles,
  MediaItem,
  playMedia,
  getResumeInfo,
  ResumeInfo,
  getCachedImageUrl,
  StreamingHistoryItem,
  getStreamingHistory,
  removeFromStreamingHistory,
  clearAllStreamingHistory,
  openVideasyPlayer,
  hasCompletedOnboarding,
  completeOnboarding,
  getTabVisibility,
  setTabVisibility,
  TabVisibility,
  markAsComplete,
} from '@/services/api'
import { initAdBlocker } from '@/utils/adBlocker'
import {
  Search, Loader2, Trash2, Play, Film, Tv, Clock,
  ChevronRight, LayoutGrid, List,
  TrendingUp, BarChart3, Calendar, Sparkles, PlayCircle, Globe, X, Cloud, RefreshCw, Minus
} from 'lucide-react'
import { useToast } from '@/components/ui/use-toast'
import { motion, AnimatePresence } from 'framer-motion'

initAdBlocker()

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
  season_number?: number
  episode_number?: number
  media_type?: string
  final_position?: number
  final_duration?: number
  completed: boolean
}

type ViewMode = 'grid' | 'list'
type SortOption = 'title' | 'year' | 'recent' | 'progress'
type MediaSubTab = 'movies' | 'tv'

function App() {
  const [view, setView] = useState<string>('home')
  const [items, setItems] = useState<MediaItem[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedShow, setSelectedShow] = useState<MediaItem | null>(null)

  // Sub-tabs for Cloud view
  const [cloudSubTab, setCloudSubTab] = useState<MediaSubTab>('movies')

  // View mode and sort
  const [viewMode, setViewMode] = useState<ViewMode>('grid')
  const [sortBy] = useState<SortOption>('title')

  // Home search state
  const [homeSearchQuery, setHomeSearchQuery] = useState('')
  const [homeSearchResults, setHomeSearchResults] = useState<MediaItem[]>([])
  const [isHomeSearching, setIsHomeSearching] = useState(false)

  // Continue watching
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

  // Modals
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [settingsInitialTab, setSettingsInitialTab] = useState<'general' | 'cloud' | 'player' | 'api' | 'danger'>('general')
  const [fixMatchOpen, setFixMatchOpen] = useState(false)
  const [itemToFix, setItemToFix] = useState<MediaItem | null>(null)

  // Player selection
  const [playerModalOpen, setPlayerModalOpen] = useState(false)
  const [pendingPlayItem, setPendingPlayItem] = useState<MediaItem | null>(null)
  const [pendingResumeTime, setPendingResumeTime] = useState(0)

  const [theme] = useState<'dark' | 'light'>('dark')
  const { toast } = useToast()

  // Resume dialog state
  const [resumeDialogOpen, setResumeDialogOpen] = useState(false)
  const [resumeDialogData, setResumeDialogData] = useState<{
    item: MediaItem
    resumeInfo: ResumeInfo
    posterUrl?: string
  } | null>(null)

  // Delete modal state
  const [deleteModalOpen, setDeleteModalOpen] = useState(false)
  const [deleteModalData, setDeleteModalData] = useState<{
    seriesId: number
    seriesTitle: string
  } | null>(null)

  // History tab state
  const [historyTab, setHistoryTab] = useState<'local' | 'streaming'>('local')
  const [streamingHistoryItems, setStreamingHistoryItems] = useState<StreamingHistoryItem[]>([])

  // Streaming resume dialog state
  const [streamingResumeDialogOpen, setStreamingResumeDialogOpen] = useState(false)
  const [streamingResumeData, setStreamingResumeData] = useState<StreamingHistoryItem | null>(null)

  // Onboarding state
  const [showOnboarding, setShowOnboarding] = useState(false)
  const [showMainAppTour, setShowMainAppTour] = useState(false)

  // Update notes state
  const [showUpdateNotes, setShowUpdateNotes] = useState(false)

  // Tab visibility state - cloud-only mode
  const [tabVisibility, setTabVisibilityState] = useState<TabVisibility>({ showLocal: false, showCloud: true })

  // Cloud connection state for contextual empty states
  const [isGDriveConnected, setIsGDriveConnected] = useState(false)

  // Continue watching expanded state
  const [continueWatchingExpanded, setContinueWatchingExpanded] = useState(false)

  // Mark complete dialog state
  const [markCompleteDialogOpen, setMarkCompleteDialogOpen] = useState(false)
  const [markCompleteData, setMarkCompleteData] = useState<{
    mediaId: number
    title: string
    seasonEpisode?: string
    progressPercent: number
    isCompletionConfirmation?: boolean // True when MPV detected end chapter
  } | null>(null)

  // Check onboarding status and load tab visibility on mount
  useEffect(() => {
    if (!hasCompletedOnboarding()) {
      setShowOnboarding(true)
    } else if (shouldShowUpdateNotes()) {
      // Show update notes if onboarding is done and notes haven't been shown
      setTimeout(() => setShowUpdateNotes(true), 500)
    }
    // Load tab visibility settings
    setTabVisibilityState(getTabVisibility())
  }, [])

  const handleOnboardingComplete = () => {
    completeOnboarding()
    setShowOnboarding(false)
    // Start the main app tour after onboarding modal
    setTimeout(() => {
      setShowMainAppTour(true)
    }, 300)
  }

  const handleMainAppTourComplete = () => {
    setShowMainAppTour(false)
  }

  const handleMainAppTourSkip = () => {
    setShowMainAppTour(false)
  }

  const handleRestartOnboarding = () => {
    // Small delay to let Settings modal close first
    setTimeout(() => {
      setShowOnboarding(true)
    }, 300)
  }

  // Check GDrive connection status for contextual empty states
  const checkGDriveStatus = async () => {
    try {
      const { isGDriveConnected: checkConnected } = await import('@/services/gdrive')
      const connected = await checkConnected()
      setIsGDriveConnected(connected)
    } catch (error) {
      console.log('[GDrive] Status check failed:', error)
      setIsGDriveConnected(false)
    }
  }

  // Check GDrive status when switching to cloud view or on mount
  useEffect(() => {
    if (view === 'cloud') {
      checkGDriveStatus()
    }
  }, [view])

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
  useEffect(() => {
    let unlistenProgress: UnlistenFn | undefined
    let unlistenComplete: UnlistenFn | undefined
    let unlistenMpvEnded: UnlistenFn | undefined
    let unlistenLibraryUpdated: UnlistenFn | undefined
    let unlistenNotification: UnlistenFn | undefined
    let unlistenCloudIndexingStarted: UnlistenFn | undefined

    const setupListeners = async () => {
      unlistenProgress = await listen<ScanProgressPayload>('scan-progress', (event) => {
        const payload = event.payload
        setScanProgress({
          current: payload.current,
          total: payload.total,
          title: payload.title
        })
      })

      // Cloud indexing started
      unlistenCloudIndexingStarted = await listen<{ count: number }>('cloud-indexing-started', (event) => {
        setIsCloudIndexing(true)
        console.log(`[Cloud] Indexing started: ${event.payload.count} files`)
      })

      unlistenComplete = await listen<ScanCompletePayload>('scan-complete', async () => {
        setIsScanning(false)
        setScanProgress(null)
        // Refresh based on current view
        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          setItems(await getWatchHistory())
          setStreamingHistoryItems(await getStreamingHistory(50))
        }
        await loadLibraryStats()
        await loadContinueWatching()

        toast({ title: "Scan Complete", description: "Library has been updated." })
      })

      unlistenMpvEnded = await listen<MpvPlaybackEndedPayload>('mpv-playback-ended', async (event) => {
        const { media_id, title, season_number, episode_number, media_type, completed, final_position, final_duration } = event.payload

        // Build season/episode string for TV episodes
        const seasonEpisode = media_type === 'tvepisode' && season_number && episode_number
          ? `S${String(season_number).padStart(2, '0')}E${String(episode_number).padStart(2, '0')}`
          : undefined

        if (completed) {
          // MPV detected end chapter - ask user to confirm completion
          setMarkCompleteData({
            mediaId: media_id,
            title: title,
            seasonEpisode,
            progressPercent: 100,
            isCompletionConfirmation: true
          })
          setMarkCompleteDialogOpen(true)
        } else if (final_position && final_duration && final_position > 30) {
          const progressPercent = (final_position / final_duration) * 100

          // Show mark as complete dialog if progress is between 80-95%
          if (progressPercent >= 80 && progressPercent < 95) {
            setMarkCompleteData({
              mediaId: media_id,
              title: title,
              seasonEpisode,
              progressPercent: progressPercent,
              isCompletionConfirmation: false
            })
            setMarkCompleteDialogOpen(true)
          } else {
            const displayTitle = seasonEpisode ? `${title} (${seasonEpisode})` : title
            toast({ title: "Progress Saved", description: `${displayTitle} - ${progressPercent.toFixed(0)}% watched` })
          }
        }

        // Refresh based on current view - don't clear items for views that don't need refresh
        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          setItems(await getWatchHistory())
          setStreamingHistoryItems(await getStreamingHistory(50))
        }
        // Always refresh continue watching since progress changed
        await loadContinueWatching()
      })

      // Listen for real-time library updates from file watcher
      unlistenLibraryUpdated = await listen<{ type: string; title: string }>('library-updated', async (event) => {
        const { type, title } = event.payload
        console.log(`[WATCHER] Library updated: ${type} - ${title}`)

        // Stop cloud indexing indicator
        setIsCloudIndexing(false)

        // Refresh based on current view
        if (view === 'cloud') {
          await fetchData()
        } else if (view === 'history') {
          setItems(await getWatchHistory())
          setStreamingHistoryItems(await getStreamingHistory(50))
        }
        await loadLibraryStats()
        await loadContinueWatching()
      })

      // Listen for notification events from file watcher
      unlistenNotification = await listen<{ type: string; title: string; message: string }>('notification', (event) => {
        const { type, title, message } = event.payload
        toast({
          title,
          description: message,
          variant: type === 'success' ? 'default' : type === 'info' ? 'default' : 'destructive'
        })
      })
    }

    setupListeners()
    return () => {
      unlistenProgress?.()
      unlistenComplete?.()
      unlistenMpvEnded?.()
      unlistenLibraryUpdated?.()
      unlistenNotification?.()
      unlistenCloudIndexingStarted?.()
    }
  }, [view, searchQuery])

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

  // Load initial data
  useEffect(() => {
    loadContinueWatching()
    loadLibraryStats()
  }, [tabVisibility])

  // Cloud change detection is now handled by the Rust backend
  // The backend polls every 60 seconds and emits 'library-updated' events
  // which are already handled elsewhere in the app

  // Load library stats - cloud only
  const loadLibraryStats = async () => {
    try {
      const [cloudMovies, cloudShows] = await Promise.all([
        getLibraryFiltered('movie', '', true),
        getLibraryFiltered('tv', '', true)
      ])

      setLibraryStats({
        movies: cloudMovies.length,
        shows: cloudShows.length,
        episodes: 0
      })
    } catch (error) {
      console.error('Failed to load stats', error)
    }
  }

  // Load continue watching
  const loadContinueWatching = async () => {
    try {
      const history = await getWatchHistory()
      // Filter to items with progress < 95%
      const inProgress = history
        .filter(item => {
          const progress = item.progress_percent || (item.resume_position_seconds && item.duration_seconds
            ? (item.resume_position_seconds / item.duration_seconds) * 100
            : 0)
          return progress > 0 && progress < 95
        })
        .slice(0, 10)
      setContinueWatching(inProgress)
    } catch (error) {
      console.error('Failed to load continue watching', error)
    }
  }

  useEffect(() => {
    if (view !== 'episodes' && view !== 'home' && view !== 'stats' && view !== 'stream') {
      const delayDebounceFn = setTimeout(() => {
        fetchData()
      }, 300)
      return () => clearTimeout(delayDebounceFn)
    }
  }, [view, searchQuery, sortBy, cloudSubTab])

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
  }, [homeSearchQuery, view])

  const handleHomeSearch = async () => {
    if (!homeSearchQuery.trim()) {
      setHomeSearchResults([])
      return
    }

    setIsHomeSearching(true)
    try {
      // Search across all 4 entities: Local Movies, Local TV, Cloud Movies, Cloud TV
      const [localMovies, localTv, cloudMovies, cloudTv] = await Promise.all([
        getLibraryFiltered('movie', homeSearchQuery, false),
        getLibraryFiltered('tv', homeSearchQuery, false),
        getLibraryFiltered('movie', homeSearchQuery, true),
        getLibraryFiltered('tv', homeSearchQuery, true)
      ])

      const combined = [...localMovies, ...localTv, ...cloudMovies, ...cloudTv]
      const query = homeSearchQuery.toLowerCase()
      combined.sort((a, b) => {
        const aTitle = a.title.toLowerCase()
        const bTitle = b.title.toLowerCase()
        if (aTitle === query && bTitle !== query) return -1
        if (bTitle === query && aTitle !== query) return 1
        if (aTitle.startsWith(query) && !bTitle.startsWith(query)) return -1
        if (bTitle.startsWith(query) && !aTitle.startsWith(query)) return 1
        return aTitle.localeCompare(bTitle)
      })

      setHomeSearchResults(combined)
    } catch (error) {
      console.error("Failed to search", error)
    } finally {
      setIsHomeSearching(false)
    }
  }

  const fetchData = async () => {
    try {
      let data: MediaItem[] = []
      if (view === 'cloud') {
        // Cloud view - filter by is_cloud = true
        const mediaType = cloudSubTab === 'movies' ? 'movie' : 'tv'
        data = await getLibraryFiltered(mediaType, searchQuery, true)
      } else if (view === 'history') {
        data = await getWatchHistory()
        const streamingData = await getStreamingHistory(50)
        setStreamingHistoryItems(streamingData)
      }

      // Sort data
      if (sortBy === 'title') {
        data.sort((a, b) => a.title.localeCompare(b.title))
      } else if (sortBy === 'year') {
        data.sort((a, b) => (b.year || 0) - (a.year || 0))
      } else if (sortBy === 'recent') {
        data.sort((a, b) => new Date(b.last_watched || 0).getTime() - new Date(a.last_watched || 0).getTime())
      }

      setItems(data)
    } catch (error) {
      console.error("Failed to fetch data", error)
    }
  }

  // Handle cloud-only indexing - scans the entire Google Drive for NEW files only
  const handleCloudScan = async () => {
    if (isScanning || isCloudIndexing) {
      toast({ title: "Update In Progress", description: "Library update is already running." })
      return
    }

    try {
      const { isGDriveConnected: checkConnected, scanCloudFolder } = await import('@/services/gdrive')
      const connected = await checkConnected()

      if (!connected) {
        toast({
          title: "Not Connected",
          description: "Connect to Google Drive in Settings first"
        })
        return
      }

      setIsCloudIndexing(true)
      setCloudIndexingStatus('Checking for new files...')

      // Scan the root folder which will recursively scan all subfolders
      // The backend automatically skips files that are already indexed
      const result = await scanCloudFolder('root', 'My Drive')

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
        setCloudIndexingStatus('✓ Library is up to date')
        toast({
          title: "Library Up to Date",
          description: "No new movies or TV shows found in your Drive"
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

  const handleItemClick = async (item: MediaItem) => {
    if (item.media_type === 'tvshow') {
      setSelectedShow(item)
      setView('episodes')
    } else {
      try {
        const resumeInfo = await getResumeInfo(item.id)

        if (resumeInfo.has_progress && resumeInfo.progress_percent < 95) {
          let posterUrl: string | undefined
          if (item.poster_path) {
            try {
              posterUrl = await getCachedImageUrl(item.poster_path.replace('image_cache/', '')) || undefined
            } catch { }
          }

          setResumeDialogData({ item, resumeInfo, posterUrl })
          setResumeDialogOpen(true)
        } else {
          // Show player selection modal
          setPendingPlayItem(item)
          setPendingResumeTime(0)
          setPlayerModalOpen(true)
        }
      } catch (e) {
        toast({ title: "Error", description: "Failed to start playback", variant: "destructive" })
      }
    }
  }

  const handleResumeChoice = async (resume: boolean) => {
    if (!resumeDialogData) return
    const { item, resumeInfo } = resumeDialogData
    const resumeTime = resume ? resumeInfo.position : 0
    // Show player selection modal
    setPendingPlayItem(item)
    setPendingResumeTime(resumeTime)
    setResumeDialogOpen(false)
    setPlayerModalOpen(true)
  }

  const handlePlayerSelect = async (player: 'mpv' | 'vlc' | 'builtin' | 'stream') => {
    if (!pendingPlayItem) return

    // Only MPV is supported now
    if (player === 'mpv') {
      try {
        await playMedia(pendingPlayItem.id, pendingResumeTime > 0)
        toast({ title: "Playing", description: `Now playing: ${pendingPlayItem.title}` })
      } catch (e) {
        console.error('[MPV] Playback error:', e)
        toast({ title: "Error", description: String(e) || "Failed to start playback", variant: "destructive" })
      }
    }

    setPendingPlayItem(null)
    setPendingResumeTime(0)
  }

  const handleFixMatch = (item: MediaItem) => {
    setItemToFix(item)
    setFixMatchOpen(true)
  }

  const handleRemoveFromHistory = async (item: MediaItem) => {
    try {
      await removeFromWatchHistory(item.id)
      toast({ title: "Removed", description: `"${item.title}" removed from watch history.` })
      await fetchData()
      await loadContinueWatching()
    } catch {
      toast({ title: "Error", description: "Failed to remove from history", variant: "destructive" })
    }
  }

  const handleClearAllHistory = async () => {
    if (!confirm("Are you sure you want to clear all watch history?")) return
    try {
      await clearAllWatchHistory()
      toast({ title: "Cleared", description: "All watch history has been cleared." })
      await fetchData()
      await loadContinueWatching()
    } catch {
      toast({ title: "Error", description: "Failed to clear watch history", variant: "destructive" })
    }
  }

  const handleRemoveFromStreamingHistory = async (item: StreamingHistoryItem) => {
    try {
      await removeFromStreamingHistory(item.id)
      toast({ title: "Removed", description: `"${item.title}" removed from streaming history.` })
      await fetchData()
    } catch {
      toast({ title: "Error", description: "Failed to remove from streaming history", variant: "destructive" })
    }
  }

  const handleClearAllStreamingHistory = async () => {
    if (!confirm("Are you sure you want to clear all streaming history?")) return
    try {
      await clearAllStreamingHistory()
      toast({ title: "Cleared", description: "All streaming history has been cleared." })
      await fetchData()
    } catch {
      toast({ title: "Error", description: "Failed to clear streaming history", variant: "destructive" })
    }
  }

  const handleStreamingItemClick = async (item: StreamingHistoryItem) => {
    setStreamingResumeData(item)
    setStreamingResumeDialogOpen(true)
  }

  const openStreamingContent = async (item: StreamingHistoryItem) => {
    const VIDEASY_PLAYER_BASE = 'https://player.videasy.net'
    const STREAMVAULT_COLOR = '8B5CF6'

    let url: string
    let displayTitle = item.title

    if (item.media_type === 'movie') {
      url = `${VIDEASY_PLAYER_BASE}/movie/${item.tmdb_id}?overlay=true&color=${STREAMVAULT_COLOR}`
    } else {
      const season = item.season || 1
      const episode = item.episode || 1
      displayTitle = `${item.title} S${String(season).padStart(2, '0')}E${String(episode).padStart(2, '0')}`
      url = `${VIDEASY_PLAYER_BASE}/tv/${item.tmdb_id}/${season}/${episode}?nextEpisode=true&autoplayNextEpisode=true&episodeSelector=true&overlay=true&color=${STREAMVAULT_COLOR}`
    }

    // Extract poster path from full URL
    const posterPath = item.poster_path?.includes('/t/p/')
      ? item.poster_path.split('/t/p/')[1]?.replace('w342', '').replace('w300', '')
      : undefined

    try {
      await openVideasyPlayer(
        url,
        item.tmdb_id,
        item.media_type as 'movie' | 'tv',
        displayTitle,
        posterPath,
        item.season || undefined,
        item.episode || undefined
      )
      toast({ title: "Opening in Browser", description: `Streaming "${displayTitle}" in your default browser` })
    } catch (error) {
      toast({ title: "Failed to Open Player", description: "Could not open the streaming player", variant: "destructive" })
    }
  }

  const handleStreamingResumeChoice = async (_resume: boolean) => {
    if (streamingResumeData) {
      await openStreamingContent(streamingResumeData)
      setStreamingResumeDialogOpen(false)
      setStreamingResumeData(null)
    }
  }

  const handleDelete = async (item: MediaItem) => {
    if (item.media_type === 'tvshow') {
      setDeleteModalData({ seriesId: item.id, seriesTitle: item.title })
      setDeleteModalOpen(true)
    } else {
      const confirmed = confirm(`Are you sure you want to permanently delete "${item.title}"?`)
      if (confirmed) {
        try {
          const result = await deleteMediaFiles([item.id])
          if (result.success) {
            toast({ title: "Deleted", description: result.message })
            await fetchData()
          } else {
            toast({ title: "Partial Delete", description: result.message, variant: "destructive" })
            await fetchData()
          }
        } catch (error) {
          toast({ title: "Error", description: "Failed to delete file", variant: "destructive" })
        }
      }
    }
  }

  const handleDeleteComplete = async () => {
    await fetchData()
    toast({ title: "Deleted", description: "Selected episodes have been permanently deleted." })
  }

  const handleMarkComplete = async () => {
    if (!markCompleteData) return
    try {
      await markAsComplete(markCompleteData.mediaId)
      toast({ title: "Marked Complete", description: `${markCompleteData.title} marked as watched` })
      // Emit event so EpisodeBrowser and other components can refresh
      await emit('media-marked-complete', { media_id: markCompleteData.mediaId })
      await loadContinueWatching()
      // Refresh library items to update progress display on cards
      await fetchData()
      if (view === 'history') {
        setItems(await getWatchHistory())
      }
    } catch (error) {
      toast({ title: "Error", description: "Failed to mark as complete", variant: "destructive" })
    }
  }

  const toggleTheme = () => {
    toast({ title: "Theme Locked", description: "Dark mode is optimized for this interface." })
  }

  return (
    <div className="flex h-screen bg-background text-foreground overflow-hidden bg-gradient-mesh">
      {/* Custom Title Bar */}
      <div
        onMouseDown={(e) => {
          if (e.buttons === 1 && e.target === e.currentTarget) {
            appWindow.startDragging()
          }
        }}
        className="fixed top-0 left-0 right-0 h-8 z-[200] flex items-center justify-between bg-background/80 backdrop-blur-sm border-b border-white/5"
      >
        <div
          onMouseDown={() => appWindow.startDragging()}
          className="flex-1 h-full flex items-center pl-3 cursor-default"
        >
          <span className="text-xs font-medium text-muted-foreground select-none pointer-events-none">StreamVault</span>
        </div>
        <div className="flex items-center h-full">
          <button
            onClick={() => appWindow.minimize()}
            className="h-full px-4 hover:bg-white/10 transition-colors flex items-center justify-center"
          >
            <Minus className="w-3.5 h-3.5 text-muted-foreground" />
          </button>
          <button
            onClick={async () => {
              await appWindow.hide()
            }}
            className="h-full px-4 hover:bg-red-500/80 transition-colors flex items-center justify-center group"
          >
            <X className="w-3.5 h-3.5 text-muted-foreground group-hover:text-white" />
          </button>
        </div>
      </div>
      {/* Background decorative orbs */}
      <div className="fixed inset-0 pointer-events-none overflow-hidden z-0">
        <div className="bg-orb bg-orb-1" />
        <div className="bg-orb bg-orb-2" />
        <div className="bg-orb bg-orb-3" />
      </div>

      <Sidebar
        currentView={view === 'episodes' ? 'cloud' : view}
        setView={(v) => {
          setView(v)
          setSelectedShow(null)
          setSearchQuery('')
          setHomeSearchQuery('')
          setHomeSearchResults([])
        }}
        onOpenSettings={() => setSettingsOpen(true)}
        onCloudScan={handleCloudScan}
        theme={theme}
        toggleTheme={toggleTheme}
        isScanning={isScanning}
        isCloudIndexing={isCloudIndexing}
        scanProgress={scanProgress}
        showCloudTab={tabVisibility.showCloud}
        className="flex-shrink-0 z-50 sticky top-0 pt-8"
      />

      <main className="flex-1 flex flex-col min-w-0 relative z-10 overflow-hidden">
        {/* Floating Scan Progress Indicator */}
        <AnimatePresence>
          {isScanning && scanProgress && (
            <motion.div
              initial={{ opacity: 0, y: -20, scale: 0.9 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -20, scale: 0.9 }}
              className="fixed top-4 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-3 px-4 py-2.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/30 shadow-lg"
            >
              <div className="relative">
                <Loader2 className="h-4 w-4 animate-spin text-white" />
                <div className="absolute inset-0 rounded-full bg-white/40 blur-md animate-pulse" />
              </div>
              <span className="text-white text-sm font-semibold">
                Scanning {scanProgress.current}/{scanProgress.total}
              </span>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Floating Cloud Indexing Indicator */}
        <AnimatePresence>
          {isCloudIndexing && !isScanning && view !== 'cloud' && (
            <motion.div
              initial={{ opacity: 0, y: -20, scale: 0.9 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -20, scale: 0.9 }}
              className="fixed top-4 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-3 px-4 py-2.5 rounded-2xl bg-card/95 backdrop-blur-xl border border-gray-500/30 shadow-glow"
            >
              <div className="relative">
                <motion.div
                  animate={{ rotate: 360 }}
                  transition={{ duration: 2, repeat: Infinity, ease: "linear" }}
                >
                  <Cloud className="h-4 w-4 text-gray-400" />
                </motion.div>
                <div className="absolute inset-0 rounded-full bg-gray-400/40 blur-md animate-pulse" />
              </div>
              <div className="flex flex-col">
                <span className="text-gray-400 text-sm font-semibold">
                  {cloudIndexingProgress
                    ? `Scanning folder ${cloudIndexingProgress.currentFolder}/${cloudIndexingProgress.totalFolders}`
                    : 'Indexing cloud files...'
                  }
                </span>
                {cloudIndexingProgress && cloudIndexingProgress.filesFound > 0 && (
                  <span className="text-xs text-muted-foreground">
                    Found {cloudIndexingProgress.filesFound} files ({cloudIndexingProgress.moviesFound} movies, {cloudIndexingProgress.tvFound} TV)
                  </span>
                )}
              </div>
              {cloudIndexingProgress && (
                <div className="w-16 h-1.5 bg-muted/30 rounded-full overflow-hidden">
                  <motion.div
                    className="h-full bg-gradient-to-r from-gray-500 to-gray-400 rounded-full"
                    animate={{ width: `${(cloudIndexingProgress.currentFolder / cloudIndexingProgress.totalFolders) * 100}%` }}
                    transition={{ duration: 0.3 }}
                  />
                </div>
              )}
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
              className="fixed top-11 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-2"
            >
              {/* Sub-tabs for Movies/TV */}
              <div className="flex p-0.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/10 shadow-md">
                <motion.button
                  onClick={() => setCloudSubTab('movies')}
                  whileTap={{ scale: 0.95 }}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${
                    cloudSubTab === 'movies'
                      ? 'bg-white text-black shadow-md'
                      : 'text-muted-foreground hover:text-foreground'
                  }`}
                >
                  <Film className="w-3.5 h-3.5" />
                  <span>Movies</span>
                </motion.button>
                <motion.button
                  onClick={() => setCloudSubTab('tv')}
                  whileTap={{ scale: 0.95 }}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${
                    cloudSubTab === 'tv'
                      ? 'bg-white text-black shadow-md'
                      : 'text-muted-foreground hover:text-foreground'
                  }`}
                >
                  <Tv className="w-3.5 h-3.5" />
                  <span>TV Shows</span>
                </motion.button>
              </div>

              {/* Search Input */}
              <div className="relative flex items-center bg-card/90 backdrop-blur-xl border border-white/10 rounded-lg shadow-md overflow-hidden">
                <Search className="w-3.5 h-3.5 text-muted-foreground ml-2.5" />
                <input
                  type="text"
                  placeholder={`Search ${cloudSubTab === 'movies' ? 'movies' : 'TV shows'}...`}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="w-32 bg-transparent border-none text-xs px-2 py-1.5 focus:outline-none text-white placeholder:text-muted-foreground/60 font-medium"
                />
                {searchQuery && (
                  <button
                    onClick={() => setSearchQuery('')}
                    className="p-1 hover:bg-white/10 rounded-full transition-colors mr-1.5"
                  >
                    <X className="w-3 h-3 text-muted-foreground" />
                  </button>
                )}
              </div>

              {/* View Mode Toggle */}
              <div className="flex p-0.5 rounded-lg bg-card/90 backdrop-blur-xl border border-white/10 shadow-md">
                <motion.button
                  onClick={() => setViewMode('grid')}
                  whileTap={{ scale: 0.95 }}
                  className={`p-1.5 rounded-md transition-all duration-200 ${viewMode === 'grid'
                    ? 'bg-white/20 text-white'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <LayoutGrid className="w-3.5 h-3.5" />
                </motion.button>
                <motion.button
                  onClick={() => setViewMode('list')}
                  whileTap={{ scale: 0.95 }}
                  className={`p-1.5 rounded-md transition-all duration-200 ${viewMode === 'list'
                    ? 'bg-white/20 text-white'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <List className="w-3.5 h-3.5" />
                </motion.button>
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Floating History Tabs */}
        <AnimatePresence>
          {view === 'history' && (
            <motion.div
              initial={{ opacity: 0, y: -15 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -15 }}
              transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
              className="fixed top-11 left-1/2 -translate-x-1/2 z-[100] flex items-center gap-2"
            >
              {/* Tab Pills */}
              <div className="flex p-0.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/10 shadow-md">
                <motion.button
                  onClick={() => setHistoryTab('local')}
                  whileTap={{ scale: 0.95 }}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${historyTab === 'local'
                    ? 'bg-white text-black shadow-md'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <Film className="w-3.5 h-3.5" />
                  <span>Local</span>
                  <span className="text-[10px] opacity-70">({items.length})</span>
                </motion.button>
                <motion.button
                  onClick={() => setHistoryTab('streaming')}
                  whileTap={{ scale: 0.95 }}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-200 ${historyTab === 'streaming'
                    ? 'bg-white text-black shadow-md'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <Globe className="w-3.5 h-3.5" />
                  <span>Stream</span>
                  <span className="text-[10px] opacity-70">({streamingHistoryItems.length})</span>
                </motion.button>
              </div>

              {/* Clear Button */}
              {((historyTab === 'local' && items.length > 0) || (historyTab === 'streaming' && streamingHistoryItems.length > 0)) && (
                <motion.button
                  initial={{ opacity: 0, scale: 0.8 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.8 }}
                  onClick={historyTab === 'local' ? handleClearAllHistory : handleClearAllStreamingHistory}
                  whileHover={{ scale: 1.05 }}
                  whileTap={{ scale: 0.95 }}
                  className="p-1.5 rounded-full bg-card/90 backdrop-blur-xl border border-white/10 text-muted-foreground hover:text-destructive hover:border-destructive/30 shadow-md transition-colors"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </motion.button>
              )}
            </motion.div>
          )}
        </AnimatePresence>

        {/* Content - Episodes view has its own scroll, others use ScrollArea */}
        {view === 'episodes' && selectedShow ? (
          <div className="flex-1 overflow-hidden p-3">
            <AnimatePresence mode="wait">
              <motion.div
                key="episodes"
                initial={{ opacity: 0, x: 20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -20 }}
                className="h-full"
              >
                <EpisodeBrowser
                  show={selectedShow}
                  onBack={() => {
                    // Navigate back to cloud view (all shows are cloud-based now)
                    setView('cloud')
                    setCloudSubTab('tv')
                    setSelectedShow(null)
                  }}
                />
              </motion.div>
            </AnimatePresence>
          </div>
        ) : (
          <ScrollArea className="flex-1">
            <div className="content-container">
              <AnimatePresence mode="wait">
                {/* Home View */}
                {view === 'home' && (
                <motion.div
                  key="home"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="min-h-[calc(100vh-80px)] flex flex-col"
                >
                  {/* Search Results - Only show when searching */}
                  {homeSearchResults.length > 0 && (
                    <motion.section
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      className="mb-4"
                    >
                      <div className="section-header">
                        <h3 className="section-title">
                          <Search className="w-5 h-5 text-white" />
                          Results ({homeSearchResults.length})
                        </h3>
                      </div>
                      <div className="grid-media">
                        {homeSearchResults.map((item, index) => (
                          <MovieCard
                            key={item.id}
                            item={item}
                            index={index}
                            onClick={handleItemClick}
                            onFixMatch={handleFixMatch}
                            onDelete={handleDelete}
                          />
                        ))}
                      </div>
                    </motion.section>
                  )}

                  {/* Hero Search Section - Large and centered */}
                  {!homeSearchQuery && (
                    <div className="flex-1 flex items-center justify-center py-6">
                      <div className="w-full max-w-xl text-center">
                        <h2 className="text-3xl font-bold tracking-tight text-white mb-2">
                          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white via-white to-white/70">
                            Discover your next
                          </span>
                          {' '}
                          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white via-gray-300 to-gray-400">
                            favorite story
                          </span>
                        </h2>

                        <p className="text-sm text-muted-foreground mb-4">
                          Search across your library and streaming services
                        </p>

                        <div className="relative max-w-md mx-auto group">
                          <div className="relative flex items-center bg-card/80 backdrop-blur-xl border border-white/10 rounded-xl shadow-lg p-1.5 transition-all group-focus-within:border-white/50 group-focus-within:bg-card">
                            <Search className="w-5 h-5 text-muted-foreground ml-3" />
                            <input
                              type="text"
                              className="w-full bg-transparent border-none text-base px-3 py-2.5 focus:outline-none text-white placeholder:text-muted-foreground font-medium"
                              placeholder="Search movies, TV shows..."
                              value={homeSearchQuery}
                              onChange={(e) => setHomeSearchQuery(e.target.value)}
                              autoFocus
                            />
                            {homeSearchQuery && (
                              <button
                                onClick={() => setHomeSearchQuery('')}
                                className="p-1.5 hover:bg-white/10 rounded-full transition-colors mr-2"
                              >
                                <X className="w-4 h-4 text-muted-foreground" />
                              </button>
                            )}
                            {isHomeSearching && (
                              <div className="mr-3">
                                <Loader2 className="w-4 h-4 animate-spin text-white" />
                              </div>
                            )}
                          </div>
                        </div>

                        {/* Quick Actions */}
                        <div className="flex items-center justify-center gap-3 mt-5">
                          {tabVisibility.showCloud && (
                            <button
                              onClick={() => setView('cloud')}
                              className="flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 hover:bg-white/10 border border-white/5 text-sm font-medium transition-all hover:scale-105"
                            >
                              <Cloud className="w-4 h-4 text-gray-400" />
                              <span>Google Drive</span>
                            </button>
                          )}
                          <button
                            onClick={() => setView('stream')}
                            className="flex items-center gap-2 px-4 py-2 rounded-full bg-white/5 hover:bg-white/10 border border-white/5 text-sm font-medium transition-all hover:scale-105"
                          >
                            <Globe className="w-4 h-4 text-gray-400" />
                            <span>Browse Online</span>
                          </button>
                        </div>
                      </div>
                    </div>
                  )}

                  {/* Bottom Sections - Continue Watching and Library Stats */}
                  {!homeSearchQuery && (
                    <div className="space-y-4 pb-4 mt-auto">
                      {/* Continue Watching - Middle Bottom */}
                      {continueWatching.length > 0 && (
                        <motion.section
                          initial={{ opacity: 0, y: 15 }}
                          animate={{ opacity: 1, y: 0 }}
                          transition={{ delay: 0.1 }}
                        >
                          <div className="section-header-compact">
                            <div className="flex items-center gap-2">
                              <div className="p-1.5 rounded-lg bg-white/10">
                                <PlayCircle className="w-4 h-4 text-white" />
                              </div>
                              <div>
                                <h3 className="text-sm font-semibold text-foreground">Continue Watching</h3>
                                <p className="text-[10px] text-muted-foreground">Pick up where you left off</p>
                              </div>
                            </div>
                            <button
                              onClick={() => setView('history')}
                              className="btn-ghost text-xs flex items-center gap-1 group py-1 px-2"
                            >
                              View All
                              <ChevronRight className="w-3 h-3 transition-transform group-hover:translate-x-1" />
                            </button>
                          </div>
                          <div className="flex gap-3 pb-3">
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

                      {/* Library Stats - Bottom */}
                      {tabVisibility.showCloud && (
                        <motion.section
                          initial={{ opacity: 0, y: 15 }}
                          animate={{ opacity: 1, y: 0 }}
                          transition={{ delay: 0.2 }}
                        >
                          <div className="section-header-compact">
                            <div className="flex items-center gap-2">
                              <div className="p-1.5 rounded-lg bg-white/10">
                                <BarChart3 className="w-4 h-4 text-white" />
                              </div>
                              <div>
                                <h3 className="text-sm font-semibold text-foreground">Your Library</h3>
                                <p className="text-[10px] text-muted-foreground">At a glance</p>
                              </div>
                            </div>
                          </div>
                          <div className="grid grid-cols-3 gap-3">
                            {/* Movies Card */}
                            <motion.div
                              onClick={() => {
                                setView('cloud'); setCloudSubTab('movies');
                              }}
                              className="stat-card-compact group cursor-pointer"
                              whileHover={{ scale: 1.02 }}
                              whileTap={{ scale: 0.98 }}
                            >
                              <div className="flex items-center justify-between mb-2">
                                <div className="p-1.5 rounded-lg bg-white/10">
                                  <Film className="w-4 h-4 text-white" />
                                </div>
                                <ChevronRight className="w-4 h-4 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                              </div>
                              <div className="text-2xl font-bold text-foreground">{libraryStats.movies}</div>
                              <div className="text-[10px] text-muted-foreground">Movies</div>
                            </motion.div>

                            {/* TV Shows Card */}
                            <motion.div
                              onClick={() => {
                                setView('cloud'); setCloudSubTab('tv');
                              }}
                              className="stat-card-compact group cursor-pointer"
                              whileHover={{ scale: 1.02 }}
                              whileTap={{ scale: 0.98 }}
                            >
                              <div className="flex items-center justify-between mb-2">
                                <div className="p-1.5 rounded-lg bg-white/10">
                                  <Tv className="w-4 h-4 text-white" />
                                </div>
                                <ChevronRight className="w-4 h-4 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                              </div>
                              <div className="text-2xl font-bold text-foreground">{libraryStats.shows}</div>
                              <div className="text-[10px] text-muted-foreground">TV Shows</div>
                            </motion.div>

                            {/* In Progress Card */}
                            <motion.div
                              onClick={() => setView('history')}
                              className="stat-card-compact group cursor-pointer"
                              whileHover={{ scale: 1.02 }}
                              whileTap={{ scale: 0.98 }}
                            >
                              <div className="flex items-center justify-between mb-2">
                                <div className="p-1.5 rounded-lg bg-white/10">
                                  <Clock className="w-4 h-4 text-gray-400" />
                                </div>
                                <ChevronRight className="w-4 h-4 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                              </div>
                              <div className="text-2xl font-bold text-foreground">{continueWatching.length}</div>
                              <div className="text-[10px] text-muted-foreground">Watching</div>
                            </motion.div>
                          </div>
                        </motion.section>
                      )}

                      {/* Empty state - only when nothing to show */}
                      {continueWatching.length === 0 && libraryStats.movies === 0 && libraryStats.shows === 0 && (
                        <motion.div
                          className="flex flex-col items-center text-center py-6"
                          initial={{ opacity: 0, scale: 0.9 }}
                          animate={{ opacity: 1, scale: 1 }}
                        >
                          <div className="p-3 rounded-xl bg-white/5 mb-3">
                            <Film className="w-8 h-8 text-muted-foreground" />
                          </div>
                          <h3 className="text-base font-semibold text-foreground mb-1">Your library is empty</h3>
                          <p className="text-xs text-muted-foreground max-w-xs mb-4">
                            Connect Google Drive to discover your movies and TV shows
                          </p>
                          <button
                            onClick={() => setSettingsOpen(true)}
                            className="btn-primary-compact inline-flex items-center gap-1.5"
                          >
                            <Sparkles className="w-3.5 h-3.5" />
                            Get Started
                          </button>
                        </motion.div>
                      )}
                    </div>
                  )}
                </motion.div>
              )}

              {/* Statistics View */}
              {view === 'stats' && (
                <motion.div
                  key="stats"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="space-y-8"
                >
                  {/* Stats Header */}
                  <motion.div
                    className="text-center mb-8"
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                  >
                    <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-white/10 text-white text-sm font-medium mb-3">
                      <TrendingUp className="w-4 h-4" />
                      <span>Your Activity</span>
                    </div>
                    <h2 className="text-2xl font-bold text-foreground">Library Overview</h2>
                    <p className="text-muted-foreground mt-1">Track your watching progress</p>
                  </motion.div>

                  {/* Main Stats Grid */}
                  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
                    {/* Movies */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(0 0% 70%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.1 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(0 0% 70%)' } as React.CSSProperties}
                        >
                          <Film className="w-6 h-6 text-white" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{libraryStats.movies}</div>
                      <div className="text-sm text-muted-foreground">Total Movies</div>
                    </motion.div>

                    {/* TV Shows */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(0 0% 60%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.15 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(0 0% 60%)' } as React.CSSProperties}
                        >
                          <Tv className="w-6 h-6 text-white" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{libraryStats.shows}</div>
                      <div className="text-sm text-muted-foreground">Total TV Shows</div>
                    </motion.div>

                    {/* In Progress */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(0 0% 50%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.2 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(0 0% 50%)' } as React.CSSProperties}
                        >
                          <Clock className="w-6 h-6 text-gray-400" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{continueWatching.length}</div>
                      <div className="text-sm text-muted-foreground">In Progress</div>
                    </motion.div>

                    {/* Items Watched */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(0 0% 55%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.25 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(0 0% 55%)' } as React.CSSProperties}
                        >
                          <TrendingUp className="w-6 h-6 text-gray-400" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{items.length}</div>
                      <div className="text-sm text-muted-foreground">Items Watched</div>
                    </motion.div>
                  </div>

                  {/* Recent Activity */}
                  {continueWatching.length > 0 && (
                    <motion.section
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.3 }}
                    >
                      <div className="section-header">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-xl bg-white/10">
                            <Calendar className="w-5 h-5 text-white" />
                          </div>
                          <div>
                            <h3 className="text-lg font-semibold text-foreground">Recent Activity</h3>
                            <p className="text-xs text-muted-foreground">Your recent watches</p>
                          </div>
                        </div>
                      </div>
                      <div className="space-y-3">
                        {continueWatching.slice(0, 5).map((item, index) => (
                          <motion.div
                            key={item.id}
                            initial={{ opacity: 0, x: -20 }}
                            animate={{ opacity: 1, x: 0 }}
                            transition={{ delay: 0.3 + index * 0.05 }}
                            onClick={() => handleItemClick(item)}
                            className="list-item-interactive group"
                          >
                            <div className="w-14 h-20 rounded-lg bg-muted overflow-hidden flex-shrink-0">
                              {item.poster_path && (
                                <img
                                  src={item.poster_path}
                                  alt={item.title}
                                  className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-110"
                                />
                              )}
                            </div>
                            <div className="flex-1 min-w-0">
                              <h4 className="font-semibold text-foreground truncate group-hover:text-white transition-colors">
                                {item.title}
                              </h4>
                              <div className="flex items-center gap-3 mt-1">
                                <div className="flex-1 h-1.5 rounded-full bg-muted overflow-hidden max-w-32">
                                  <div
                                    className="h-full bg-white rounded-full"
                                    style={{ width: `${item.progress_percent || 0}%` }}
                                  />
                                </div>
                                <span className="text-sm text-muted-foreground">
                                  {Math.round(item.progress_percent || 0)}%
                                </span>
                              </div>
                            </div>
                            <div className="p-2 rounded-full bg-muted/50 opacity-0 group-hover:opacity-100 transition-opacity">
                              <Play className="w-5 h-5 text-white" />
                            </div>
                          </motion.div>
                        ))}
                      </div>
                    </motion.section>
                  )}

                  {/* Empty state for stats */}
                  {continueWatching.length === 0 && libraryStats.movies === 0 && libraryStats.shows === 0 && (
                    <motion.div
                      className="empty-state-enhanced flex flex-col items-center text-center min-h-[40vh] justify-center"
                      initial={{ opacity: 0, scale: 0.9 }}
                      animate={{ opacity: 1, scale: 1 }}
                    >
                      <div className="icon-wrapper mb-4">
                        <div className="icon-bg">
                          <BarChart3 className="w-10 h-10 text-muted-foreground" />
                        </div>
                      </div>
                      <h3 className="text-xl font-semibold text-foreground mb-2 text-center">No activity yet</h3>
                      <p className="text-muted-foreground max-w-sm text-center mx-auto">
                        Start watching content to see your statistics here
                      </p>
                    </motion.div>
                  )}
                </motion.div>
              )}

              {/* Stream View */}
              {view === 'stream' && (
                <motion.div
                  key="stream"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                >
                  <StreamView />
                </motion.div>
              )}

              {/* History View */}
              {view === 'history' && (
                <motion.div
                  key={`history-${historyTab}`}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="pt-14"
                >
                  {historyTab === 'local' ? (
                    <div className="grid-media">
                      {items.map((item, index) => (
                        <MovieCard
                          key={item.id}
                          item={item}
                          index={index}
                          onClick={handleItemClick}
                          onFixMatch={handleFixMatch}
                          onRemoveFromHistory={handleRemoveFromHistory}
                          onDelete={handleDelete}
                        />
                      ))}
                      {items.length === 0 && (
                        <div className="col-span-full flex items-center justify-center min-h-[60vh]">
                          <motion.div
                            className="empty-state-enhanced flex flex-col items-center text-center"
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                          >
                            <div className="icon-wrapper mb-4">
                              <div className="icon-bg">
                                <Film className="w-10 h-10 text-muted-foreground" />
                              </div>
                            </div>
                            <h3 className="text-xl font-semibold text-foreground mb-2 text-center">No local watch history</h3>
                            <p className="text-muted-foreground max-w-sm text-center mx-auto">
                              Start watching content from your library
                            </p>
                          </motion.div>
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="grid-media">
                      {streamingHistoryItems.map((item, index) => (
                        <motion.div
                          key={item.id}
                          initial={{ opacity: 0, y: 20 }}
                          animate={{ opacity: 1, y: 0 }}
                          transition={{ delay: index * 0.03 }}
                          onClick={() => handleStreamingItemClick(item)}
                          className="group relative overflow-hidden rounded-xl bg-card border border-border/50 cursor-pointer transition-all duration-300 hover:border-white/40 hover:shadow-lg"
                          style={{
                            transform: 'translateY(0)',
                          }}
                          whileHover={{ y: -6, scale: 1.02 }}
                        >
                          <div className="aspect-[2/3] relative overflow-hidden">
                            {item.poster_path ? (
                              <img
                                src={item.poster_path}
                                alt={item.title}
                                className="w-full h-full object-cover transition-transform duration-500 group-hover:scale-110"
                              />
                            ) : (
                              <div className="w-full h-full flex items-center justify-center bg-muted">
                                <Tv className="w-12 h-12 text-muted-foreground" />
                              </div>
                            )}

                            {/* Gradient Overlay */}
                            <div className="absolute inset-0 bg-gradient-to-t from-background via-background/30 to-transparent opacity-60 group-hover:opacity-100 transition-opacity" />

                            {/* Play Button */}
                            <motion.div
                              className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                            >
                              <div className="relative">
                                <div className="absolute inset-0 rounded-full bg-white/30 blur-xl scale-150" />
                                <div className="relative w-14 h-14 rounded-full bg-white flex items-center justify-center shadow-lg">
                                  <Play className="w-6 h-6 text-black fill-black ml-0.5" />
                                </div>
                              </div>
                            </motion.div>

                            {/* Progress Bar */}
                            {item.progress_percent > 0 && item.progress_percent < 95 && (
                              <div className="absolute bottom-0 left-0 right-0 h-1 bg-background/50">
                                <motion.div
                                  className="h-full bg-white"
                                  initial={{ width: 0 }}
                                  animate={{ width: `${item.progress_percent}%` }}
                                  transition={{ duration: 0.8, delay: 0.2 }}
                                />
                              </div>
                            )}

                            {/* Media Type Badge */}
                            <div className={`media-type-badge ${item.media_type}`}>
                              {item.media_type === 'movie' ? 'Movie' : 'TV'}
                            </div>
                          </div>
                          <div className="p-3">
                            <h4 className="font-medium text-sm truncate group-hover:text-white transition-colors">{item.title}</h4>
                            {item.media_type === 'tv' && item.season && item.episode && (
                              <p className="text-xs text-muted-foreground mt-0.5">
                                Season {item.season} · Episode {item.episode}
                              </p>
                            )}
                          </div>
                          <button
                            onClick={(e) => { e.stopPropagation(); handleRemoveFromStreamingHistory(item) }}
                            className="absolute top-2 right-2 p-2 rounded-full bg-background/80 backdrop-blur-sm text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-all"
                          >
                            <Trash2 className="w-4 h-4" />
                          </button>
                        </motion.div>
                      ))}
                      {streamingHistoryItems.length === 0 && (
                        <div className="col-span-full flex items-center justify-center min-h-[60vh]">
                          <motion.div
                            className="empty-state-enhanced flex flex-col items-center text-center"
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                          >
                            <div className="icon-wrapper mb-4">
                              <div className="icon-bg">
                                <Tv className="w-10 h-10 text-muted-foreground" />
                              </div>
                            </div>
                            <h3 className="text-xl font-semibold text-foreground mb-2 text-center">No streaming history</h3>
                            <p className="text-muted-foreground max-w-sm text-center mx-auto">
                              Stream content from the Stream tab
                            </p>
                          </motion.div>
                        </div>
                      )}
                    </div>
                  )}
                </motion.div>
              )}

              {/* Cloud Media Grid */}
              {view === 'cloud' && (
                <motion.div
                  key={`cloud-${cloudSubTab}`}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="pt-14"
                >
                  <div className={viewMode === 'grid' ? 'grid-media' : 'list-media'}>
                    {items.map((item, index) => (
                      <MovieCard
                        key={item.id}
                        item={item}
                        index={index}
                        onClick={handleItemClick}
                        onFixMatch={handleFixMatch}
                        onDelete={handleDelete}
                      />
                    ))}
                    {items.length === 0 && (
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
                              className="flex flex-col items-center w-full max-w-md"
                            >
                              <div className="relative mb-6">
                                {/* Animated rings */}
                                <motion.div
                                  className="absolute inset-0 rounded-full border-2 border-gray-500/30"
                                  animate={{ scale: [1, 1.5, 1.5], opacity: [0.5, 0, 0] }}
                                  transition={{ duration: 2, repeat: Infinity, ease: "easeOut" }}
                                  style={{ width: 80, height: 80 }}
                                />
                                <motion.div
                                  className="absolute inset-0 rounded-full border-2 border-gray-500/30"
                                  animate={{ scale: [1, 1.5, 1.5], opacity: [0.5, 0, 0] }}
                                  transition={{ duration: 2, repeat: Infinity, ease: "easeOut", delay: 0.5 }}
                                  style={{ width: 80, height: 80 }}
                                />
                                {/* Center icon */}
                                <div className="w-20 h-20 rounded-full bg-gradient-to-br from-gray-500/20 to-gray-400/20 border border-gray-500/30 flex items-center justify-center">
                                  <motion.div
                                    animate={cloudIndexingStatus.includes('complete') ? {} : { rotate: 360 }}
                                    transition={{ duration: 2, repeat: Infinity, ease: "linear" }}
                                  >
                                    <Cloud className={`w-8 h-8 ${cloudIndexingStatus.includes('complete') ? 'text-white' : 'text-gray-400'}`} />
                                  </motion.div>
                                </div>
                              </div>

                              {/* Status Title */}
                              <h3 className="text-xl font-semibold text-foreground mb-1">
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
                              <div className="icon-wrapper mb-4">
                                <div className="icon-bg">
                                  <Cloud className="w-10 h-10 text-muted-foreground" />
                                </div>
                              </div>
                              <h3 className="text-xl font-semibold text-foreground mb-2 text-center">
                                {`No cloud ${(cloudSubTab === 'movies' ? 'movies' : 'TV shows')} found`}
                              </h3>
                              <p className="text-muted-foreground max-w-sm mb-6 text-center mx-auto">
                                {isGDriveConnected
                                  ? 'Click Update Library to scan your Google Drive for movies and TV shows'
                                  : 'Connect your Google Drive account to stream your cloud media'
                                }
                              </p>
                              <div className="flex items-center gap-3">
                                {isGDriveConnected ? (
                                  <button
                                    onClick={handleCloudScan}
                                    disabled={isScanning || isCloudIndexing}
                                    className="btn-primary inline-flex items-center gap-2"
                                  >
                                    <RefreshCw className={`w-4 h-4 ${isCloudIndexing ? 'animate-spin' : ''}`} />
                                    {isCloudIndexing ? 'Updating...' : 'Update Library'}
                                  </button>
                                ) : (
                                  <button
                                    onClick={() => {
                                      setSettingsInitialTab('cloud')
                                      setSettingsOpen(true)
                                    }}
                                    className="btn-primary inline-flex items-center gap-2"
                                  >
                                    <Sparkles className="w-4 h-4" />
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

      {/* Modals */}
      <OnboardingModal
        open={showOnboarding}
        onComplete={handleOnboardingComplete}
      />

      {/* Main App Tour - shows after onboarding */}
      <MainAppTour
        isActive={showMainAppTour}
        onComplete={handleMainAppTourComplete}
        onSkip={handleMainAppTourSkip}
        setView={(v) => {
          setView(v)
          setSelectedShow(null)
          setSearchQuery('')
          setHomeSearchQuery('')
          setHomeSearchResults([])
        }}
      />

      <SettingsModal
        open={settingsOpen}
        onOpenChange={(open) => {
          setSettingsOpen(open)
          if (!open) setSettingsInitialTab('general')
        }}
        onRestartOnboarding={handleRestartOnboarding}
        onViewUpdateNotes={() => setShowUpdateNotes(true)}
        initialTab={settingsInitialTab}
        tabVisibility={tabVisibility}
        onTabVisibilityChange={handleTabVisibilityChange}
      />
      <FixMatchModal
        open={fixMatchOpen}
        onOpenChange={setFixMatchOpen}
        item={itemToFix}
        onSuccess={() => { fetchData(); loadLibraryStats() }}
      />
      <PlayerModal
        open={playerModalOpen}
        onOpenChange={setPlayerModalOpen}
        onSelectPlayer={handlePlayerSelect}
        title={pendingPlayItem?.title || ''}
        hasTmdbId={!!pendingPlayItem?.tmdb_id}
      />

      {resumeDialogData && (
        <ResumeDialog
          open={resumeDialogOpen}
          onOpenChange={setResumeDialogOpen}
          title={resumeDialogData.item.title}
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

      {streamingResumeData && (
        <ResumeDialog
          open={streamingResumeDialogOpen}
          onOpenChange={(open) => {
            setStreamingResumeDialogOpen(open)
            if (!open) setStreamingResumeData(null)
          }}
          title={streamingResumeData.title}
          mediaType={streamingResumeData.media_type === 'movie' ? 'movie' : 'tvepisode'}
          seasonEpisode={
            streamingResumeData.media_type === 'tv' && streamingResumeData.season && streamingResumeData.episode
              ? `S${String(streamingResumeData.season).padStart(2, '0')}E${String(streamingResumeData.episode).padStart(2, '0')}`
              : undefined
          }
          currentPosition={streamingResumeData.resume_position_seconds}
          duration={streamingResumeData.duration_seconds}
          posterUrl={streamingResumeData.poster_path || undefined}
          onResume={() => handleStreamingResumeChoice(true)}
          onStartOver={() => handleStreamingResumeChoice(false)}
          isStreaming={true}
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

      {/* Update Notes Modal */}
      <UpdateNotesModal
        open={showUpdateNotes}
        onOpenChange={setShowUpdateNotes}
      />

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
            toast({ title: "Progress Saved", description: `${markCompleteData.title} - ${markCompleteData.progressPercent.toFixed(0)}% watched` })
          }}
        />
      )}

      <Toaster />
    </div>
  )
}

export default App
