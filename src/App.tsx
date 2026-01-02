import { useState, useEffect } from 'react'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { Sidebar } from '@/components/Sidebar'
import { MovieCard, ContinueCard } from '@/components/MovieCard'
import { EpisodeBrowser } from '@/components/EpisodeBrowser'
import { StreamView } from '@/components/StreamView'
import { SettingsModal } from '@/components/SettingsModal'
import { FixMatchModal } from '@/components/FixMatchModal'
import { PlayerModal } from '@/components/PlayerModal'
import { VideoPlayer } from '@/components/VideoPlayer'
import { ResumeDialog } from '@/components/ResumeDialog'
import { DeleteEpisodesModal } from '@/components/DeleteEpisodesModal'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Toaster } from '@/components/ui/toaster'
import {
  getLibrary,
  getWatchHistory,
  removeFromWatchHistory,
  clearAllWatchHistory,
  deleteMediaFiles,
  MediaItem,
  playMedia,
  getResumeInfo,
  scanLibrary,
  updateWatchProgress,
  StreamInfo,
  ResumeInfo,
  getCachedImageUrl,
  StreamingHistoryItem,
  getStreamingHistory,
  removeFromStreamingHistory,
  clearAllStreamingHistory,
  openVideasyPlayer,
} from '@/services/api'
import { initAdBlocker } from '@/utils/adBlocker'
import {
  Search, Loader2, Trash2, Play, Film, Tv, Clock,
  ChevronRight, LayoutGrid, List,
  TrendingUp, BarChart3, Calendar, Sparkles, PlayCircle, Globe, X
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
  final_position?: number
  final_duration?: number
  completed: boolean
}

type ViewMode = 'grid' | 'list'
type SortOption = 'title' | 'year' | 'recent' | 'progress'

