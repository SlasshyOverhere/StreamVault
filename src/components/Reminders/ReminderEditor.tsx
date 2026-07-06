import { useState, useEffect } from 'react'
import { 
  Bell, Loader2, Info, Calendar as CalendarIcon, StickyNote
} from 'lucide-react'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { 
  getTmdbReleaseSchedule, 
  MovieReminderInput, 
  MovieReminder,
  TmdbReleaseSchedule
} from '@/services/api'
import { CountdownTimer } from './CountdownTimer'
import { formatLocalReleaseTime, getLocalTimezoneLabel } from './CountdownTimer.utils'
import { LazyMotion, domAnimation, m, AnimatePresence } from 'framer-motion'

interface ReminderEditorProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialData?: Partial<MovieReminderInput> | MovieReminder
  onSave: (reminder: MovieReminderInput) => Promise<void>
}

// Helper to convert UTC string to local datetime-local value
const utcToLocalDatetime = (utcString?: string | null): string => {
  if (!utcString) return ''
  const date = new Date(utcString)
  if (isNaN(date.getTime())) return ''
  
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  const hours = String(date.getHours()).padStart(2, '0')
  const minutes = String(date.getMinutes()).padStart(2, '0')
  
  return `${year}-${month}-${day}T${hours}:${minutes}`
}

// Helper to convert datetime-local value to UTC string
const localDatetimeToUtc = (localString: string): string => {
  if (!localString) return new Date().toISOString()
  const date = new Date(localString)
  return date.toISOString()
}

const formatEpisodeCode = (season?: number | null, episode?: number | null): string | null => {
  if (season == null || episode == null) return null
  return `S${season}E${episode}`
}

const formatTvReminderTitle = (
  rawTitle: string,
  season?: number | null,
  episode?: number | null
): string => {
  const episodeCode = formatEpisodeCode(season, episode)
  if (!episodeCode) return rawTitle.trim()

  const cleaned = rawTitle
    .trim()
    .replace(/\s+-\s+S\d+E\d+$/i, '')
    .replace(/\s+S\d+E\d+$/i, '')

  if (cleaned.toLowerCase().endsWith(episodeCode.toLowerCase())) {
    return cleaned
  }

  return `${cleaned} - ${episodeCode}`
}

const typedData = (d: Partial<MovieReminderInput> | MovieReminder | undefined | null) => (d ?? {}) as Partial<MovieReminderInput> & Partial<MovieReminder>

