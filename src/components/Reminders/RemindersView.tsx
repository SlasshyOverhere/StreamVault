import { useEffect, useMemo, useState } from 'react'
import {
  Search, Film, Tv, Loader2, Bookmark, Bell, Sparkles, ArrowUpRight,
} from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { useToast } from '@/components/ui/use-toast'
import {
  searchTmdb,
  getMovieReminders,
  createMovieReminder,
  updateMovieReminder,
  getTmdbTrending,
  getMovieDetails,
  getTvDetails,
  getTmdbImageUrl,
  getConfig,
  saveConfig,
  createOrUpdateWatchlistItem,
  getWatchlistItems,
  syncWatchlist,
  updateWatchlistItem,
  MovieReminder,
  TmdbSearchResult,
  TmdbTrendingItem,
  Config,
  MovieReminderInput,
  WatchlistItem,
  WatchlistItemInput,
} from '@/services/api'
import { ReminderEditor } from './ReminderEditor'
import { RemindersList } from './RemindersList'
import { TmdbDetailsModal } from './TmdbDetailsModal'
import { WatchlistEditor } from './WatchlistEditor'
import { WatchlistList } from './WatchlistList'
import { LazyMotion, domAnimation, m, AnimatePresence } from 'framer-motion'
import { cn } from '@/lib/utils'

type Tab = 'discover' | 'reminders' | 'watchlist'
const TABS: { id: Tab; label: string; icon: typeof Search; hint: string }[] = [
  { id: 'discover',  label: 'Discover',  icon: Search,   hint: 'Search TMDB' },
  { id: 'reminders', label: 'Reminders', icon: Bell,     hint: 'Scheduled alerts' },
  { id: 'watchlist', label: 'Watchlist', icon: Bookmark, hint: 'Saved for later' },
]

// local rich type — TMDB returns minimal trending shape; we enrich from full details
type TrendingRich = {
  id: number
  media_type: 'movie' | 'tv'
  title: string
  poster_path: string | null
  backdrop_path: string | null
  release_date: string | null
  vote_average: number | null
  overview: string | null
}