function App() {
  const [view, setView] = useState<string>('home')
  const [items, setItems] = useState<MediaItem[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedShow, setSelectedShow] = useState<MediaItem | null>(null)

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

  // Modals
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [fixMatchOpen, setFixMatchOpen] = useState(false)
  const [itemToFix, setItemToFix] = useState<MediaItem | null>(null)

  // Player selection
  const [playerModalOpen, setPlayerModalOpen] = useState(false)
  const [pendingPlayItem] = useState<MediaItem | null>(null)

  // Built-in player state
  const [isPlayerOpen, setIsPlayerOpen] = useState(false)
  const [currentStreamInfo, setCurrentStreamInfo] = useState<StreamInfo | null>(null)
  const [currentPlayingId, setCurrentPlayingId] = useState<number | null>(null)

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

  // Listen for Tauri events
  useEffect(() => {
    let unlistenProgress: UnlistenFn | undefined
    let unlistenComplete: UnlistenFn | undefined
    let unlistenMpvEnded: UnlistenFn | undefined

    const setupListeners = async () => {
      unlistenProgress = await listen<ScanProgressPayload>('scan-progress', (event) => {
        const payload = event.payload
        setScanProgress({
          current: payload.current,
          total: payload.total,
          title: payload.title
        })
      })

      unlistenComplete = await listen<ScanCompletePayload>('scan-complete', async () => {
        setIsScanning(false)
        setScanProgress(null)
        await fetchData()
        await loadLibraryStats()

        toast({ title: "Scan Complete", description: "Library has been updated." })
      })

      unlistenMpvEnded = await listen<MpvPlaybackEndedPayload>('mpv-playback-ended', async (event) => {
        const { title, completed, final_position, final_duration } = event.payload

        if (completed) {
          toast({ title: "Finished", description: `Finished watching: ${title}` })
        } else if (final_position && final_duration && final_position > 30) {
          const progressPercent = (final_position / final_duration) * 100
          toast({ title: "Progress Saved", description: `${title} - ${progressPercent.toFixed(0)}% watched` })
        }

        await fetchData()
        await loadContinueWatching()
      })
    }

    setupListeners()
    return () => {
      unlistenProgress?.()
      unlistenComplete?.()
      unlistenMpvEnded?.()
    }
  }, [])

  useEffect(() => {
    document.documentElement.classList.add('dark')
  }, [])

  // Load initial data
  useEffect(() => {
    loadContinueWatching()
    loadLibraryStats()
  }, [])

  // Load library stats
  const loadLibraryStats = async () => {
    try {
      const [movies, shows] = await Promise.all([
        getLibrary('movie'),
        getLibrary('tv')
      ])
      setLibraryStats({
        movies: movies.length,
        shows: shows.length,
        episodes: 0 // Would need a separate API call
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
    if (view !== 'episodes' && view !== 'home' && view !== 'stats') {
      const delayDebounceFn = setTimeout(() => {
        fetchData()
      }, 300)
      return () => clearTimeout(delayDebounceFn)
    }
  }, [view, searchQuery, sortBy])

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
      const [movies, tvShows] = await Promise.all([
        getLibrary('movie', homeSearchQuery),
        getLibrary('tv', homeSearchQuery)
      ])

      const combined = [...movies, ...tvShows]
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
      if (view === 'movies') {
        data = await getLibrary('movie', searchQuery)
      } else if (view === 'tv') {
        data = await getLibrary('tv', searchQuery)
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

  const handleScan = async () => {
    if (isScanning) {
      toast({ title: "Scan In Progress", description: "A scan is already running." })
      return
    }

    try {
      setIsScanning(true)
      setScanProgress(null)
      await scanLibrary()
      toast({ title: "Scan Started", description: "Library scan is running in the background." })
    } catch (error) {
      setIsScanning(false)
      toast({ title: "Error", description: "Failed to start scan", variant: "destructive" })
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
          await startPlayback(item, 0)
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
    await startPlayback(item, resumeTime)
  }

  const startPlayback = async (item: MediaItem, resumeTime: number) => {
    try {
      await playMedia(item.id, resumeTime > 0)
      toast({ title: "Playing", description: `Now playing: ${item.title}` })
    } catch (e) {
      toast({ title: "Error", description: "Failed to start playback", variant: "destructive" })
    }
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
    const SLASSHY_COLOR = '8B5CF6'

    let url: string
    let displayTitle = item.title

    if (item.media_type === 'movie') {
      url = `${VIDEASY_PLAYER_BASE}/movie/${item.tmdb_id}?overlay=true&color=${SLASSHY_COLOR}`
    } else {
      const season = item.season || 1
      const episode = item.episode || 1
      displayTitle = `${item.title} S${String(season).padStart(2, '0')}E${String(episode).padStart(2, '0')}`
      url = `${VIDEASY_PLAYER_BASE}/tv/${item.tmdb_id}/${season}/${episode}?nextEpisode=true&autoplayNextEpisode=true&episodeSelector=true&overlay=true&color=${SLASSHY_COLOR}`
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
      toast({ title: "Opening Player", description: `Now streaming "${displayTitle}"` })
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

  const toggleTheme = () => {
    toast({ title: "Theme Locked", description: "Dark mode is optimized for this interface." })
  }

  const renderViewTitle = () => {
    switch (view) {
      case 'home': return 'Home'
      case 'movies': return 'Movies'
      case 'tv': return 'TV Shows'
      case 'stream': return 'Stream'
      case 'history': return 'History'
      case 'stats': return 'Statistics'
      case 'episodes': return selectedShow?.title || 'Episodes'
      default: return ''
    }
  }

  return (
    <div className="flex h-screen bg-background text-foreground overflow-hidden bg-gradient-mesh">
      {/* Background decorative orbs */}
      <div className="fixed inset-0 pointer-events-none overflow-hidden z-0">
        <div className="bg-orb bg-orb-1" />
        <div className="bg-orb bg-orb-2" />
        <div className="bg-orb bg-orb-3" />
      </div>

      <Sidebar
        currentView={view === 'episodes' ? 'tv' : view}
        setView={(v) => {
          setView(v)
          setSelectedShow(null)
          setSearchQuery('')
          setHomeSearchQuery('')
          setHomeSearchResults([])
        }}
        onOpenSettings={() => setSettingsOpen(true)}
        onScan={handleScan}
        theme={theme}
        toggleTheme={toggleTheme}
        isScanning={isScanning}
        scanProgress={scanProgress}
        className="flex-shrink-0 z-50 sticky top-0"
      />

      <main className="flex-1 flex flex-col min-w-0 relative z-10 overflow-hidden">
        {/* Header */}
        <header className="px-8 py-5 flex items-center justify-between gap-4 sticky top-0 z-40">
          {/* Glassmorphism header background */}
          <div className="absolute inset-0 bg-background/70 backdrop-blur-2xl border-b border-white/[0.06] z-[-1]" />

          <div className="flex items-center gap-5">
            <motion.div
              key={view}
              initial={{ opacity: 0, x: -15 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
              className="flex items-center gap-4"
            >
              <h1 className="text-2xl font-bold text-foreground tracking-tight">
                {renderViewTitle()}
              </h1>
            </motion.div>

            {isScanning && scanProgress && (
              <motion.div
                initial={{ opacity: 0, scale: 0.9, x: -10 }}
                animate={{ opacity: 1, scale: 1, x: 0 }}
                className="flex items-center gap-2.5 px-4 py-2 rounded-xl bg-primary/10 border border-primary/25 shadow-glow-sm"
              >
                <div className="relative">
                  <Loader2 className="h-4 w-4 animate-spin text-primary" />
                  <div className="absolute inset-0 rounded-full bg-primary/40 blur-md animate-pulse" />
                </div>
                <span className="text-primary text-sm font-semibold">
                  Scanning {scanProgress.current}/{scanProgress.total}
                </span>
              </motion.div>
            )}
          </div>

          {/* Controls */}
          {(view === 'movies' || view === 'tv' || view === 'history') && (
            <motion.div
              initial={{ opacity: 0, x: 15 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ duration: 0.3 }}
              className="flex items-center gap-4"
            >
              {/* Search */}
              <div className="group relative w-80">
                <input
                  type="text"
                  placeholder="Search your library..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="w-full pl-11 pr-4 py-2.5 bg-card/60 backdrop-blur-sm border border-white/[0.08] rounded-xl text-sm font-medium focus:outline-none focus:bg-card/80 focus:border-primary/40 focus:shadow-glow-sm transition-all duration-300 placeholder:text-muted-foreground/60"
                />
                <Search className="absolute left-4 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground group-focus-within:text-primary transition-colors duration-200" />
              </div>

              {/* View Mode Toggle */}
              <div className="flex p-1.5 rounded-xl bg-card/60 backdrop-blur-sm border border-white/[0.08]">
                <motion.button
                  onClick={() => setViewMode('grid')}
                  whileTap={{ scale: 0.95 }}
                  className={`p-2.5 rounded-lg transition-all duration-200 ${viewMode === 'grid'
                    ? 'bg-primary/15 text-primary shadow-sm border border-primary/20'
                    : 'text-muted-foreground hover:text-foreground hover:bg-white/5'
                    }`}
                >
                  <LayoutGrid className="w-4 h-4" />
                </motion.button>
                <motion.button
                  onClick={() => setViewMode('list')}
                  whileTap={{ scale: 0.95 }}
                  className={`p-2.5 rounded-lg transition-all duration-200 ${viewMode === 'list'
                    ? 'bg-primary/15 text-primary shadow-sm border border-primary/20'
                    : 'text-muted-foreground hover:text-foreground hover:bg-white/5'
                    }`}
                >
                  <List className="w-4 h-4" />
                </motion.button>
              </div>
            </motion.div>
          )}

          {/* History tabs */}
          {view === 'history' && (
            <div className="flex items-center gap-3">
              <div className="flex p-1 rounded-xl bg-muted/30 border border-border/30">
                <button
                  onClick={() => setHistoryTab('local')}
                  className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 ${historyTab === 'local'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <Film className="w-4 h-4" />
                  Local ({items.length})
                </button>
                <button
                  onClick={() => setHistoryTab('streaming')}
                  className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 ${historyTab === 'streaming'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground'
                    }`}
                >
                  <Tv className="w-4 h-4" />
                  Streaming ({streamingHistoryItems.length})
                </button>
              </div>

              {((historyTab === 'local' && items.length > 0) || (historyTab === 'streaming' && streamingHistoryItems.length > 0)) && (
                <button
                  onClick={historyTab === 'local' ? handleClearAllHistory : handleClearAllStreamingHistory}
                  className="flex items-center gap-2 px-3 py-2 rounded-xl text-sm text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-all duration-200"
                >
                  <Trash2 className="w-4 h-4" />
                  <span>Clear</span>
                </button>
              )}
            </div>
          )}
        </header>

        {/* Content */}
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
                  className="space-y-8"
                >
                  {/* Hero Search Section - Minimal & Centered */}
                  <div className="fixed inset-0 pointer-events-none overflow-hidden -z-10 bg-sheen opacity-100" />

                  {/* Hero Search Section */}
                  <motion.div
                    className="relative z-10 w-full flex flex-col items-center justify-center py-6 md:py-10"
                    initial={{ opacity: 0, scale: 0.95 }}
                    animate={{ opacity: 1, scale: 1 }}
                    transition={{ duration: 0.5, ease: [0.22, 1, 0.36, 1] }}
                  >
                    <div className="relative z-10 w-full max-w-3xl mx-auto text-center px-4">
                      <motion.div
                        initial={{ opacity: 0, y: 20 }}
                        animate={{ opacity: 1, y: 0 }}
                        transition={{ delay: 0.1 }}
                      >
                        <h2 className="text-4xl md:text-5xl font-bold tracking-tight text-white mb-2 drop-shadow-2xl">
                          <span className="bg-clip-text text-transparent bg-gradient-to-r from-white via-white to-white/70">
                            Discover your next
                          </span>
                          <br />
                          <span className="bg-clip-text text-transparent bg-gradient-to-r from-primary via-violet-400 to-accent animate-gradient-x">
                            favorite story
                          </span>
                        </h2>
                      </motion.div>

                      <motion.p
                        className="text-base text-muted-foreground mb-6 max-w-lg mx-auto font-medium"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        transition={{ delay: 0.2 }}
                      >
                        Search across your entire local library and streaming services.
                      </motion.p>

                      <motion.div
                        className="relative max-w-xl mx-auto group"
                        initial={{ opacity: 0, y: 20 }}
                        animate={{ opacity: 1, y: 0 }}
                        transition={{ delay: 0.3 }}
                      >
                        {/* Glow effect */}
                        <div className="absolute -inset-1 bg-gradient-to-r from-primary to-accent rounded-2xl blur-md opacity-25 group-hover:opacity-50 transition duration-1000 group-hover:duration-200" />

                        <div className="relative flex items-center bg-card/80 backdrop-blur-xl border border-white/10 rounded-2xl shadow-2xl p-1.5 transition-all group-focus-within:border-primary/50 group-focus-within:bg-card">
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
                              <Loader2 className="w-4 h-4 animate-spin text-primary" />
                            </div>
                          )}
                        </div>
                      </motion.div>

                      {/* Quick Actions */}
                      <motion.div
                        className="flex items-center justify-center gap-3 mt-6"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        transition={{ delay: 0.4 }}
                      >
                        <button
                          onClick={() => setView('movies')}
                          className="flex items-center gap-2 px-5 py-2.5 rounded-full bg-white/5 hover:bg-white/10 border border-white/5 text-xs font-semibold transition-all hover:scale-105"
                        >
                          <Film className="w-3.5 h-3.5 text-violet-400" />
                          <span>Movies</span>
                        </button>
                        <button
                          onClick={() => setView('tv')}
                          className="flex items-center gap-2 px-5 py-2.5 rounded-full bg-white/5 hover:bg-white/10 border border-white/5 text-xs font-semibold transition-all hover:scale-105"
                        >
                          <Tv className="w-3.5 h-3.5 text-blue-400" />
                          <span>TV Shows</span>
                        </button>
                        <button
                          onClick={() => setView('stream')}
                          className="flex items-center gap-2 px-5 py-2.5 rounded-full bg-white/5 hover:bg-white/10 border border-white/5 text-xs font-semibold transition-all hover:scale-105"
                        >
                          <Globe className="w-3.5 h-3.5 text-cyan-400" />
                          <span>Browse Online</span>
                        </button>
                      </motion.div>
                    </div>
                  </motion.div>

                  {/* Search Results */}
                  {homeSearchResults.length > 0 && (
                    <motion.section
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                    >
                      <div className="section-header">
                        <h3 className="section-title">
                          <Search className="w-5 h-5 text-primary" />
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

                  {/* Continue Watching */}
                  {!homeSearchQuery && continueWatching.length > 0 && (
                    <motion.section
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.2 }}
                    >
                      <div className="section-header">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-xl bg-primary/10">
                            <PlayCircle className="w-5 h-5 text-primary" />
                          </div>
                          <div>
                            <h3 className="text-lg font-semibold text-foreground">Continue Watching</h3>
                            <p className="text-xs text-muted-foreground">Pick up where you left off</p>
                          </div>
                        </div>
                        <button
                          onClick={() => setView('history')}
                          className="btn-ghost text-sm flex items-center gap-1 group"
                        >
                          View All
                          <ChevronRight className="w-4 h-4 transition-transform group-hover:translate-x-1" />
                        </button>
                      </div>
                      <div className="flex gap-4 overflow-x-auto pb-4 -mx-2 px-2 scroll-fade">
                        {continueWatching.map((item, index) => (
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

                  {/* Library Stats */}
                  {!homeSearchQuery && (
                    <motion.section
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.3 }}
                    >
                      <div className="section-header">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-xl bg-accent/10">
                            <BarChart3 className="w-5 h-5 text-accent" />
                          </div>
                          <div>
                            <h3 className="text-lg font-semibold text-foreground">Your Library</h3>
                            <p className="text-xs text-muted-foreground">At a glance</p>
                          </div>
                        </div>
                      </div>
                      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
                        {/* Movies Card */}
                        <motion.div
                          onClick={() => setView('movies')}
                          className="stat-card-enhanced group"
                          style={{ '--stat-color': 'hsl(265 84% 62%)' } as React.CSSProperties}
                          whileHover={{ scale: 1.02 }}
                          whileTap={{ scale: 0.98 }}
                        >
                          <div className="flex items-start justify-between mb-4">
                            <div
                              className="stat-icon-wrapper"
                              style={{ '--icon-color': 'hsl(265 84% 62%)' } as React.CSSProperties}
                            >
                              <Film className="w-6 h-6 text-primary" />
                            </div>
                            <ChevronRight className="w-5 h-5 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                          </div>
                          <div className="text-3xl font-bold text-foreground mb-1">{libraryStats.movies}</div>
                          <div className="text-sm text-muted-foreground">Movies in library</div>
                        </motion.div>

                        {/* TV Shows Card */}
                        <motion.div
                          onClick={() => setView('tv')}
                          className="stat-card-enhanced group"
                          style={{ '--stat-color': 'hsl(200 100% 55%)' } as React.CSSProperties}
                          whileHover={{ scale: 1.02 }}
                          whileTap={{ scale: 0.98 }}
                        >
                          <div className="flex items-start justify-between mb-4">
                            <div
                              className="stat-icon-wrapper"
                              style={{ '--icon-color': 'hsl(200 100% 55%)' } as React.CSSProperties}
                            >
                              <Tv className="w-6 h-6 text-accent" />
                            </div>
                            <ChevronRight className="w-5 h-5 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                          </div>
                          <div className="text-3xl font-bold text-foreground mb-1">{libraryStats.shows}</div>
                          <div className="text-sm text-muted-foreground">TV Shows in library</div>
                        </motion.div>

                        {/* In Progress Card */}
                        <motion.div
                          onClick={() => setView('history')}
                          className="stat-card-enhanced group"
                          style={{ '--stat-color': 'hsl(142 76% 36%)' } as React.CSSProperties}
                          whileHover={{ scale: 1.02 }}
                          whileTap={{ scale: 0.98 }}
                        >
                          <div className="flex items-start justify-between mb-4">
                            <div
                              className="stat-icon-wrapper"
                              style={{ '--icon-color': 'hsl(142 76% 36%)' } as React.CSSProperties}
                            >
                              <Clock className="w-6 h-6 text-emerald-500" />
                            </div>
                            <ChevronRight className="w-5 h-5 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                          </div>
                          <div className="text-3xl font-bold text-foreground mb-1">{continueWatching.length}</div>
                          <div className="text-sm text-muted-foreground">Currently watching</div>
                        </motion.div>
                      </div>
                    </motion.section>
                  )}

                  {/* Empty state */}
                  {!homeSearchQuery && continueWatching.length === 0 && libraryStats.movies === 0 && libraryStats.shows === 0 && (
                    <motion.div
                      className="empty-state-enhanced"
                      initial={{ opacity: 0, scale: 0.9 }}
                      animate={{ opacity: 1, scale: 1 }}
                    >
                      <div className="icon-wrapper">
                        <div className="icon-bg">
                          <Film className="w-10 h-10 text-muted-foreground" />
                        </div>
                      </div>
                      <h3 className="text-xl font-semibold text-foreground mb-2">Your library is empty</h3>
                      <p className="text-muted-foreground max-w-sm mb-6">
                        Add media folders in Settings and scan to discover your movies and TV shows
                      </p>
                      <button
                        onClick={() => setSettingsOpen(true)}
                        className="btn-primary inline-flex items-center gap-2"
                      >
                        <Sparkles className="w-4 h-4" />
                        Get Started
                      </button>
                    </motion.div>
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
                    <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-accent/10 text-accent text-sm font-medium mb-3">
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
                      style={{ '--stat-color': 'hsl(265 84% 62%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.1 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(265 84% 62%)' } as React.CSSProperties}
                        >
                          <Film className="w-6 h-6 text-primary" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{libraryStats.movies}</div>
                      <div className="text-sm text-muted-foreground">Total Movies</div>
                    </motion.div>

                    {/* TV Shows */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(200 100% 55%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.15 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(200 100% 55%)' } as React.CSSProperties}
                        >
                          <Tv className="w-6 h-6 text-accent" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{libraryStats.shows}</div>
                      <div className="text-sm text-muted-foreground">Total TV Shows</div>
                    </motion.div>

                    {/* In Progress */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(142 76% 36%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.2 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(142 76% 36%)' } as React.CSSProperties}
                        >
                          <Clock className="w-6 h-6 text-emerald-500" />
                        </div>
                      </div>
                      <div className="text-4xl font-bold text-foreground mb-1">{continueWatching.length}</div>
                      <div className="text-sm text-muted-foreground">In Progress</div>
                    </motion.div>

                    {/* Items Watched */}
                    <motion.div
                      className="stat-card-enhanced"
                      style={{ '--stat-color': 'hsl(38 92% 50%)' } as React.CSSProperties}
                      initial={{ opacity: 0, y: 20 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: 0.25 }}
                      whileHover={{ scale: 1.02 }}
                    >
                      <div className="flex items-start justify-between mb-3">
                        <div
                          className="stat-icon-wrapper"
                          style={{ '--icon-color': 'hsl(38 92% 50%)' } as React.CSSProperties}
                        >
                          <TrendingUp className="w-6 h-6 text-amber-500" />
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
                          <div className="p-2 rounded-xl bg-primary/10">
                            <Calendar className="w-5 h-5 text-primary" />
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
                              <h4 className="font-semibold text-foreground truncate group-hover:text-primary transition-colors">
                                {item.title}
                              </h4>
                              <div className="flex items-center gap-3 mt-1">
                                <div className="flex-1 h-1.5 rounded-full bg-muted overflow-hidden max-w-32">
                                  <div
                                    className="h-full bg-gradient-to-r from-primary to-primary/80 rounded-full"
                                    style={{ width: `${item.progress_percent || 0}%` }}
                                  />
                                </div>
                                <span className="text-sm text-muted-foreground">
                                  {Math.round(item.progress_percent || 0)}%
                                </span>
                              </div>
                            </div>
                            <div className="p-2 rounded-full bg-muted/50 opacity-0 group-hover:opacity-100 transition-opacity">
                              <Play className="w-5 h-5 text-primary" />
                            </div>
                          </motion.div>
                        ))}
                      </div>
                    </motion.section>
                  )}

                  {/* Empty state for stats */}
                  {continueWatching.length === 0 && libraryStats.movies === 0 && libraryStats.shows === 0 && (
                    <motion.div
                      className="empty-state-enhanced"
                      initial={{ opacity: 0, scale: 0.9 }}
                      animate={{ opacity: 1, scale: 1 }}
                    >
                      <div className="icon-wrapper">
                        <div className="icon-bg">
                          <BarChart3 className="w-10 h-10 text-muted-foreground" />
                        </div>
                      </div>
                      <h3 className="text-xl font-semibold text-foreground mb-2">No activity yet</h3>
                      <p className="text-muted-foreground max-w-sm">
                        Start watching content to see your statistics here
                      </p>
                    </motion.div>
                  )}
                </motion.div>
              )}

              {/* Episodes View */}
              {view === 'episodes' && selectedShow && (
                <motion.div
                  key="episodes"
                  initial={{ opacity: 0, x: 20 }}
                  animate={{ opacity: 1, x: 0 }}
                  exit={{ opacity: 0, x: -20 }}
                >
                  <EpisodeBrowser
                    show={selectedShow}
                    onBack={() => { setView('tv'); setSelectedShow(null) }}
                  />
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
                        <div className="col-span-full">
                          <motion.div
                            className="empty-state-enhanced"
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                          >
                            <div className="icon-wrapper">
                              <div className="icon-bg">
                                <Film className="w-10 h-10 text-muted-foreground" />
                              </div>
                            </div>
                            <h3 className="text-xl font-semibold text-foreground mb-2">No local watch history</h3>
                            <p className="text-muted-foreground max-w-sm">
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
                          className="group relative overflow-hidden rounded-xl bg-card border border-border/50 cursor-pointer transition-all duration-300 hover:border-primary/40 hover:shadow-glow-sm"
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
                                <div className="absolute inset-0 rounded-full bg-primary/30 blur-xl scale-150" />
                                <div className="relative w-14 h-14 rounded-full bg-primary flex items-center justify-center shadow-lg">
                                  <Play className="w-6 h-6 text-white fill-white ml-0.5" />
                                </div>
                              </div>
                            </motion.div>

                            {/* Progress Bar */}
                            {item.progress_percent > 0 && item.progress_percent < 95 && (
                              <div className="absolute bottom-0 left-0 right-0 h-1 bg-background/50">
                                <motion.div
                                  className="h-full bg-primary"
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
                            <h4 className="font-medium text-sm truncate group-hover:text-primary transition-colors">{item.title}</h4>
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
                        <div className="col-span-full">
                          <motion.div
                            className="empty-state-enhanced"
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                          >
                            <div className="icon-wrapper">
                              <div className="icon-bg">
                                <Tv className="w-10 h-10 text-muted-foreground" />
                              </div>
                            </div>
                            <h3 className="text-xl font-semibold text-foreground mb-2">No streaming history</h3>
                            <p className="text-muted-foreground max-w-sm">
                              Stream content from the Stream tab
                            </p>
                          </motion.div>
                        </div>
                      )}
                    </div>
                  )}
                </motion.div>
              )}

              {/* Movies/TV Grid */}
              {(view === 'movies' || view === 'tv') && (
                <motion.div
                  key="grid"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                >
                  <div className="grid-media">
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
                      <div className="col-span-full">
                        <motion.div
                          className="empty-state-enhanced"
                          initial={{ opacity: 0, scale: 0.9 }}
                          animate={{ opacity: 1, scale: 1 }}
                        >
                          <div className="icon-wrapper">
                            <div className="icon-bg">
                              <Search className="w-10 h-10 text-muted-foreground" />
                            </div>
                          </div>
                          <h3 className="text-xl font-semibold text-foreground mb-2">No items found</h3>
                          <p className="text-muted-foreground max-w-sm mb-6">
                            Try adjusting your search or scan your library
                          </p>
                          <button
                            onClick={handleScan}
                            className="btn-primary inline-flex items-center gap-2"
                          >
                            <Sparkles className="w-4 h-4" />
                            Scan Library
                          </button>
                        </motion.div>
                      </div>
                    )}
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </ScrollArea>
      </main>

      {/* Modals */}
      <SettingsModal open={settingsOpen} onOpenChange={setSettingsOpen} />
      <FixMatchModal
        open={fixMatchOpen}
        onOpenChange={setFixMatchOpen}
        item={itemToFix}
        onSuccess={() => { fetchData(); loadLibraryStats() }}
      />
      <PlayerModal
        open={playerModalOpen}
        onOpenChange={setPlayerModalOpen}
        onSelectPlayer={() => { }}
        title={pendingPlayItem?.title || ''}
      />

      {isPlayerOpen && currentStreamInfo && (
        <VideoPlayer
          src={currentStreamInfo.stream_url}
          title={currentStreamInfo.title}
          poster={currentStreamInfo.poster}
          initialTime={currentStreamInfo.resume_position_seconds}
          onClose={() => { setIsPlayerOpen(false); setCurrentStreamInfo(null); setCurrentPlayingId(null) }}
          onProgress={async (currentTime, duration) => {
            if (currentPlayingId) {
              try { await updateWatchProgress(currentPlayingId, currentTime, duration) } catch { }
            }
          }}
        />
      )}

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

      <Toaster />
    </div>
  )
}

export default App