export function ReminderEditor({
  open,
  onOpenChange,
  initialData,
  onSave
}: ReminderEditorProps) {
  const [loading, setLoading] = useState(false)
  const [suggesting, setSuggesting] = useState(false)
  const [schedule, setSchedule] = useState<TmdbReleaseSchedule | null>(null)
  
  const [title, setTitle] = useState('')
  const [titleError, setTitleError] = useState('')
  const [reminderAt, setReminderAt] = useState('')
  const [reminderAtError, setReminderAtError] = useState('')
  const [notes, setNotes] = useState('')
  const [isActive, setIsActive] = useState(true)
  const [source, setSource] = useState('manual')

  useEffect(() => {
    if (!open) return

    setTitleError('')
    setReminderAtError('')

    const setup = async () => {
      if (initialData) {
        const data = typedData(initialData)
        
        setTitle(data.title || '')
        setReminderAt(utcToLocalDatetime(data.reminder_at || data.reminderAt))
        setNotes(data.notes || '')
        setIsActive(data.is_active ?? data.isActive ?? true)
        setSource(data.source || 'manual')

        // If we have TMDB info but no reminder time, fetch suggestion
        const tmdbId = data.tmdb_id || data.tmdbId
        const mediaType = data.media_type || data.mediaType || 'movie'
        const seasonNumber = data.season_number ?? data.seasonNumber
        const episodeNumber = data.episode_number ?? data.episodeNumber

        if (tmdbId && !data.reminder_at && !data.reminderAt) {
          setSuggesting(true)
          try {
            const sched = await getTmdbReleaseSchedule(
              Number(tmdbId),
              mediaType as 'movie' | 'tv',
              seasonNumber,
              episodeNumber,
              imdbId ?? null,
            )
            setSchedule(sched)
            if (sched.title) {
              setTitle(sched.title)
            }
            if (sched.suggestedReminderAt) {
              setReminderAt(utcToLocalDatetime(sched.suggestedReminderAt))
              setSource(sched.source || 'tmdb')
            }
          } catch (error) {
            console.error('Failed to get schedule:', error)
          } finally {
            setSuggesting(false)
          }
        }
      } else {
        setTitle('')
        setReminderAt(utcToLocalDatetime(new Date().toISOString()))
        setNotes('')
        setIsActive(true)
        setSource('manual')
        setSchedule(null)
      }
    }

    setup()
  }, [open, initialData])

  const handleSave = async () => {
    let hasError = false
    if (!title.trim()) {
      setTitleError('Title is required')
      hasError = true
    } else {
      setTitleError('')
    }
    if (!reminderAt) {
      setReminderAtError('Scheduled time is required')
      hasError = true
    } else {
      setReminderAtError('')
    }
    if (hasError) return

    setLoading(true)
    try {
      const data = typedData(initialData)
      const mediaType = data?.media_type || data?.mediaType || 'movie'
      const seasonNumber = schedule?.seasonNumber ?? data?.season_number ?? data?.seasonNumber ?? null
      const episodeNumber = schedule?.episodeNumber ?? data?.episode_number ?? data?.episodeNumber ?? null
      const trackingMode = data?.tracking_mode || data?.trackingMode || 'single'
      const trackingSeasonNumber = data?.tracking_season_number ?? data?.trackingSeasonNumber ?? seasonNumber ?? null
      const savedTitle = mediaType === 'tv'
        ? formatTvReminderTitle(title, seasonNumber, episodeNumber)
        : title
      const input: MovieReminderInput = {
        tmdbId: String(data?.tmdb_id || data?.tmdbId || ''),
        mediaType,
        title: savedTitle,
        posterPath: data?.poster_path || data?.posterPath || null,
        seasonNumber,
        episodeNumber,
        releaseDate: schedule?.releaseDate || data?.release_date || data?.releaseDate || null,
        reminderAt: localDatetimeToUtc(reminderAt),
        source,
        trackingMode,
        trackingSeasonNumber,
        notes,
        isActive
      }
      await onSave(input)
      onOpenChange(false)
    } catch (error) {
      console.error('Failed to save reminder:', error)
    } finally {
      setLoading(false)
    }
  }

  const isEdit = initialData && 'id' in initialData

  return (
    <LazyMotion features={domAnimation}>
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px] bg-background/95 backdrop-blur-[32px] border border-white/10 text-white p-0 overflow-hidden shadow-[0_0_120px_rgba(0,0,0,0.8)] rounded-[2rem]">
        <div className="absolute inset-0 bg-gradient-to-b from-white/[0.02] to-transparent pointer-events-none" />
        
        <DialogHeader className="p-8 pb-4 relative z-10">
          <DialogTitle className="text-2xl font-black tracking-tight flex items-center gap-3">
            <div className="size-10 rounded-2xl bg-white/5 border border-white/10 flex items-center justify-center shadow-inner">
              <Bell className="size-5 text-white/60" />
            </div>
            <div className="flex flex-col items-start">
              <span>{isEdit ? 'Update Details' : 'Set Reminder'}</span>
              <span className="text-[10px] font-bold text-white/20 uppercase tracking-widest leading-none mt-1">Configure your viewing schedule</span>
            </div>
          </DialogTitle>
        </DialogHeader>

        <div className="px-8 pb-8 space-y-6 relative z-10 overflow-y-auto max-h-[70vh] custom-scrollbar">
          <div className="space-y-6">
            <div className="space-y-3">
              <Label htmlFor="title" className="text-[10px] font-black uppercase tracking-[0.2em] text-white/30 ml-1">Title</Label>
              <div className="relative group">
                <Input 
                  id="title" 
                  value={title} 
                  onChange={e => { setTitle(e.target.value); setTitleError('') }}
                  placeholder="Movie or TV Show Name"
                  aria-label="Reminder title"
                  aria-describedby="title-error"
                  aria-invalid={!!titleError}
                  className="bg-white/5 border-white/10 focus:border-white/20 focus:bg-white/[0.08] h-14 rounded-2xl px-5 text-base font-bold placeholder:text-white/10 transition-all shadow-inner"
                />
                {titleError && (
                  <p id="title-error" className="text-[10px] font-bold text-red-400 ml-1 mt-1">{titleError}</p>
                )}
              </div>
            </div>

            <div className="space-y-3">
              <Label htmlFor="reminderAt" className="text-[10px] font-black uppercase tracking-[0.2em] text-white/30 ml-1">Scheduled Time</Label>
              <div className="relative group">
                <CalendarIcon className="absolute left-5 top-1/2 -translate-y-1/2 size-4 text-white/20 group-focus-within:text-white/60 transition-colors" />
                <Input 
                  id="reminderAt" 
                  type="datetime-local"
                  value={reminderAt} 
                  onChange={e => {
                    setReminderAt(e.target.value)
                    setReminderAtError('')
                    setSource('manual')
                  }}
                  aria-label="Reminder scheduled time"
                  aria-describedby="reminderAt-error"
                  aria-invalid={!!reminderAtError}
                  className="bg-white/5 border-white/10 focus:border-white/20 focus:bg-white/[0.08] h-14 rounded-2xl pl-12 pr-5 text-base font-bold text-white [color-scheme:dark] transition-all shadow-inner"
                />
                {reminderAtError && (
                  <p id="reminderAt-error" className="text-[10px] font-bold text-red-400 ml-1 mt-1">{reminderAtError}</p>
                )}
                <AnimatePresence>
                  {suggesting && (
                    <m.div 
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0 }}
                      className="absolute right-5 top-1/2 -translate-y-1/2"
                    >
                      <Loader2 className="size-5 animate-spin text-white/20" />
                    </m.div>
                  )}
                </AnimatePresence>
              </div>

              <AnimatePresence>
                {schedule && (source === 'tmdb' || source === 'tvmaze') && (
                  <m.div 
                    initial={{ opacity: 0, y: -10 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="flex items-start gap-3 p-4 rounded-2xl bg-emerald-500/5 border border-emerald-500/10"
                  >
                    <Info className="size-4 text-emerald-400/60 shrink-0 mt-0.5" />
                    <div className="text-[11px] text-emerald-400/60 leading-relaxed font-medium">
                      Smart suggestion from <span className="text-emerald-400 font-black">{schedule.source === 'tvmaze' ? 'TVmaze' : 'TMDB'}</span>: 
                      <span className="text-emerald-300 font-bold ml-1">
                        {schedule.precision === 'datetime' && schedule.suggestedReminderAt 
                          ? formatLocalReleaseTime(schedule.suggestedReminderAt) 
                          : schedule.releaseDate}
                      </span>.
                    </div>
                  </m.div>
                )}
              </AnimatePresence>
              
              {reminderAt && (
                <div className="space-y-4 pt-2">
                  <CountdownTimer
                    target={localDatetimeToUtc(reminderAt)}
                    label="Release Countdown"
                    expiredLabel="Release time reached"
                    className="bg-white/[0.02] border-white/10 shadow-inner"
                  />
                  <div className="flex items-center gap-2 px-1">
                     <div className="size-1.5 rounded-full bg-white/20" />
                     <p className="text-[10px] font-bold text-white/20 uppercase tracking-widest">
                       Targeting {getLocalTimezoneLabel()} system clock
                     </p>
                  </div>
                </div>
              )}
            </div>

            <div className="space-y-3">
              <Label htmlFor="notes" className="text-[10px] font-black uppercase tracking-[0.2em] text-white/30 ml-1">Personal Notes</Label>
              <div className="relative group">
                <StickyNote className="absolute left-5 top-1/2 -translate-y-1/2 size-4 text-white/20 group-focus-within:text-white/60 transition-colors" />
                <Input 
                  id="notes" 
                  value={notes} 
                  onChange={e => setNotes(e.target.value)}
                  placeholder="Notes for yourself..."
                  className="bg-white/5 border-white/10 focus:border-white/20 focus:bg-white/[0.08] h-14 rounded-2xl pl-12 pr-5 text-base font-bold placeholder:text-white/10 transition-all shadow-inner"
                />
              </div>
            </div>

            <div className="flex items-center justify-between p-5 rounded-[1.5rem] bg-white/[0.03] border border-white/10 shadow-inner">
              <div className="space-y-1">
                <Label className="text-sm font-black text-white">Enable Notifications</Label>
                <p className="text-[10px] text-white/20 font-bold uppercase tracking-widest">
                  Track and alert on release
                </p>
              </div>
              <Switch 
                checked={isActive} 
                onCheckedChange={setIsActive}
                className="data-[state=checked]:bg-white/90"
              />
            </div>
          </div>
        </div>

        <DialogFooter className="p-8 pt-4 bg-white/[0.02] border-t border-white/5 flex gap-3 sm:gap-0">
          <Button 
            variant="ghost" 
            onClick={() => onOpenChange(false)}
            className="text-white/40 hover:text-white hover:bg-white/5 rounded-2xl h-14 px-8 font-black uppercase tracking-widest text-[11px] transition-all"
          >
            Cancel
          </Button>
          <Button 
            onClick={handleSave}
            disabled={loading || !title || !reminderAt}
            className="bg-white text-black hover:bg-neutral-200 font-black uppercase tracking-[0.2em] text-[11px] rounded-2xl h-14 px-10 shadow-[0_20px_50px_rgba(255,255,255,0.1)] transition-all active:scale-95"
          >
            {loading ? <Loader2 className="size-4 animate-spin" /> : (isEdit ? 'Save Changes' : 'Confirm Reminder')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
    </LazyMotion>
  )
}
