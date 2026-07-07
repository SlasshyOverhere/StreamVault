import { useState } from 'react'
import { 
  Bell, Calendar, Clock, Trash2, Edit2, Play, Pause, 
  Film, Tv, Loader2, History
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { 
  MovieReminder, 
  getTmdbImageUrl, 
  deleteMovieReminder, 
  setMovieReminderActive 
} from '@/services/api'
import { cn } from '@/lib/utils'
import { LazyMotion, domAnimation, m, AnimatePresence } from 'framer-motion'
import { CountdownTimer } from './CountdownTimer'
import { formatLocalReleaseTime } from './CountdownTimer.utils'
import { ScrollArea } from '@/components/ui/scroll-area'

interface RemindersListProps {
  reminders: MovieReminder[]
  onEdit: (reminder: MovieReminder) => void
  onRefresh: () => void
}

const containerVariants = {
  hidden: { opacity: 0 },
  visible: {
    opacity: 1,
    transition: {
      staggerChildren: 0.05
    }
  }
}

const itemVariants = {
  hidden: { opacity: 0, y: 15 },
  visible: { 
    opacity: 1, 
    y: 0,
    transition: {
      type: 'spring',
      stiffness: 400,
      damping: 30
    }
  }
} as const

export function RemindersList({
  reminders,
  onEdit,
  onRefresh
}: RemindersListProps) {
  const [deletingId, setDeletingId] = useState<number | null>(null)

  const handleDelete = async (id: number) => {
    setDeletingId(id)
    try {
      await deleteMovieReminder(id)
      onRefresh()
    } catch (error) {
      console.error('Failed to delete reminder:', error)
    } finally {
      setDeletingId(null)
    }
  }

  const handleToggleActive = async (id: number, isActive: boolean) => {
    try {
      await setMovieReminderActive(id, !isActive)
      onRefresh()
    } catch (error) {
      console.error('Failed to toggle reminder status:', error)
    }
  }

  const upcoming = reminders
    .filter(r => new Date(r.reminder_at) > new Date())
    .sort((a, b) => new Date(a.reminder_at).getTime() - new Date(b.reminder_at).getTime())
  
  const past = reminders
    .filter(r => new Date(r.reminder_at) <= new Date())
    .sort((a, b) => new Date(b.reminder_at).getTime() - new Date(a.reminder_at).getTime())

  if (reminders.length === 0) {
    return (
      <LazyMotion features={domAnimation}>
      <m.div
        initial={{ opacity: 0, scale: 0.9 }}
        animate={{ opacity: 1, scale: 1 }}
        className="flex h-full min-h-0 flex-col items-center justify-center overflow-hidden text-center gap-y-6"
      >
        <div className="relative">
          <div className="absolute inset-0 bg-white/10 blur-3xl rounded-full" />
          <div className="relative size-16 rounded-2xl bg-white/5 border border-white/10 flex items-center justify-center shadow-2xl backdrop-blur-xl">
            <Bell className="size-8 text-white/30" />
          </div>
        </div>
        <div className="gap-y-2 max-w-sm">
          <h2 className="text-xl font-black text-white tracking-tight">No reminders yet</h2>
          <p className="text-sm text-white/40 leading-relaxed font-medium px-6">
            Discover movies and TV shows and set reminders to get notified when they're released or available.
          </p>
        </div>
      </m.div>
      </LazyMotion>
    )
  }

  return (
    <LazyMotion features={domAnimation}>
    <div className={cn(
      "grid h-full min-h-0 gap-4 overflow-hidden",
      upcoming.length > 0 ? "grid-rows-[minmax(0,200px)_minmax(0,1fr)]" : "grid-rows-[minmax(0,1fr)]"
    )}>
      {upcoming.length > 0 && (
        <div className="shrink-0 overflow-hidden">
          <NextUpPanel reminder={upcoming[0]} />
        </div>
      )}

      <section className="flex min-h-0 flex-col gap-6 overflow-hidden p-1">
        <div className="flex shrink-0 items-center justify-between gap-4 px-1">
          <div className="flex items-center gap-3">
            <div className="size-9 rounded-xl bg-white/[0.04] flex items-center justify-center border border-white/10">
            {upcoming.length > 0 ? <Calendar className="size-4 text-white/40" /> : <History className="size-4 text-white/40" />}
            </div>
            <div className="flex flex-col">
              <h3 className="text-[10px] font-black uppercase tracking-[0.2em] text-white/70">
                {upcoming.length > 0 ? 'Upcoming Reminders' : 'Watch History'}
              </h3>
              <span className="text-[9px] font-bold text-white/25 uppercase tracking-widest">
                {upcoming.length > 0 ? `${upcoming.length} shows scheduled` : `${past.length} past reminders`}
              </span>
            </div>
          </div>

          <div className="hidden h-8 items-center rounded-full border border-white/[0.08] bg-white/[0.03] px-3 text-[8px] font-black uppercase tracking-[0.2em] text-white/30 sm:flex">
            Local time
          </div>
        </div>

        <ScrollArea className="flex-1 min-h-0 pr-3">
          <div className="pb-5">
            {upcoming.length > 0 ? (
              <m.div 
                variants={containerVariants}
                initial="hidden"
                animate="visible"
                className="grid content-start grid-cols-1 gap-4 xl:grid-cols-2"
              >
                <AnimatePresence mode="popLayout">
                  {upcoming.map((reminder) => (
                    <ReminderCard 
                      key={reminder.id}
                      reminder={reminder}
                      onEdit={() => onEdit(reminder)}
                      onDelete={() => handleDelete(reminder.id)}
                      onToggle={() => handleToggleActive(reminder.id, reminder.is_active)}
                      isDeleting={deletingId === reminder.id}
                    />
                  ))}
                </AnimatePresence>
              </m.div>
            ) : (
              <div className="grid content-start grid-cols-1 gap-4 xl:grid-cols-2">
                {past.map((reminder) => (
                  <ReminderCard
                    key={reminder.id}
                    reminder={reminder}
                    onEdit={() => onEdit(reminder)}
                    onDelete={() => handleDelete(reminder.id)}
                    onToggle={() => handleToggleActive(reminder.id, reminder.is_active)}
                    isDeleting={deletingId === reminder.id}
                  />
                ))}
              </div>
            )}
          </div>
        </ScrollArea>
      </section>
    </div>
    </LazyMotion>
  )
}

function NextUpPanel({ reminder }: { reminder: MovieReminder }) {
  return (
    <LazyMotion features={domAnimation}>
    <m.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      className="group relative h-full overflow-hidden rounded-[2rem] border border-white/[0.05] bg-white/[0.02] p-8 shadow-inner"
    >
      {/* Background Effect */}
      {reminder.poster_path && (
        <div className="absolute inset-0 z-0">
          <img
            src={getTmdbImageUrl(reminder.poster_path, 'original') || ''}
            alt=""
            className="h-full w-full object-cover opacity-5 blur-3xl saturate-200"
          />
          <div className="absolute inset-0 bg-gradient-to-br from-black/80 via-black/40 to-black/80" />
        </div>
      )}

      <div className="relative z-10 flex flex-col md:flex-row md:items-center justify-between gap-8 h-full">
        {/* Left: Info */}
        <div className="min-w-0 flex-1 space-y-4">
          <div className="inline-flex items-center gap-2 rounded-full border border-white/20 bg-white/10 px-4 py-1.5 text-[10px] font-black uppercase tracking-[0.2em] text-white">
            <div className="size-2 animate-pulse rounded-full bg-white shadow-[0_0_10px_rgba(255,255,255,0.6)]" />
            Next Airing
          </div>

          <div className="space-y-1.5">
            <h2 className="truncate text-3xl md:text-4xl font-black tracking-tighter text-white leading-tight">
              {reminder.title}
            </h2>

            <div className="flex flex-wrap items-center gap-4 text-[11px] font-bold text-white/50">
              <div className="flex items-center gap-2.5 bg-black/20 px-3 py-1.5 rounded-lg backdrop-blur">
                <Clock className="size-4 opacity-60" />
                <span>{formatLocalReleaseTime(reminder.reminder_at)}</span>
              </div>

              {reminder.season_number != null && (
                <div className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 font-black text-white/80">
                  S{reminder.season_number} • E{reminder.episode_number}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Right: Countdown */}
        <div className="shrink-0 pt-4 md:pt-0">
          <CountdownTimer
            target={reminder.reminder_at}
            banner
            className="md:justify-end"
          />
        </div>
      </div>
    </m.div>
    </LazyMotion>
  )
}
function ReminderCard({ 
  reminder, 
  onEdit, 
  onDelete, 
  onToggle,
  isDeleting 
}: { 
  reminder: MovieReminder
  onEdit: () => void
  onDelete: () => void
  onToggle: () => void
  isDeleting: boolean
}) {
  const reminderDate = new Date(reminder.reminder_at)
  const isPast = reminderDate < new Date()
  const sourceLabel = reminder.source === 'tvmaze' ? 'TVmaze' : (reminder.source === 'tmdb' ? 'TMDB' : 'Manual')
  
  return (
    <m.div
      variants={itemVariants}
      layout
      className={cn(
        "group relative min-h-[118px] overflow-hidden rounded-[1.5rem] p-4 transition-all duration-300 hover:bg-white/[0.03]",
        !reminder.is_active && "opacity-55 grayscale-[0.2]"
      )}
    >
      <div className="pointer-events-none absolute inset-0 bg-gradient-to-r from-white/[0.025] via-transparent to-transparent opacity-0 transition-opacity duration-300 group-hover:opacity-100" />

      <div className="relative z-10 flex h-full items-center gap-4">
        <div className="shrink-0 w-16 aspect-[2/3] rounded-xl overflow-hidden bg-neutral-950 border border-white/10 shadow-xl relative">
          {reminder.poster_path ? (
            <img 
              src={getTmdbImageUrl(reminder.poster_path, 'w185') || ''} 
              alt={reminder.title}
              className="w-full h-full object-cover"
            />
          ) : (
            <div className="w-full h-full flex items-center justify-center text-white/10">
              {reminder.media_type === 'movie' ? <Film className="size-5" /> : <Tv className="size-5" />}
            </div>
          )}
          <div className="absolute bottom-1.5 left-1.5 right-1.5 rounded-md bg-black/70 px-1.5 py-0.5 text-center text-[7px] font-black uppercase tracking-widest text-white/60 backdrop-blur-md">
            {reminder.media_type}
          </div>
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0 space-y-1.5">
              <h4 className="truncate text-[15px] font-black leading-none text-white">
                {reminder.title}
              </h4>
              <div className="flex flex-wrap items-center gap-2">
                <span className="rounded-md bg-white/[0.04] px-2 py-1 text-[8px] font-black uppercase tracking-widest text-white/35">
                  {sourceLabel}
                </span>
                {reminder.season_number != null && (
                  <span className="rounded-md border border-white/[0.08] bg-white/[0.025] px-2 py-1 text-[8px] font-black uppercase tracking-widest text-white/50">
                    S{reminder.season_number} E{reminder.episode_number}
                  </span>
                )}
              </div>
            </div>

            <div className="flex shrink-0 items-center gap-1">
              <Button 
                variant="ghost" 
                size="icon" 
                className="size-8 rounded-lg text-white/30 hover:text-white hover:bg-white/10 transition-all"
                onClick={(e) => { e.stopPropagation(); onEdit(); }}
              >
                <Edit2 className="size-3.5" />
              </Button>
              <Button 
                variant="ghost" 
                size="icon" 
                className="size-8 rounded-lg text-white/30 hover:text-red-400 hover:bg-red-400/10 transition-all"
                onClick={(e) => { e.stopPropagation(); onDelete(); }}
                disabled={isDeleting}
              >
                {isDeleting ? <Loader2 className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
              </Button>
            </div>
          </div>

          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div className={cn(
              "flex h-8 min-w-0 items-center gap-2 rounded-xl border border-white/[0.06] bg-black/20 px-3 text-[10px] font-bold",
              isPast ? "text-white/20" : "text-white/55"
            )}>
              <Clock className="size-3.5 shrink-0 opacity-40" />
              <span className="truncate leading-none">
                {reminderDate.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} at {reminderDate.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
              </span>
            </div>

            <div className="flex shrink-0 items-center gap-2">
              {!isPast && (
                <CountdownTimer
                  target={reminder.reminder_at}
                  compact
                  className="justify-center"
                />
              )}
              <button
                type="button"
                onClick={(e) => { e.stopPropagation(); onToggle(); }}
                className={cn(
                  "size-8 rounded-lg flex items-center justify-center transition-all border border-white/[0.08] bg-white/[0.03]",
                  reminder.is_active
                    ? "text-white/35 hover:text-white hover:bg-white/10"
                    : "text-white/60 hover:text-white hover:bg-white/10"
                )}
                title={reminder.is_active ? "Pause" : "Resume"}
              >
                {reminder.is_active ? <Pause className="size-3.5" /> : <Play className="size-3.5" />}
              </button>
            </div>
          </div>
        </div>
      </div>
    </m.div>
  )
}
