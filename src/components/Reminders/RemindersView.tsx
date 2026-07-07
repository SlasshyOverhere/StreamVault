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
        <header className="relative w-full shrink-0 overflow-hidden pt-6 pb-5 px-6 md:px-10">
          {/* atmospheric backdrop — radial spotlight from the title, like a marquee */}
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 opacity-90"
            style={{
              background:
                'radial-gradient(ellipse 60% 100% at 18% 50%, hsl(0 0% 100% / 0.05) 0%, transparent 60%),' +
                'radial-gradient(ellipse 50% 80% at 85% 20%, hsl(0 0% 100% / 0.025) 0%, transparent 65%)',
            }}
          />
          {/* hairline border for separation */}
          <div aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-gradient-to-r from-transparent via-white/[0.08] to-transparent" />

          <div className="relative mx-auto max-w-[1400px]">
            {/* eyebrow line — editorial style with date stamp */}
            <div className="flex items-center gap-4 text-[10px] font-medium uppercase tracking-[0.32em] text-white/35">
              <span className="inline-flex items-center gap-1.5">
                <span className="size-1 rounded-full bg-white/50 shadow-[0_0_6px_rgba(255,255,255,0.6)]" />
                <span className="text-white/55">No. <span className="tabular-nums text-white/75">04</span></span>
              </span>
              <span aria-hidden className="h-px w-8 bg-white/15" />
              <span>Catalogue · Reminders · Queue</span>
              <span aria-hidden className="h-px flex-1 bg-gradient-to-r from-white/10 to-transparent" />
              <span className="tabular-nums text-white/35">{new Date().toLocaleDateString(undefined, { month: 'short', day: '2-digit', year: 'numeric' }).toUpperCase()}</span>
            </div>

            {/* main row — title + stats + notif */}
            <div className="mt-5 flex items-end justify-between gap-8">
              <div className="min-w-0 flex-1">
                {/* editorial title — magazine masthead style with vol. number + year + cursor */}
                <div className="relative">
                  <div className="flex items-center gap-3 mb-2.5">
                    <span className="text-[10px] font-bold uppercase tracking-[0.34em] text-white/40">
                      Vol.
                    </span>
                    <span className="tabular-nums text-[10px] font-bold tracking-[0.22em] text-white/75">
                      {new Date().getFullYear()}
                    </span>
                    <span aria-hidden className="h-px flex-1 max-w-[100px] bg-white/15" />
                    <span className="text-[9px] font-bold uppercase tracking-[0.32em] text-white/40">
                      Curated
                    </span>
                  </div>
                  <h1 className="text-[40px] md:text-[54px] font-bold tracking-[-0.04em] leading-[0.92] text-white">
                    Watchlist
                  </h1>
                  {/* hairline rule with progressive dot terminator */}
                  <div className="mt-3.5 flex items-center gap-2">
                    <span className="h-[2px] w-14 rounded-full bg-white" />
                    <span className="h-[1px] w-8 bg-white/30" />
                    <span className="h-[1px] w-4 bg-white/15" />
                    <span aria-hidden className="ml-1 size-1.5 rounded-full bg-white shadow-[0_0_6px_rgba(255,255,255,0.5)]" />
                  </div>
                </div>
                <p className="mt-4 text-[13px] leading-relaxed text-white/45 max-w-md">
                  Find films and series, schedule a release alert, or quietly save them for the right night.
                </p>
              </div>

              {/* stat strip — clean inline editorial blocks */}
              <div className="hidden md:flex items-stretch divide-x divide-white/[0.08] rounded-lg border border-white/[0.08] bg-white/[0.02]">
                <StatBlock label="Saved" value={stats.watchlistCount} hint="Watchlist" />
                <StatBlock label="Reminded" value={stats.remindedCount} hint="Active alerts" />
                <StatBlock label="Scheduled" value={stats.reminderCount} hint="Releases" />
              </div>

              <label className="flex items-center gap-3 text-[11px] font-medium uppercase tracking-[0.2em] text-white/55 cursor-pointer select-none shrink-0 self-start md:self-end pb-1 group/notif">
                <span className="relative">
                  Notifications
                  {config?.notifications_enabled && (
                    <span className="absolute -right-3 -top-0.5 size-1 rounded-full bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.8)]" />
                  )}
                </span>
                <Switch
                  checked={config?.notifications_enabled || false}
                  onCheckedChange={handleToggleNotifications}
                  className="scale-90 data-[state=checked]:bg-white transition-colors"
                />
              </label>
            </div>

            {/* tab row — magazine-issue style with numeral prefix and underline indicator */}
            <div className="mt-7 flex items-end gap-0">
              {TABS.map((tab, i) => {
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
                      'group relative -mb-px flex items-center gap-2.5 px-4 py-3 text-[13px] font-semibold transition-all duration-200',
                      active ? 'text-white' : 'text-white/40 hover:text-white/75',
                    )}
                  >
                    {/* section numeral */}
                    <span className={cn(
                      'tabular-nums text-[9px] font-bold tracking-[0.2em] transition-colors duration-200',
                      active ? 'text-white/70' : 'text-white/25 group-hover:text-white/45',
                    )}>
                      {String(i + 1).padStart(2, '0')}
                    </span>
                    <tab.icon className={cn('size-3.5 transition-transform duration-300', active && 'scale-110')} />
                    <span className="tracking-[-0.005em]">{tab.label}</span>
                    {count !== null && count > 0 && (
                      <span className={cn(
                        'tabular-nums text-[10px] font-bold px-1.5 py-0.5 rounded-full transition-colors',
                        active ? 'bg-white text-black' : 'bg-white/[0.06] text-white/50 group-hover:bg-white/10',
                      )}>
                        {count}
                      </span>
                    )}
                    {active && (
                      <>
                        {/* bold underline indicator */}
                        <span
                          aria-hidden
                          className="absolute inset-x-3 -bottom-px h-[2px] bg-white"
                        />
                        {/* small dot terminal at end of underline */}
                        <span
                          aria-hidden
                          className="absolute -bottom-[3px] right-3 size-[6px] rounded-full bg-white shadow-[0_0_8px_rgba(255,255,255,0.6)]"
                        />
                      </>
                    )}
                  </button>
                )
              })}
              {/* right-side meta */}
              <div className="ml-auto hidden lg:flex items-center gap-3 pb-3 text-[10px] font-medium uppercase tracking-[0.28em] text-white/30">
                <span>Edition</span>
                <span className="tabular-nums text-white/60">2025</span>
              </div>
            </div>
          </div>
        </header>

        <main className="flex-1 w-full min-h-0 relative overflow-hidden flex justify-center">
          <div className="w-full max-w-[1400px] h-full min-h-0 overflow-hidden px-6 md:px-10 pb-10">
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
function StatBlock({ label, value, hint }: { label: string; value: number; hint: string }) {
  return (
    <div className="px-4 py-2.5 min-w-[88px] transition-colors hover:bg-white/[0.025]">
      <div className="flex items-baseline gap-1.5">
        <span className="text-[20px] font-bold tracking-[-0.03em] tabular-nums text-white leading-none">
          {String(value).padStart(2, '0')}
        </span>
        <span className="text-[9px] font-bold uppercase tracking-[0.22em] text-white/45">
          {label}
        </span>
      </div>
      <div className="mt-1 text-[9px] uppercase tracking-[0.2em] text-white/35">
        {hint}
      </div>
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
      <div className="shrink-0 flex items-center gap-3 pb-5">
        <div className="relative flex-1 max-w-2xl group/search">
          <Search className="absolute left-4 top-1/2 -translate-y-1/2 size-4 text-white/35 transition-colors group-focus-within/search:text-white" />
          <Input
            value={props.searchQuery}
            onChange={e => props.onSearchInputChange(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && props.onSearch()}
            placeholder="Search TMDB…"
            className="h-11 pl-11 pr-24 rounded-lg bg-white/[0.025] border border-white/[0.08] focus:bg-white/[0.05] focus:border-white/30 focus:shadow-[0_0_0_3px_rgba(255,255,255,0.04)] text-[13px] placeholder:text-white/30 transition-all duration-200"
          />
          {/* keyboard hint */}
          <div className="absolute right-3 top-1/2 -translate-y-1/2 hidden sm:flex items-center gap-1">
            <kbd className="rounded border border-white/10 bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-widest text-white/45">
              Enter
            </kbd>
          </div>
          {props.isSearching && (
            <Loader2 className="absolute right-3 top-1/2 -translate-y-1/2 size-4 animate-spin text-white/40 sm:hidden" />
          )}
        </div>
        <Button
          type="button"
          onClick={() => props.onSearch()}
          disabled={props.isSearching || !props.searchQuery.trim()}
          className="h-11 px-5 rounded-lg bg-white text-black text-[12px] font-bold uppercase tracking-[0.12em] hover:bg-white/90 hover:shadow-[0_0_20px_-3px_rgba(255,255,255,0.3)] disabled:opacity-30 disabled:shadow-none transition-all duration-200"
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

                  <div className="relative grid grid-cols-1 sm:grid-cols-[200px_1fr_auto] gap-6 p-6 md:p-7 items-center min-h-[220px]">
                    <div className="relative w-[160px] aspect-[2/3] overflow-hidden rounded-md border border-white/10 shadow-elevation-2">
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
                      <div className="flex items-center gap-2.5 text-[10px] font-bold uppercase tracking-[0.28em] text-white/45">
                        <span className="text-white/35">§</span>
                        <Sparkles className="size-3 text-white/55" />
                        <span>The Pick</span>
                        <span aria-hidden className="h-px w-6 bg-white/15" />
                        <span className="text-white/35">{heroItem.media_type === 'movie' ? 'Film' : 'Series'}</span>
                      </div>
                      <h2 className="text-[28px] md:text-[36px] font-bold tracking-[-0.035em] leading-[0.95] text-white">
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
                  <div className="mb-5 flex items-end justify-between gap-4 border-b border-white/[0.06] pb-3">
                    <div className="flex items-end gap-3">
                      <span className="text-[9px] font-bold uppercase tracking-[0.32em] text-white/35">
                        §
                      </span>
                      <h2 className="text-[14px] font-bold uppercase tracking-[0.18em] text-white/85">
                        Trending today
                      </h2>
                      <span aria-hidden className="mb-1 h-px w-8 bg-white/15" />
                      <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-white/35 pb-0.5">
                        Editor's selection
                      </span>
                    </div>
                    <div className="flex items-center gap-2 text-[10px] tabular-nums text-white/40">
                      <span className="size-1 rounded-full bg-emerald-400/80 shadow-[0_0_4px_rgba(52,211,153,0.6)]" />
                      <span className="uppercase tracking-[0.18em]">Live</span>
                      <span className="text-white/55">{props.trending.length} titles</span>
                    </div>
                  </div>
                  <ul className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-x-5 gap-y-6">
                    {props.trending.slice(1).map((item, idx) => (
                      <TrendingTile
                        key={`${item.media_type}-${item.id}`}
                        item={item}
                        index={idx}
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
                <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-x-6 gap-y-8">
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

function TrendingTile({ item, index, onOpen }: { item: TrendingRich; index?: number; onOpen: () => void }) {
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
        {/* editorial rank chip — top-left */}
        {index !== undefined && (
          <div className="absolute top-2 left-2 inline-flex items-center gap-1 rounded-md bg-black/70 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.18em] text-white/80 backdrop-blur-sm">
            <span className="tabular-nums text-white">{String(index + 1).padStart(2, '0')}</span>
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