export function RemindersView() {
  const { toast } = useToast()
  const [activeTab, setActiveTab] = useState<Tab>('discover')
  const [searchQuery, setSearchQuery] = useState('')
  const [mediaFilter, setMediaFilter] = useState<'all' | 'movie' | 'tv'>('all')
  const [isSearching, setIsSearching] = useState(false)
  const [searchResults, setSearchResults] = useState<TmdbSearchResult[]>([])
  const [trendingRich, setTrendingRich] = useState<TrendingRich[]>([])
  const [reminders, setReminders] = useState<MovieReminder[]>([])
  const [watchlistItems, setWatchlistItems] = useState<WatchlistItem[]>([])
  const [config, setConfig] = useState<Config | null>(null)

  const [detailsModalOpen, setDetailsModalOpen] = useState(false)
  const [selectedResult, setSelectedResult] = useState<{ id: number, type: 'movie' | 'tv' } | null>(null)

  const [editorOpen, setEditorOpen] = useState(false)
  const [editingReminder, setEditingReminder] = useState<Partial<MovieReminderInput> | MovieReminder | null>(null)

  const [watchlistEditorOpen, setWatchlistEditorOpen] = useState(false)
  const [editingWatchlistItem, setEditingWatchlistItem] = useState<Partial<WatchlistItemInput> | WatchlistItem | null>(null)

  useEffect(() => {
    loadReminders()
    loadConfig()
    loadTrendingSuggestions()
    loadWatchlist(true)

    let unlistenReminderRefresh: (() => void) | undefined
    let unlistenWatchlistRefresh: (() => void) | undefined

    const setup = async () => {
      const { listen } = await import('@tauri-apps/api/event')
      unlistenReminderRefresh = await listen('refresh-reminders', () => {
        loadReminders()
      })
      unlistenWatchlistRefresh = await listen('refresh-watchlist', () => {
        loadWatchlist()
      })
    }
    setup()

    return () => {
      unlistenReminderRefresh?.()
      unlistenWatchlistRefresh?.()
    }
  }, [])

  const loadReminders = async () => {
    try {
      const data = await getMovieReminders(true)
      setReminders(data)
    } catch (error) {
      console.error('Failed to load reminders:', error)
    }
  }

  const loadWatchlist = async (shouldSync = false) => {
    if (shouldSync) {
      try {
        await syncWatchlist()
      } catch (error) {
        console.error('Watchlist sync failed, falling back to local data:', error)
      }
    }

    try {
      const data = await getWatchlistItems(true)
      setWatchlistItems(data)
    } catch (error) {
      console.error('Failed to load watchlist:', error)
    }
  }

  useEffect(() => {
    if (activeTab === 'watchlist') {
      loadWatchlist()
    }
  }, [activeTab])

  const loadConfig = async () => {
    try {
      const data = await getConfig()
      setConfig(data)
    } catch (error) {
      console.error('Failed to load config:', error)
    }
  }

  // pull full TMDB details for each trending item so the hero card has poster + backdrop
  const loadTrendingSuggestions = async () => {
    try {
      const response = await getTmdbTrending()
      const slice = response.results.slice(0, 8)
      const enriched = await Promise.all(
        slice.map(async (item: TmdbTrendingItem) => {
          try {
            const details = item.media_type === 'movie'
              ? await getMovieDetails(item.id, null)
              : await getTvDetails(item.id, null)
            if (!details) return null
            return {
              id: item.id,
              media_type: item.media_type,
              title: 'title' in details ? details.title : details.name,
              poster_path: details.poster_path ?? null,
              backdrop_path: details.backdrop_path ?? null,
              release_date: 'release_date' in details ? (details.release_date ?? null) : (('first_air_date' in details ? details.first_air_date : null) ?? null),
              vote_average: details.vote_average ?? null,
              overview: details.overview ?? null,
            } satisfies TrendingRich
          } catch {
            return null
          }
        })
      )
      setTrendingRich(enriched.filter((x): x is TrendingRich => x !== null))
    } catch (error) {
      console.error('Failed to load TMDB trending suggestions:', error)
      setTrendingRich([])
    }
  }

  const handleSearch = async (queryOverride?: string) => {
    const query = (queryOverride ?? searchQuery).trim()
    if (!query) return
    setSearchQuery(query)
    setIsSearching(true)
    try {
      const response = await searchTmdb(query)
      setSearchResults(response.results)
    } catch (error) {
      console.error('TMDB search failed:', error)
      toast({
        title: "Search failed",
        description: "Could not connect to TMDB. Please check your internet connection.",
        variant: "destructive"
      })
    } finally {
      setIsSearching(false)
    }
  }

  const handleSearchInputChange = (value: string) => {
    setSearchQuery(value)
    if (!value.trim()) {
      setSearchResults([])
      setMediaFilter('all')
    }
  }

  const openTrendingDetails = (item: TrendingRich) => {
    setSelectedResult({ id: item.id, type: item.media_type })
    setDetailsModalOpen(true)
  }

  const handleDetailsOpenChange = (open: boolean) => {
    setDetailsModalOpen(open)
    if (!open) {
      setSelectedResult(null)
    }
  }

  const handleToggleNotifications = async (enabled: boolean) => {
    if (!config) return
    const newConfig = { ...config, notifications_enabled: enabled }
    try {
      await saveConfig(newConfig)
      setConfig(newConfig)
      toast({
        title: enabled ? "Notifications enabled" : "Notifications disabled",
        description: enabled
          ? "You will receive native alerts for your reminders and watchlist."
          : "Schedules stay saved, but alerts are silenced."
      })
    } catch (error) {
      console.error('Failed to save config:', error)
    }
  }

  const filteredResults = searchResults.filter(result => {
    if (mediaFilter === 'all') return true
    return result.media_type === mediaFilter
  })

  const handleSetReminderFromSearch = (data: Partial<MovieReminderInput>) => {
    setEditingReminder(data)
    setEditorOpen(true)
  }

  const handleAddToWatchlist = (data: Partial<WatchlistItemInput>) => {
    setEditingWatchlistItem(data)
    setWatchlistEditorOpen(true)
  }

  const handleSaveReminder = async (input: MovieReminderInput) => {
    try {
      if (editingReminder && 'id' in editingReminder) {
        await updateMovieReminder(editingReminder.id as number, input)
        toast({ title: "Reminder updated" })
      } else {
        await createMovieReminder(input)
        toast({ title: "Reminder set successfully" })
      }
      loadReminders()
      setActiveTab('reminders')
    } catch (error) {
      console.error('Failed to save reminder:', error)
      toast({
        title: "Failed to save reminder",
        description: "There was an error communicating with the backend.",
        variant: "destructive"
      })
    }
  }

  const handleSaveWatchlist = async (input: WatchlistItemInput) => {
    try {
      if (editingWatchlistItem && 'id' in editingWatchlistItem) {
        await updateWatchlistItem(editingWatchlistItem.id as number, input)
        toast({ title: "Watchlist updated" })
      } else {
        await createOrUpdateWatchlistItem(input)
        toast({ title: "Added to watchlist" })
      }
      loadWatchlist()
      setActiveTab('watchlist')
    } catch (error) {
      console.error('Failed to save watchlist item:', error)
      toast({
        title: "Failed to update watchlist",
        description: "There was an error saving your watchlist item.",
        variant: "destructive"
      })
    }
  }

  // stat strip — feeds the header with something real to display
  const stats = useMemo(() => {
    const watchlistCount = watchlistItems.length
    const remindedCount = watchlistItems.filter(i => i.notification_enabled).length
    const reminderCount = reminders.length
    return { watchlistCount, remindedCount, reminderCount }
  }, [watchlistItems, reminders])

  return (
    <LazyMotion features={domAnimation}>
      <div className="h-full min-h-0 flex flex-col bg-transparent text-white relative overflow-hidden font-sans">
        <header className="w-full shrink-0 pt-8 pb-5 px-6 md:px-10">
          <div className="mx-auto max-w-6xl">
            {/* eyebrow line */}
            <div className="flex items-center gap-3 text-[10px] font-medium uppercase tracking-[0.32em] text-white/30">
              <span className="size-1 rounded-full bg-white/40" />
              <span>Catalogue · Reminders · Queue</span>
            </div>

            {/* main row — title + stats + notif */}
            <div className="mt-3 flex items-end justify-between gap-8">
              <div className="min-w-0 flex-1">
                <h1 className="text-[34px] md:text-[42px] font-semibold tracking-[-0.02em] leading-[0.95] text-white">
                  Watchlist
                </h1>
                <p className="mt-2 text-[13px] text-white/45 max-w-md">
                  Find films and series, schedule a release alert, or quietly save them for the right night.
                </p>
              </div>

              {/* stat strip */}
              <div className="hidden md:flex items-stretch divide-x divide-white/[0.06] rounded-lg border border-white/[0.06] bg-white/[0.015]">
                <Stat label="Saved" value={stats.watchlistCount} hint="Watchlist" />
                <Stat label="Reminded" value={stats.remindedCount} hint="Active alerts" />
                <Stat label="Scheduled" value={stats.reminderCount} hint="Releases" />
              </div>

              <label className="flex items-center gap-2.5 text-[12px] text-white/45 cursor-pointer select-none shrink-0 self-start md:self-end pb-1.5">
                <span>Notifications</span>
                <Switch
                  checked={config?.notifications_enabled || false}
                  onCheckedChange={handleToggleNotifications}
                  className="scale-90 data-[state=checked]:bg-white transition-colors"
                />
              </label>
            </div>

            {/* tab row */}
            <div className="mt-7 flex items-center gap-1 border-b border-white/[0.06]">
              {TABS.map(tab => {
                const active = activeTab === tab.id
                const count = tab.id === 'watchlist' ? stats.watchlistCount
                  : tab.id === 'reminders' ? stats.reminderCount
                  : null
                return (
                  <button
                    type="button"
                    key={tab.id}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveTab(tab.id)}
                    className={cn(
                      'group relative -mb-px flex items-center gap-2 px-3 py-2.5 text-[13px] font-medium transition-colors duration-150',
                      active ? 'text-white' : 'text-white/40 hover:text-white/70',
                    )}
                  >
                    <tab.icon className="size-3.5" />
                    <span>{tab.label}</span>
                    {count !== null && count > 0 && (
                      <span className={cn(
                        'tabular-nums text-[10px] px-1.5 py-0.5 rounded',
                        active ? 'bg-white/10 text-white/70' : 'bg-white/[0.04] text-white/35',
                      )}>
                        {count}
                      </span>
                    )}
                    {active && (
                      <span
                        aria-hidden
                        className="absolute inset-x-0 -bottom-px h-[2px] bg-white"
                      />
                    )}
                  </button>
                )
              })}
            </div>
          </div>
        </header>

        <main className="flex-1 w-full min-h-0 relative overflow-hidden flex justify-center">
          <div className="w-full max-w-6xl h-full min-h-0 overflow-hidden px-6 md:px-10 pb-10">
            <AnimatePresence mode="wait">
              {activeTab === 'discover' ? (
                <m.div
                  key="discover"
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
                  className="h-full flex flex-col min-h-0"
                >
                  <DiscoverTab
                    searchQuery={searchQuery}
                    onSearchInputChange={handleSearchInputChange}
                    onSearch={handleSearch}
                    isSearching={isSearching}
                    results={filteredResults}
                    allResults={searchResults}
                    mediaFilter={mediaFilter}
                    setMediaFilter={setMediaFilter}
                    trending={trendingRich}
                    onOpenTrending={openTrendingDetails}
                    onOpenResult={(id, type) => {
                      setSelectedResult({ id, type })
                      setDetailsModalOpen(true)
                    }}
                  />
                </m.div>
              ) : activeTab === 'reminders' ? (
                <m.div
                  key="reminders"
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
                  className="h-full min-h-0"
                >
                  <RemindersList
                    reminders={reminders}
                    onEdit={(r) => {
                      setEditingReminder(r)
                      setEditorOpen(true)
                    }}
                    onRefresh={loadReminders}
                  />
                </m.div>
              ) : (
                <m.div
                  key="watchlist"
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
                  className="h-full min-h-0"
                >
                  <WatchlistList
                    items={watchlistItems}
                    onEdit={(item) => {
                      setEditingWatchlistItem(item)
                      setWatchlistEditorOpen(true)
                    }}
                    onRefresh={loadWatchlist}
                  />
                </m.div>
              )}
            </AnimatePresence>
          </div>
        </main>

        {selectedResult && (
          <TmdbDetailsModal
            key={`${selectedResult.type}-${selectedResult.id}`}
            open={detailsModalOpen}
            onOpenChange={handleDetailsOpenChange}
            tmdbId={selectedResult.id}
            mediaType={selectedResult.type}
            onSetReminder={(data) => {
              handleDetailsOpenChange(false)
              handleSetReminderFromSearch(data)
            }}
            onAddToWatchlist={(data) => {
              handleDetailsOpenChange(false)
              handleAddToWatchlist(data)
            }}
          />
        )}

        <ReminderEditor open={editorOpen} onOpenChange={setEditorOpen} initialData={editingReminder || undefined} onSave={handleSaveReminder} />
        <WatchlistEditor open={watchlistEditorOpen} onOpenChange={setWatchlistEditorOpen} initialData={editingWatchlistItem || undefined} onSave={handleSaveWatchlist} />
      </div>
    </LazyMotion>
  )
}

