import { useState, useMemo, useCallback, useEffect, useRef } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import {
  Film, Tv, Bell, Pencil, Trash2, Loader2,
  LayoutGrid, Rows3, ChevronLeft, ChevronRight, X, CalendarDays, Bookmark
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { WatchlistItem, deleteWatchlistItem, getTmdbImageUrl } from '@/services/api'
import { CountdownTimer } from './CountdownTimer'
import { formatLocalReleaseTime } from './CountdownTimer.utils'
import { cn } from '@/lib/utils'

interface WatchlistListProps {
  items: WatchlistItem[]
  onEdit: (item: WatchlistItem) => void
  onRefresh: () => void
  loading?: boolean
}

// ─── helpers ─────────────────────────────────────────────
const startOfDay = (d: Date) => { const x = new Date(d); x.setHours(0, 0, 0, 0); return x }
const endOfWeek = (d: Date) => { const x = startOfDay(d); const day = x.getDay(); x.setDate(x.getDate() + (7 - day)); x.setHours(23, 59, 59, 999); return x }
const isOverdue = (n?: string | null) => n && new Date(n).getTime() < Date.now()

type Bucket = 'today' | 'week' | 'later' | 'saved'
type Mode = 'deck' | 'index'

const bucketLabel = (b: Bucket): string =>
  b === 'today' ? 'Today'
  : b === 'week' ? 'This week'
  : b === 'later' ? 'Later'
  : 'Saved'

const bucketHint = (b: Bucket): string =>
  b === 'today' ? 'Reminders that have already passed or land today'
  : b === 'week' ? 'Releases within the next seven days'
  : b === 'later' ? 'Releases further out'
  : 'Quietly saved, no reminder yet'

const bucketize = (item: WatchlistItem): Bucket => {
  if (!item.notification_enabled || !item.notify_at) return 'saved'
  const t = startOfDay(new Date()).getTime()
  const at = new Date(item.notify_at).getTime()
  if (at < t) return 'today'
  if (at <= endOfWeek(new Date()).getTime()) return 'week'
  return 'later'
}

const BUCKET_ORDER: Bucket[] = ['today', 'week', 'later', 'saved']

// ─── main component ──────────────────────────────────────
export function WatchlistList({ items, onEdit, onRefresh, loading = false }: WatchlistListProps) {
  const [mode, setMode] = useState<Mode>('deck')
  const [deletingId, setDeletingId] = useState<number | null>(null)
  const [selected, setSelected] = useState<WatchlistItem | null>(null)

  const handleDelete = useCallback(async (id: number) => {
    setDeletingId(id)
    try {
      await deleteWatchlistItem(id)
      onRefresh()
      setSelected(null)
    } finally {
      setDeletingId(null)
    }
  }, [onRefresh])

  const groups = useMemo(() => {
    const map: Record<Bucket, WatchlistItem[]> = { today: [], week: [], later: [], saved: [] }
    for (const item of items) map[bucketize(item)].push(item)
    const sortedInBucket = (arr: WatchlistItem[]) => [...arr].sort((a, b) => {
      const aN = a.notify_at ? new Date(a.notify_at).getTime() : Infinity
      const bN = b.notify_at ? new Date(b.notify_at).getTime() : Infinity
      if (aN !== bN) return aN - bN
      const aC = a.created_at ? new Date(a.created_at).getTime() : 0
      const bC = b.created_at ? new Date(b.created_at).getTime() : 0
      return bC - aC
    })
    return BUCKET_ORDER.map(b => ({ key: b, items: sortedInBucket(map[b]) }))
      .filter(g => g.items.length > 0)
  }, [items])

  const flat = useMemo(() => groups.flatMap(g => g.items), [groups])
  const selectedIdx = useMemo(
    () => (selected ? flat.findIndex(i => i.id === selected.id) : -1),
    [flat, selected],
  )
  const navSelected = useCallback((dir: 1 | -1) => {
    setSelected(prev => {
      if (!prev) return prev
      const idx = flat.findIndex(i => i.id === prev.id)
      const next = idx + dir
      if (next < 0 || next >= flat.length) return prev
      return flat[next]
    })
  }, [flat])

  useEffect(() => {
    if (!selected) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') { e.preventDefault(); navSelected(-1) }
      else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') { e.preventDefault(); navSelected(1) }
      else if (e.key === 'Escape') { e.preventDefault(); setSelected(null) }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [selected, navSelected])

  // ── loading ────────────────────────────────────────────
  if (loading) {
    return (
      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 xl:grid-cols-5 gap-x-6 gap-y-10">
        {Array.from({ length: 10 }).map((_, i) => (
          <div key={i} className="space-y-2">
            <div className="aspect-[2/3] rounded-md bg-white/[0.025] border border-white/[0.04]" />
            <div className="h-3 w-2/3 rounded-sm bg-white/[0.04]" />
            <div className="h-2 w-1/3 rounded-sm bg-white/[0.03]" />
          </div>
        ))}
      </div>
    )
  }

  // ── empty ──────────────────────────────────────────────
  if (items.length === 0) {
    return (
      <div className="flex h-full min-h-[420px] items-center justify-center">
        <div className="grid grid-cols-1 md:grid-cols-[180px_1fr] gap-8 max-w-2xl items-center">
          {/* composition: visual stack on the left, copy on the right */}
          <div className="relative h-[240px] w-[180px] justify-self-center md:justify-self-start">
            <div className="absolute -left-4 top-4 size-[180px] rotate-[-6deg] rounded-md border border-white/[0.05] bg-[hsl(0_0%_7%)]" />
            <div className="absolute -right-4 top-8 size-[180px] rotate-[4deg] rounded-md border border-white/[0.05] bg-[hsl(0_0%_8%)]" />
            <div className="relative size-[180px] rounded-md border border-white/[0.1] bg-[hsl(0_0%_5%)] flex items-center justify-center">
              <Bookmark className="size-8 text-white/25" />
            </div>
          </div>
          <div className="space-y-3">
            <p className="text-[10px] font-medium uppercase tracking-[0.32em] text-white/35">Empty queue</p>
            <h2 className="text-[24px] font-semibold tracking-[-0.02em] text-white leading-[1.1]">
              Nothing saved yet
            </h2>
            <p className="text-[13px] text-white/45 leading-relaxed max-w-sm">
              Head to <span className="text-white/70">Discover</span>, search for a film or series,
              and add it here. Reminders and quiet saves both land in this queue.
            </p>
          </div>
        </div>
      </div>
    )
  }

  // ── render ─────────────────────────────────────────────
  return (
    <div className="flex flex-col min-h-0">
      {/* chrome */}
      <div className="shrink-0 flex items-center justify-between pb-4">
        <div className="flex items-center gap-3">
          <p className="text-[12px] text-white/45 tabular-nums">
            {items.length} {items.length === 1 ? 'title' : 'titles'} saved
            {items.filter(i => i.notification_enabled).length > 0 && (
              <>
                <span className="mx-2 text-white/15">·</span>
                <span className="text-white/65">{items.filter(i => i.notification_enabled).length} reminded</span>
              </>
            )}
          </p>
        </div>

        <div
          role="tablist"
          aria-label="Watchlist view mode"
          className="flex items-center rounded-md border border-white/[0.06] bg-white/[0.015] p-0.5"
        >
          {([
            { id: 'deck' as Mode, label: 'Deck', icon: LayoutGrid },
            { id: 'index' as Mode, label: 'Index', icon: Rows3 },
          ]).map(opt => (
            <button
              type="button"
              key={opt.id}
              role="tab"
              aria-selected={mode === opt.id}
              onClick={() => { setMode(opt.id); setSelected(null) }}
              className={cn(
                'flex items-center gap-1.5 rounded-[5px] px-2.5 py-1 text-[11px] font-medium transition-colors duration-150',
                mode === opt.id
                  ? 'bg-white/[0.08] text-white'
                  : 'text-white/35 hover:text-white/70'
              )}
            >
              <opt.icon className="size-3.5" />
              <span>{opt.label}</span>
            </button>
          ))}
        </div>
      </div>

      {/* how it works hint */}
      <div className="shrink-0 mb-4 flex items-start gap-2.5 rounded-lg border border-white/[0.06] bg-white/[0.015] px-3.5 py-2.5">
        <span className="mt-[3px] size-1.5 shrink-0 rounded-full bg-white/40" />
        <p className="text-[11.5px] leading-relaxed text-white/45">
          <span className="text-white/70 font-medium">Click any saved title</span> to open its details — view the reminder countdown, edit the schedule or notes, or remove it from the queue. Use the arrow keys to step through titles, or press <kbd className="rounded border border-white/10 bg-white/[0.04] px-1 py-px text-[10px] text-white/55">Esc</kbd> to close.
        </p>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto pr-1 pb-4">
        <div className="space-y-10">
          {groups.map(group => (
            <BucketSection
              key={group.key}
              bucket={group.key}
              items={group.items}
              mode={mode}
              selectedId={selected?.id ?? null}
              onSelect={setSelected}
            />
          ))}
        </div>
      </div>

      <AnimatePresence>
        {selected && (
          <DetailPane
            item={selected}
            onClose={() => setSelected(null)}
            onEdit={() => { onEdit(selected); setSelected(null) }}
            onDelete={() => handleDelete(selected.id)}
            isDeleting={deletingId === selected.id}
            hasPrev={selectedIdx > 0}
            hasNext={selectedIdx < flat.length - 1}
            onPrev={() => navSelected(-1)}
            onNext={() => navSelected(1)}
          />
        )}
      </AnimatePresence>
    </div>
  )
}

// ─── bucket section ──────────────────────────────────────
function BucketSection({
  bucket, items, mode, selectedId, onSelect,
}: {
  bucket: Bucket
  items: WatchlistItem[]
  mode: Mode
  selectedId: number | null
  onSelect: (item: WatchlistItem) => void
}) {
  return (
    <section aria-labelledby={`bucket-${bucket}`}>
      <header className="mb-3 flex items-end justify-between gap-4">
        <div className="min-w-0">
          <h2
            id={`bucket-${bucket}`}
            className="text-[16px] font-semibold tracking-[-0.01em] text-white/90"
          >
            {bucketLabel(bucket)}
          </h2>
          <p className="text-[11px] text-white/35 mt-0.5">{bucketHint(bucket)}</p>
        </div>
        <span className="text-[10px] tabular-nums text-white/30 shrink-0">{items.length}</span>
      </header>

      {mode === 'deck' ? (
        <DeckRow items={items} selectedId={selectedId} onSelect={onSelect} />
      ) : (
        <IndexList items={items} selectedId={selectedId} onSelect={onSelect} />
      )}
    </section>
  )
}

// ─── deck (poster scroller) ──────────────────────────────
function DeckRow({
  items, selectedId, onSelect,
}: {
  items: WatchlistItem[]
  selectedId: number | null
  onSelect: (item: WatchlistItem) => void
}) {
  const scrollerRef = useRef<HTMLDivElement>(null)
  const scrollBy = (dir: 1 | -1) => {
    const el = scrollerRef.current
    if (!el) return
    const card = el.querySelector<HTMLElement>('[data-deck-card]')
    if (!card) return
    el.scrollBy({ left: dir * (card.offsetWidth + 24), behavior: 'smooth' })
  }
  return (
    <div className="relative group/deck">
      <div className="pointer-events-none absolute inset-y-0 left-0 w-8 bg-gradient-to-r from-background to-transparent z-10" />
      <div className="pointer-events-none absolute inset-y-0 right-0 w-8 bg-gradient-to-l from-background to-transparent z-10" />

      <button
        type="button"
        aria-label="Scroll left"
        onClick={() => scrollBy(-1)}
        className="absolute -left-3 top-1/2 z-20 -translate-y-1/2 flex size-8 items-center justify-center rounded-full border border-white/[0.06] bg-[hsl(0_0%_7%)]/90 text-white/40 opacity-0 transition-opacity duration-150 hover:text-white group-hover/deck:opacity-100"
      >
        <ChevronLeft className="size-4" />
      </button>
      <button
        type="button"
        aria-label="Scroll right"
        onClick={() => scrollBy(1)}
        className="absolute -right-3 top-1/2 z-20 -translate-y-1/2 flex size-8 items-center justify-center rounded-full border border-white/[0.06] bg-[hsl(0_0%_7%)]/90 text-white/40 opacity-0 transition-opacity duration-150 hover:text-white group-hover/deck:opacity-100"
      >
        <ChevronRight className="size-4" />
      </button>

      <div
        ref={scrollerRef}
        className="no-scrollbar flex gap-6 overflow-x-auto scroll-smooth"
        role="list"
      >
        {items.map((it, i) => (
          <PosterCard
            data-deck-card
            key={it.id}
            item={it}
            index={i}
            isSelected={selectedId === it.id}
            onSelect={() => onSelect(it)}
          />
        ))}
      </div>
    </div>
  )
}

// ─── poster card ─────────────────────────────────────────
function PosterCard({
  item, index, isSelected, onSelect,
}: {
  item: WatchlistItem
  index: number
  isSelected: boolean
  onSelect: () => void
}) {
  const posterUrl = item.poster_path ? getTmdbImageUrl(item.poster_path, 'w300') : null
  const hasReminder = !!item.notification_enabled && !!item.notify_at
  const overdue = isOverdue(item.notify_at)
  return (
    <motion.button
      type="button"
      onClick={onSelect}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ delay: index * 0.03, duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
      className={cn(
        'group/poster relative w-[150px] sm:w-[170px] md:w-[200px] shrink-0 text-left focus:outline-none',
      )}
      aria-label={`Open ${item.title} details`}
    >
      <div
        className={cn(
          'relative aspect-[2/3] overflow-hidden rounded-md bg-[hsl(0_0%_5%)] border transition-colors duration-150',
          isSelected
            ? 'border-white/40 ring-1 ring-white/15'
            : 'border-white/[0.05] hover:border-white/20',
        )}
      >
        {posterUrl ? (
          <img
            src={posterUrl}
            alt={item.title}
            loading="lazy"
            className="h-full w-full object-cover transition-transform duration-500 group-hover/poster:scale-[1.02]"
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-white/10">
            {item.media_type === 'movie'
              ? <Film className="size-8" />
              : <Tv className="size-8" />
            }
          </div>
        )}

        {/* bottom 1px hairline for reminded items; doubles as a quiet status signal */}
        {hasReminder && (
          <div className={cn(
            'pointer-events-none absolute inset-x-2 bottom-2 h-px',
            overdue ? 'bg-amber-400/70' : 'bg-white/40',
          )} />
        )}

        {/* hover affordance — subtle "Open" cue so users know titles are clickable */}
        <div className="pointer-events-none absolute inset-0 flex items-end justify-end p-2 opacity-0 transition-opacity duration-200 group-hover/poster:opacity-100">
          <span className="inline-flex items-center gap-1 rounded-full bg-black/60 px-2 py-0.5 text-[9px] font-medium uppercase tracking-[0.16em] text-white/80 backdrop-blur-sm">
            Open
          </span>
        </div>
      </div>

      <div className="mt-2.5 space-y-0.5">
        <p className="text-[13px] font-medium leading-tight tracking-tight text-white/85 line-clamp-1">
          {item.title}
        </p>
        <p className="text-[11px] tabular-nums text-white/30 line-clamp-1">
          {hasReminder
            ? formatLocalReleaseTime(item.notify_at!)
            : item.release_date
              ? new Date(item.release_date).getFullYear()
              : '—'}
        </p>
      </div>
    </motion.button>
  )
}

// ─── index (table) ───────────────────────────────────────
function IndexList({
  items, selectedId, onSelect,
}: {
  items: WatchlistItem[]
  selectedId: number | null
  onSelect: (item: WatchlistItem) => void
}) {
  return (
    <div className="overflow-hidden rounded-lg border border-white/[0.05]" role="list">
      <div className="hidden sm:grid grid-cols-[40px_1fr_140px_100px] gap-4 px-4 py-2 text-[10px] font-medium uppercase tracking-[0.18em] text-white/25 border-b border-white/[0.05] bg-white/[0.01]">
        <span aria-hidden />
        <span>Title</span>
        <span>Reminder</span>
        <span className="text-right">Type</span>
      </div>
      <div className="divide-y divide-white/[0.04]">
        {items.map((it, i) => (
          <IndexRow
            key={it.id}
            item={it}
            index={i}
            isSelected={selectedId === it.id}
            onSelect={() => onSelect(it)}
          />
        ))}
      </div>
    </div>
  )
}

function IndexRow({
  item, isSelected, onSelect,
}: {
  item: WatchlistItem
  index: number
  isSelected: boolean
  onSelect: () => void
}) {
  const posterUrl = item.poster_path ? getTmdbImageUrl(item.poster_path, 'w185') : null
  const hasReminder = !!item.notification_enabled && !!item.notify_at
  return (
    <motion.button
      type="button"
      onClick={onSelect}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className={cn(
        'w-full text-left transition-colors duration-150',
        isSelected
          ? 'bg-white/[0.05]'
          : 'hover:bg-white/[0.025]',
      )}
    >
      <div className="grid grid-cols-[40px_1fr_140px_100px] items-center gap-4 px-4 py-3">
        <div className="h-12 w-8 overflow-hidden rounded-[3px] bg-[hsl(0_0%_5%)] border border-white/[0.05]">
          {posterUrl ? (
            <img src={posterUrl} alt="" className="h-full w-full object-cover" loading="lazy" />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-white/10">
              {item.media_type === 'movie'
                ? <Film className="size-3.5" />
                : <Tv className="size-3.5" />
              }
            </div>
          )}
        </div>

        <div className="min-w-0 space-y-0.5">
          <p className="truncate text-[13px] font-medium text-white/85">{item.title}</p>
          <p className="truncate text-[11px] text-white/30">
            {item.notes?.trim() || (item.release_date ? new Date(item.release_date).getFullYear() : 'No notes')}
          </p>
        </div>

        <div className="text-[12px] tabular-nums text-white/45 truncate">
          {hasReminder ? (
            <span className="inline-flex items-center gap-1.5">
              <CalendarDays className="size-3 text-white/30" />
              {formatLocalReleaseTime(item.notify_at!)}
            </span>
          ) : (
            <span className="text-white/20">—</span>
          )}
        </div>

        <div className="text-right text-[10px] font-medium uppercase tracking-[0.18em] text-white/35">
          {item.media_type === 'movie' ? 'Film' : 'Series'}
        </div>
      </div>
    </motion.button>
  )
}

// ─── detail pane ─────────────────────────────────────────
function DetailPane({
  item, onClose, onEdit, onDelete, isDeleting,
  hasPrev, hasNext, onPrev, onNext,
}: {
  item: WatchlistItem
  onClose: () => void
  onEdit: () => void
  onDelete: () => void
  isDeleting: boolean
  hasPrev: boolean
  hasNext: boolean
  onPrev: () => void
  onNext: () => void
}) {
  const posterUrl = item.poster_path ? getTmdbImageUrl(item.poster_path, 'w185') : null
  const backdropUrl = item.poster_path ? getTmdbImageUrl(item.poster_path, 'w500') : null
  const hasReminder = !!item.notification_enabled && !!item.notify_at
  const overdue = isOverdue(item.notify_at)

  return (
    <motion.aside
      role="dialog"
      aria-label={`${item.title} details`}
      initial={{ y: 24, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      exit={{ y: 24, opacity: 0 }}
      transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
      className="shrink-0 mt-4 overflow-hidden rounded-xl border border-white/[0.06] bg-[hsl(0_0%_6%)]/80 backdrop-blur-md"
    >
      {/* backdrop: faint poster blur as a quiet visual anchor */}
      {backdropUrl && (
        <div className="relative h-16 overflow-hidden">
          <div
            className="absolute inset-0 opacity-25 scale-110 blur-2xl"
            style={{ backgroundImage: `url(${backdropUrl})`, backgroundSize: 'cover', backgroundPosition: 'center' }}
          />
          <div className="absolute inset-0 bg-gradient-to-b from-transparent to-[hsl(0_0%_6%)]/95" />
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-[64px_1fr_auto] gap-4 md:gap-6 p-4 md:p-5 items-start">
        <div className="hidden md:block aspect-[2/3] w-16 overflow-hidden rounded-md bg-[hsl(0_0%_5%)] border border-white/[0.06]">
          {posterUrl ? (
            <img src={posterUrl} alt="" className="h-full w-full object-cover" />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-white/10">
              {item.media_type === 'movie'
                ? <Film className="size-4" />
                : <Tv className="size-4" />
              }
            </div>
          )}
        </div>

        <div className="min-w-0 space-y-1.5">
          <p className="text-[15px] font-medium text-white leading-tight">{item.title}</p>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-white/40">
            <span className="uppercase tracking-[0.18em] text-white/35">{item.media_type === 'movie' ? 'Film' : 'Series'}</span>
            {item.release_date && !Number.isNaN(new Date(item.release_date).getTime()) && (
              <>
                <span className="text-white/15">·</span>
                <span className="tabular-nums">{new Date(item.release_date).getFullYear()}</span>
              </>
            )}
            {hasReminder && (
              <>
                <span className="text-white/15">·</span>
                <span className="inline-flex items-center gap-1.5">
                  <Bell className={cn('size-3', overdue ? 'text-amber-400/80' : 'text-white/40')} />
                  <span className={cn('tabular-nums', overdue && 'text-amber-400/80')}>{formatLocalReleaseTime(item.notify_at!)}</span>
                </span>
              </>
            )}
          </div>
          {item.notes?.trim() && (
            <p className="text-[12px] text-white/40 leading-relaxed pt-1 line-clamp-2">{item.notes}</p>
          )}
          {hasReminder && (
            <div className="pt-1">
              <CountdownTimer target={item.notify_at!} compact />
            </div>
          )}
        </div>

        <div className="flex items-center gap-1.5 md:flex-col md:items-end md:gap-2">
          <div className="flex items-center gap-1 order-2 md:order-1">
            <Button
              type="button"
              onClick={onPrev}
              disabled={!hasPrev}
              aria-label="Previous"
              size="icon"
              variant="ghost"
              className="size-8 rounded-md text-white/40 hover:text-white hover:bg-white/[0.06] disabled:opacity-30"
            >
              <ChevronLeft className="size-4" />
            </Button>
            <Button
              type="button"
              onClick={onNext}
              disabled={!hasNext}
              aria-label="Next"
              size="icon"
              variant="ghost"
              className="size-8 rounded-md text-white/40 hover:text-white hover:bg-white/[0.06] disabled:opacity-30"
            >
              <ChevronRight className="size-4" />
            </Button>
          </div>
          <div className="flex items-center gap-1.5 order-1 md:order-2">
            <Button
              type="button"
              onClick={onEdit}
              className="h-8 px-3 rounded-md bg-white text-black text-[12px] font-medium hover:bg-white/90"
            >
              <Pencil className="size-3.5 mr-1.5" />
              Edit
            </Button>
            <Button
              type="button"
              onClick={onDelete}
              disabled={isDeleting}
              variant="ghost"
              className="h-8 px-3 rounded-md text-white/45 hover:text-red-400 hover:bg-red-400/[0.08] text-[12px] font-medium"
            >
              {isDeleting ? <Loader2 className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5 mr-1.5" />}
              {isDeleting ? 'Removing' : 'Remove'}
            </Button>
            <Button
              type="button"
              onClick={onClose}
              aria-label="Close"
              size="icon"
              variant="ghost"
              className="size-8 rounded-md text-white/35 hover:text-white hover:bg-white/[0.06]"
            >
              <X className="size-4" />
            </Button>
          </div>
        </div>
      </div>
    </motion.aside>
  )
}