// ─── header stat block ─────────────────────────────────
function Stat({ label, value, hint }: { label: string; value: number; hint: string }) {
  return (
    <div className="px-5 py-2.5 min-w-[88px]">
      <div className="flex items-baseline gap-1.5">
        <span className="text-[20px] font-semibold tracking-[-0.02em] tabular-nums text-white">{value}</span>
        <span className="text-[10px] uppercase tracking-[0.18em] text-white/30">{label}</span>
      </div>
      <div className="text-[10px] text-white/25 mt-0.5">{hint}</div>
    </div>
  )
}

// ─── discover subtab ────────────────────────────────────
function DiscoverTab(props: {
  searchQuery: string
  onSearchInputChange: (v: string) => void
  onSearch: (override?: string) => void
  isSearching: boolean
  results: TmdbSearchResult[]
  allResults: TmdbSearchResult[]
  mediaFilter: 'all' | 'movie' | 'tv'
  setMediaFilter: (m: 'all' | 'movie' | 'tv') => void
  trending: TrendingRich[]
  onOpenTrending: (item: TrendingRich) => void
  onOpenResult: (id: number, type: 'movie' | 'tv') => void
}) {
  const showingResults = props.allResults.length > 0

  const heroItem = props.trending[0]
  const heroPoster = heroItem?.poster_path ? getTmdbImageUrl(heroItem.poster_path, 'w500') : null
  const heroBackdrop = heroItem?.backdrop_path ? getTmdbImageUrl(heroItem.backdrop_path, 'w500') : null
  const heroYear = heroItem?.release_date
    ? new Date(heroItem.release_date).getFullYear()
    : null

  return (
    <div className="h-full flex flex-col min-h-0">
      {/* search row */}
      <div className="shrink-0 flex items-center gap-2 pb-5">
        <div className="relative flex-1 max-w-2xl group/search">
          <Search className="absolute left-3.5 top-1/2 -translate-y-1/2 size-4 text-white/30 transition-colors group-focus-within/search:text-white/60" />
          <Input
            value={props.searchQuery}
            onChange={e => props.onSearchInputChange(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && props.onSearch()}
            placeholder="Search TMDB…"
            className="h-10 pl-10 pr-3 rounded-md bg-white/[0.02] border-white/[0.06] focus:bg-white/[0.04] focus:border-white/20 text-[13px] placeholder:text-white/25 transition-colors"
          />
          {props.isSearching && (
            <Loader2 className="absolute right-3 top-1/2 -translate-y-1/2 size-4 animate-spin text-white/40" />
          )}
        </div>
        <Button
          type="button"
          onClick={() => props.onSearch()}
          disabled={props.isSearching || !props.searchQuery.trim()}
          className="h-10 px-4 rounded-md bg-white text-black text-[12px] font-medium hover:bg-white/90 disabled:opacity-40"
        >
          Search
        </Button>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto pr-1">
        <AnimatePresence mode="wait">
          {!showingResults ? (
            <m.div
              key="idle"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.3, ease: 'easeOut' }}
              className="space-y-8"
            >
              {/* hero feature card */}
              {heroItem && (
                <button
                  type="button"
                  onClick={() => props.onOpenTrending(heroItem)}
                  className="group/hero relative block w-full overflow-hidden rounded-xl border border-white/[0.06] bg-[hsl(0_0%_5%)] text-left transition-colors hover:border-white/[0.15]"
                >
                  {heroBackdrop && (
                    <div
                      className="absolute inset-0 opacity-40 transition-opacity duration-700 group-hover/hero:opacity-50"
                      style={{ backgroundImage: `url(${heroBackdrop})`, backgroundSize: 'cover', backgroundPosition: 'center' }}
                    />
                  )}
                  <div className="absolute inset-0 bg-gradient-to-r from-[hsl(0_0%_4%)] via-[hsl(0_0%_4%)/0.7] to-[hsl(0_0%_4%)/0]" />
                  <div className="absolute inset-0 bg-gradient-to-t from-[hsl(0_0%_4%)] via-transparent to-transparent" />

                  <div className="relative grid grid-cols-1 sm:grid-cols-[180px_1fr_auto] gap-6 p-6 md:p-7 items-center min-h-[260px]">
                    <div className="relative w-[140px] aspect-[2/3] overflow-hidden rounded-md border border-white/10 shadow-elevation-2">
                      {heroPoster ? (
                        <img
                          src={heroPoster}
                          alt={heroItem.title}
                          className="h-full w-full object-cover transition-transform duration-700 group-hover/hero:scale-[1.02]"
                          loading="lazy"
                        />
                      ) : (
                        <div className="flex h-full w-full items-center justify-center text-white/10">
                          {heroItem.media_type === 'movie' ? <Film className="size-10" /> : <Tv className="size-10" />}
                        </div>
                      )}
                    </div>

                    <div className="min-w-0 space-y-3">
                      <div className="flex items-center gap-2 text-[10px] uppercase tracking-[0.24em] text-white/40">
                        <Sparkles className="size-3 text-white/40" />
                        <span>Featured today</span>
                      </div>
                      <h2 className="text-[24px] md:text-[28px] font-semibold tracking-[-0.02em] leading-[1.1] text-white">
                        {heroItem.title}
                      </h2>
                      <div className="flex items-center gap-2.5 text-[12px] text-white/45">
                        {heroYear && <span className="tabular-nums">{heroYear}</span>}
                        {heroYear && <span className="text-white/15">·</span>}
                        <span className="uppercase tracking-[0.18em] text-white/40">
                          {heroItem.media_type === 'movie' ? 'Film' : 'Series'}
                        </span>
                        {(heroItem.vote_average ?? 0) > 0 && (
                          <>
                            <span className="text-white/15">·</span>
                            <span className="tabular-nums">{(heroItem.vote_average ?? 0).toFixed(1)}</span>
                          </>
                        )}
                      </div>
                      {heroItem.overview && (
                        <p className="hidden sm:block text-[13px] text-white/50 leading-relaxed line-clamp-2 max-w-lg">
                          {heroItem.overview}
                        </p>
                      )}
                    </div>

                    <div className="flex sm:flex-col items-start sm:items-end gap-2 sm:gap-3 sm:self-center">
                      <span className="inline-flex items-center gap-1.5 text-[11px] uppercase tracking-[0.2em] text-white/60 group-hover/hero:text-white transition-colors">
                        Open details
                        <ArrowUpRight className="size-3.5 transition-transform group-hover/hero:translate-x-0.5 group-hover/hero:-translate-y-0.5" />
                      </span>
                    </div>
                  </div>
                </button>
              )}

              {/* trending row */}
              {props.trending.length > 1 && (
                <section>
                  <div className="mb-3 flex items-baseline justify-between">
                    <h2 className="text-[11px] font-medium uppercase tracking-[0.18em] text-white/40">
                      Trending today
                    </h2>
                    <span className="text-[10px] tabular-nums text-white/20">{props.trending.length} titles</span>
                  </div>
                  <ul className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-x-5 gap-y-6">
                    {props.trending.slice(1).map(item => (
                      <TrendingTile
                        key={`${item.media_type}-${item.id}`}
                        item={item}
                        onOpen={() => props.onOpenTrending(item)}
                      />
                    ))}
                  </ul>
                </section>
              )}

              {!props.isSearching && !props.searchQuery && props.trending.length === 0 && (
                <div className="flex flex-col items-center justify-center text-center py-20">
                  <p className="text-[13px] text-white/35">
                    Search the catalog or open a trending title.
                  </p>
                </div>
              )}
            </m.div>
          ) : (
            <m.div
              key="results"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.3, ease: 'easeOut' }}
              className="space-y-5"
            >
              <div className="flex items-center justify-between">
                <p className="text-[11px] text-white/40 tabular-nums">
                  {props.results.length} {props.results.length === 1 ? 'result' : 'results'} for &ldquo;{props.searchQuery}&rdquo;
                </p>
                <div className="flex items-center rounded-md border border-white/[0.06] bg-white/[0.015] p-0.5">
                  {([
                    { id: 'all', label: 'All' },
                    { id: 'movie', label: 'Movies' },
                    { id: 'tv', label: 'Series' },
                  ] as { id: 'all' | 'movie' | 'tv'; label: string }[]).map(opt => (
                    <button
                      type="button"
                      key={opt.id}
                      onClick={() => props.setMediaFilter(opt.id)}
                      className={cn(
                        'px-2.5 py-1 rounded-[5px] text-[11px] font-medium transition-colors duration-150',
                        props.mediaFilter === opt.id
                          ? 'bg-white/[0.08] text-white'
                          : 'text-white/35 hover:text-white/70',
                      )}
                    >
                      {opt.label}
                    </button>
                  ))}
                </div>
              </div>

              {props.results.length === 0 ? (
                <p className="text-center text-[12px] text-white/35 py-12">No matches for this filter.</p>
              ) : (
                <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-x-6 gap-y-8">
                  {props.results.map((r, i) => (
                    <DiscoverResult
                      key={`${r.media_type}-${r.id}`}
                      result={r}
                      index={i}
                      onOpen={() => props.onOpenResult(r.id, r.media_type as 'movie' | 'tv')}
                    />
                  ))}
                </div>
              )}
            </m.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}

function TrendingTile({ item, onOpen }: { item: TrendingRich; onOpen: () => void }) {
  const posterUrl = item.poster_path ? getTmdbImageUrl(item.poster_path, 'w185') : null
  const year = item.release_date ? new Date(item.release_date).getFullYear() : null
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group text-left w-full"
      aria-label={`Open ${item.title}`}
    >
      <div className="relative aspect-[2/3] overflow-hidden rounded-md bg-[hsl(0_0%_5%)] border border-white/[0.05] transition-colors duration-150 group-hover:border-white/20">
        {posterUrl ? (
          <img
            src={posterUrl}
            alt={item.title}
            loading="lazy"
            className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.02]"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-white/10">
            {item.media_type === 'movie' ? <Film className="size-8" /> : <Tv className="size-8" />}
          </div>
        )}
      </div>
      <div className="mt-2 space-y-0.5">
        <p className="text-[12px] font-medium text-white/80 line-clamp-1 leading-tight">
          {item.title}
        </p>
        {year !== null && (
          <p className="text-[10px] tabular-nums text-white/30">
            {year}
            {(item.vote_average ?? 0) > 0 && (
              <>
                <span className="mx-1.5 text-white/15">·</span>
                <span>{(item.vote_average ?? 0).toFixed(1)}</span>
              </>
            )}
          </p>
        )}
      </div>
    </button>
  )
}

function DiscoverResult({ result, index, onOpen }: { result: TmdbSearchResult; index: number; onOpen: () => void }) {
  const posterUrl = result.poster_path ? getTmdbImageUrl(result.poster_path, 'w300') : null
  return (
    <m.button
      type="button"
      onClick={onOpen}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4, delay: Math.min(index * 0.025, 0.4), ease: [0.22, 1, 0.36, 1] }}
      className="group text-left w-full"
      aria-label={`View details for ${result.title || result.name}`}
    >
      <div className="relative aspect-[2/3] overflow-hidden rounded-md bg-[hsl(0_0%_5%)] border border-white/[0.05] transition-colors duration-150 group-hover:border-white/20">
        {posterUrl ? (
          <img
            src={posterUrl}
            alt={result.title || result.name || ''}
            loading="lazy"
            className="h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.02]"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-white/10">
            {result.media_type === 'movie'
              ? <Film className="size-8" />
              : <Tv className="size-8" />
            }
          </div>
        )}
      </div>
      <div className="mt-2.5 space-y-0.5">
        <p className="text-[13px] font-medium leading-tight tracking-tight text-white/85 line-clamp-1">
          {result.title || result.name}
        </p>
        <p className="text-[11px] tabular-nums text-white/30 line-clamp-1">
          {result.release_date
            ? new Date(result.release_date).getFullYear()
            : result.first_air_date
              ? new Date(result.first_air_date).getFullYear()
              : 'TBA'}
          {(result.vote_average ?? 0) > 0 && (
            <>
              <span className="mx-1.5 text-white/15">·</span>
              <span>{(result.vote_average ?? 0).toFixed(1)}</span>
            </>
          )}
        </p>
      </div>
    </m.button>
  )
}
