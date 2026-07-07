import { useState, useEffect } from 'react'
import { 
  Calendar, Clock, Bell, Loader2,
  LayoutGrid, Star, PlayCircle
} from 'lucide-react'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { 
  getMovieDetails, 
  getTvDetails, 
  getTvSeasonEpisodes, 
  getTmdbImageUrl,
  TmdbMovieDetails,
  TmdbShowDetails,
  TmdbSeasonDetails
} from '@/services/api'
import { cn } from '@/lib/utils'
import { ImdbDetailsPanel } from '@/components/ImdbDetailsPanel'
import { CountdownTimer } from './CountdownTimer'
import { isFutureReleaseTarget, parseReleaseTarget } from './CountdownTimer.utils'
import { motion, AnimatePresence } from 'framer-motion'

interface TmdbDetailsModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  tmdbId: number
  mediaType: 'movie' | 'tv'
  imdbId?: string | null
  onSetReminder: (data: {
    tmdbId: string,
    mediaType: 'movie' | 'tv',
    title: string,
    posterPath?: string | null,
    seasonNumber?: number | null,
    episodeNumber?: number | null,
    releaseDate?: string | null,
    trackingMode?: 'single' | 'tv_season',
    trackingSeasonNumber?: number | null
  }) => void
  onAddToWatchlist: (data: {
    tmdbId: string,
    mediaType: 'movie' | 'tv',
    title: string,
    posterPath?: string | null,
    releaseDate?: string | null,
  }) => void
}

export function TmdbDetailsModal({
  open,
  onOpenChange,
  tmdbId,
  mediaType,
  imdbId,
  onSetReminder,
  onAddToWatchlist
}: TmdbDetailsModalProps) {
  const [loading, setLoading] = useState(true)
  const [movieDetails, setMovieDetails] = useState<TmdbMovieDetails | null>(null)
  const [tvDetails, setTvDetails] = useState<TmdbShowDetails | null>(null)
  const [selectedSeason, setSelectedSeason] = useState<number | null>(null)
  const [seasonDetails, setSeasonDetails] = useState<TmdbSeasonDetails | null>(null)
  const [loadingSeason, setLoadingSeason] = useState(false)
  const [imdbPanelOpen, setImdbPanelOpen] = useState(false)

  useEffect(() => {
    if (!open) {
      setMovieDetails(null)
      setTvDetails(null)
      setSelectedSeason(null)
      setSeasonDetails(null)
      return
    }

    const loadDetails = async () => {
      setLoading(true)
      try {
        if (mediaType === 'movie') {
          const data = await getMovieDetails(tmdbId, imdbId ?? null)
          setMovieDetails(data)
        } else {
          const data = await getTvDetails(tmdbId, imdbId ?? null)
          setTvDetails(data)
          if (data && data.seasons.length > 0) {
            const preferredSeasonNumber =
              data.next_episode_to_air?.season_number ??
              data.last_episode_to_air?.season_number ??
              data.seasons.reduce((max, season) => Math.max(max, season.season_number), 1)
            setSelectedSeason(preferredSeasonNumber)

            setLoadingSeason(true)
            try {
              const seasonData = await getTvSeasonEpisodes(
                tmdbId,
                preferredSeasonNumber,
                imdbId ?? null,
              )
              setSeasonDetails(seasonData)
            } catch (error) {
              console.error('Failed to load season details:', error)
            } finally {
              setLoadingSeason(false)
            }
          }
        }
      } catch (error) {
        console.error('Failed to load TMDB details:', error)
      } finally {
        setLoading(false)
      }
    }

    loadDetails()
  }, [open, tmdbId, mediaType])

  useEffect(() => {
    if (mediaType !== 'tv' || selectedSeason === null || !tmdbId || !open) return

    const loadSeason = async () => {
      setLoadingSeason(true)
      try {
        const data = await getTvSeasonEpisodes(tmdbId, selectedSeason)
        setSeasonDetails(data)
      } catch (error) {
        console.error('Failed to load season details:', error)
      } finally {
        setLoadingSeason(false)
      }
    }

    loadSeason()
  }, [selectedSeason, tmdbId, mediaType, open])

  const title = mediaType === 'movie' ? movieDetails?.title : tvDetails?.name
  const overview = mediaType === 'movie' ? movieDetails?.overview : tvDetails?.overview
  const firstReleaseDate = mediaType === 'movie' ? movieDetails?.release_date : tvDetails?.first_air_date
  const backdropPath = mediaType === 'movie' ? movieDetails?.backdrop_path : tvDetails?.backdrop_path
  const posterPath = mediaType === 'movie' ? movieDetails?.poster_path : tvDetails?.poster_path
  const latestSeason = tvDetails?.seasons.reduce((latest, season) => {
    return season.season_number > latest.season_number ? season : latest
  }, tvDetails.seasons[0])
  const selectedSeasonInfo = tvDetails?.seasons.find(season => season.season_number === selectedSeason)
  const seasonNextEpisode = seasonDetails?.episodes.find(episode => isFutureReleaseTarget(episode.air_date))
  const tmdbNextEpisode = tvDetails?.next_episode_to_air
  const nextEpisode = tmdbNextEpisode && isFutureReleaseTarget(tmdbNextEpisode.air_date)
    ? tmdbNextEpisode
    : seasonNextEpisode || tmdbNextEpisode
  const lastEpisode = tvDetails?.last_episode_to_air
  const airedEpisodes = lastEpisode
    ? tvDetails?.seasons.reduce((sum, season) => {
        if (season.season_number < (lastEpisode.season_number || 0)) return sum + season.episode_count
        if (season.season_number === lastEpisode.season_number) return sum + lastEpisode.episode_number
        return sum
      }, 0) || 0
    : seasonDetails?.episodes.filter(episode => {
        const target = parseReleaseTarget(episode.air_date)
        return !!target && target.getTime() <= Date.now()
      }).length || 0
  const selectedSeasonAired = lastEpisode?.season_number === selectedSeason
    ? lastEpisode.episode_number
    : selectedSeason != null && lastEpisode?.season_number != null && selectedSeason < lastEpisode.season_number
      ? selectedSeasonInfo?.episode_count || seasonDetails?.episodes.length || 0
      : seasonDetails?.episodes.filter(episode => {
          const target = parseReleaseTarget(episode.air_date)
          return !!target && target.getTime() <= Date.now()
        }).length || 0
  const selectedSeasonLeft = Math.max(0, (selectedSeasonInfo?.episode_count || seasonDetails?.episodes.length || 0) - selectedSeasonAired)
  const tvStatus = nextEpisode ? 'Airing' : tvDetails?.status || 'Available'
  const rating = mediaType === 'movie' ? movieDetails?.vote_average : tvDetails?.vote_average
  
  const reminderPayload = () => {
    if (mediaType === 'tv' && tvDetails && nextEpisode) {
      const hasFutureTarget = isFutureReleaseTarget(nextEpisode.air_date)
      return {
        tmdbId: String(tmdbId),
        mediaType: 'tv' as const,
        title: `${tvDetails.name} - ${nextEpisode.name}`,
        posterPath: nextEpisode.still_path || tvDetails.poster_path,
        seasonNumber: hasFutureTarget ? nextEpisode.season_number ?? selectedSeason : null,
        episodeNumber: hasFutureTarget ? nextEpisode.episode_number : null,
        releaseDate: hasFutureTarget ? nextEpisode.air_date : null,
        trackingMode: 'tv_season' as const,
        trackingSeasonNumber: nextEpisode.season_number ?? selectedSeason ?? null,
      }
    }

    return {
      tmdbId: String(tmdbId),
      mediaType,
      title: title || '',
      posterPath,
      releaseDate: mediaType === 'movie' ? movieDetails?.release_date : latestSeason?.air_date,
      trackingMode: mediaType === 'tv' ? 'tv_season' as const : 'single' as const,
      trackingSeasonNumber: mediaType === 'tv'
        ? (nextEpisode?.season_number ?? selectedSeason ?? latestSeason?.season_number ?? null)
        : null
    }
  }

  const watchlistPayload = () => ({
    tmdbId: String(tmdbId),
    mediaType,
    title: title || '',
    posterPath,
    releaseDate: mediaType === 'movie' ? movieDetails?.release_date : tvDetails?.first_air_date,
  })
  
  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange} modal={false}>
      <DialogContent className="max-w-6xl p-0 overflow-hidden bg-background border border-white/10 shadow-[0_0_120px_rgba(0,0,0,0.8)] rounded-[2.5rem]">
        <div className="relative h-full flex flex-col max-h-[80vh]">
          {/* Hero Section */}
          <div className="relative h-[300px] sm:h-[330px] shrink-0 overflow-hidden">
            {backdropPath ? (
              <motion.div 
                initial={{ scale: 1.1, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                transition={{ duration: 1.2, ease: "easeOut" }}
                className="absolute inset-0"
              >
                <img 
                  src={getTmdbImageUrl(backdropPath, 'original') || ''} 
                  alt={title}
                  className="w-full h-full object-cover"
                />
                <div className="absolute inset-0 bg-gradient-to-t from-background via-background/40 to-transparent" />
                <div className="absolute inset-y-0 left-0 w-1/2 bg-gradient-to-r from-background via-background/35 to-transparent" />
              </motion.div>
            ) : (
              <div className="absolute inset-0 bg-gradient-to-br from-card to-background" />
            )}
            


            <div className="absolute inset-x-0 bottom-0 p-6 sm:p-8 flex flex-col sm:flex-row items-end gap-6 z-10">
              {posterPath && (
                <motion.div 
                  initial={{ y: 40, opacity: 0 }}
                  animate={{ y: 0, opacity: 1 }}
                  transition={{ delay: 0.3, duration: 0.8 }}
                  className="hidden sm:block w-36 shrink-0 aspect-[2/3] rounded-2xl overflow-hidden shadow-[0_32px_64px_-16px_rgba(0,0,0,0.8)] border border-white/10"
                >
                  <img 
                    src={getTmdbImageUrl(posterPath, 'w500') || ''} 
                    alt={title}
                    className="w-full h-full object-cover"
                  />
                </motion.div>
              )}
              
              <div className="flex-1 space-y-4">
                <motion.div 
                  initial={{ x: -20, opacity: 0 }}
                  animate={{ x: 0, opacity: 1 }}
                  transition={{ delay: 0.4 }}
                  className="space-y-2"
                >
                  <div className="flex items-center gap-3">
                    <span className="px-3 py-1 rounded-lg bg-white/10 backdrop-blur-md border border-white/10 text-[10px] font-black uppercase tracking-[0.2em] text-white/80">
                      {mediaType === 'movie' ? 'Reminderstic Feature' : 'TV Series'}
                    </span>
                    {rating && (
                      <button
                        type="button"
                        onClick={() => setImdbPanelOpen(true)}
                        className="flex items-center gap-1.5 px-3 py-1 rounded-lg bg-amber-500/10 border border-amber-500/20 text-amber-400 font-black text-[10px] cursor-pointer hover:bg-amber-500/20 transition-colors"
                      >
                        <Star className="size-3 fill-current" />
                        {rating.toFixed(1)}
                      </button>
                    )}
                  </div>
                  <h2 className="text-3xl sm:text-4xl lg:text-5xl font-black text-white tracking-tighter drop-shadow-2xl">
                    {title || (loading ? 'Loading Content...' : 'Unknown')}
                  </h2>
                </motion.div>

                <motion.div 
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ delay: 0.6 }}
                  className="flex flex-wrap items-center gap-6 text-white/50 text-xs font-bold"
                >
                  {firstReleaseDate && (
                    <div className="flex items-center gap-2">
                      <Calendar className="size-4 opacity-40" />
                      <span>{new Date(firstReleaseDate).getFullYear()}</span>
                    </div>
                  )}
                  {mediaType === 'movie' && movieDetails?.runtime && (
                    <div className="flex items-center gap-2">
                      <Clock className="size-4 opacity-40" />
                      <span>{Math.floor(movieDetails.runtime / 60)}h {movieDetails.runtime % 60}m</span>
                    </div>
                  )}
                  {tvStatus && (
                    <div className="flex items-center gap-2 px-3 py-1 rounded-full border border-emerald-500/20 bg-emerald-500/5 text-emerald-400/80">
                      <div className="size-1.5 rounded-full bg-emerald-500 animate-pulse" />
                      <span className="uppercase tracking-widest text-[9px] font-black">{tvStatus}</span>
                    </div>
                  )}
                </motion.div>

                <motion.div 
                   initial={{ y: 20, opacity: 0 }}
                   animate={{ y: 0, opacity: 1 }}
                   transition={{ delay: 0.7 }}
                   className="flex flex-wrap items-center gap-4"
                >
                  <Button 
                    onClick={() => title && onAddToWatchlist(watchlistPayload())}
                    variant="outline"
                    className="border-white/15 bg-black/30 text-white hover:bg-white hover:text-black font-black uppercase tracking-widest text-[11px] rounded-[1.25rem] h-12 px-8"
                  >
                    <LayoutGrid className="size-4 mr-2" />
                    Add To Watchlist
                  </Button>

                  <Button 
                    onClick={() => title && onSetReminder(reminderPayload())}
                    className="bg-white text-black hover:bg-neutral-200 font-black uppercase tracking-widest text-[11px] rounded-[1.25rem] h-12 px-8 shadow-[0_20px_50px_rgba(255,255,255,0.1)] active:scale-95 transition-all"
                  >
                    <Bell className="size-4 mr-2" />
                    Add to Reminder
                  </Button>
                  
                  {mediaType === 'movie' && movieDetails?.release_date && isFutureReleaseTarget(movieDetails.release_date) && (
                    <CountdownTimer
                      target={movieDetails.release_date}
                      compact
                      label="Releases in"
                      className="bg-black/40 backdrop-blur-2xl border-white/10 h-12 rounded-[1.25rem] px-5"
                    />
                  )}
                  
                  {mediaType === 'tv' && nextEpisode?.air_date && (
                    <CountdownTimer
                      target={nextEpisode.air_date}
                      compact
                      label="Airs in"
                      forcePending
                      className="bg-black/40 backdrop-blur-2xl border-white/10 h-12 rounded-[1.25rem] px-5"
                    />
                  )}
                </motion.div>
              </div>
            </div>
          </div>

          {/* Content Area */}
          <ScrollArea className="flex-1 relative z-10">
            <div className="p-8 sm:p-12 grid grid-cols-1 lg:grid-cols-12 gap-12">
              <div className="lg:col-span-8 space-y-12">
                <section className="space-y-4">
                  <h3 className="text-sm font-black uppercase tracking-[0.3em] text-white/20">Storyline</h3>
                  <p className="text-white/70 leading-relaxed text-lg font-medium selection:bg-white selection:text-black">
                    {overview || 'No description available for this entry.'}
                  </p>
                </section>

                {mediaType === 'tv' && tvDetails && (
                  <section className="space-y-8">
                    <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
                      <StatTile label="Season" value={`S${latestSeason?.season_number || '-'}`} sub={latestSeason?.air_date ? new Date(latestSeason.air_date).getFullYear().toString() : 'TBA'} />
                      <StatTile label="Total Runtime" value={String(airedEpisodes)} sub="Episodes Aired" />
                      <StatTile label="Progress" value={`${selectedSeasonAired}/${selectedSeasonInfo?.episode_count || 0}`} sub={selectedSeasonLeft > 0 ? `${selectedSeasonLeft} remaining` : 'Finished'} />
                      <StatTile label="Network" value={tvDetails.networks?.[0]?.name || 'Unknown'} sub="Official Source" />
                    </div>

                    <div className="space-y-6">
                      <div className="flex items-center justify-between border-b border-white/5 pb-4">
                        <h3 className="text-sm font-black uppercase tracking-[0.3em] text-white/20">Seasons</h3>
                        <div className="flex gap-2 overflow-x-auto pb-2 max-w-full scrollbar-hide">
                          {tvDetails.seasons.map(season => (
                            <button
                              type="button"
                              key={season.season_number}
                              onClick={() => setSelectedSeason(season.season_number)}
                              className={cn(
                                "shrink-0 px-4 py-2 rounded-xl text-xs font-black uppercase tracking-widest transition-all border",
                                selectedSeason === season.season_number
                                  ? "bg-white text-black border-white shadow-lg"
                                  : "bg-white/10 text-white/40 border-white/5 hover:border-white/20 hover:text-white"
                              )}
                            >
                              S{season.season_number}
                            </button>
                          ))}
                        </div>
                      </div>

                      <div className="space-y-4">
                        <AnimatePresence mode="wait">
                          {loadingSeason ? (
                            <motion.div 
                              key="loading"
                              initial={{ opacity: 0 }}
                              animate={{ opacity: 1 }}
                              exit={{ opacity: 0 }}
                              className="flex items-center justify-center py-20"
                            >
                              <Loader2 className="size-8 animate-spin text-white/10" />
                            </motion.div>
                          ) : (
                            <motion.div 
                              key={selectedSeason}
                              initial={{ opacity: 0, y: 10 }}
                              animate={{ opacity: 1, y: 0 }}
                              className="grid gap-4"
                            >
                              {seasonDetails?.episodes.map((episode) => (
                                <EpisodeCard
                                  key={episode.episode_number}
                                  episode={episode}
                                  tvDetails={tvDetails}
                                  tmdbId={tmdbId}
                                  selectedSeason={selectedSeason}
                                  onSetReminder={onSetReminder}
                                />
                              ))}
                            </motion.div>
                          )}
                        </AnimatePresence>
                      </div>
                    </div>
                  </section>
                )}
              </div>

              <div className="lg:col-span-4 space-y-8">
                <section className="p-8 rounded-[2rem] bg-white/[0.06] border border-white/10 shadow-inner space-y-8">
                   <div className="space-y-4">
                    <h3 className="text-[10px] font-black uppercase tracking-[0.3em] text-white/20">Technical Details</h3>
                    
                    <div className="grid gap-6">
                      <DetailRow label={mediaType === 'tv' ? 'Series Creator' : 'Director'} value={mediaType === 'tv' ? tvDetails?.creator || 'Unknown' : movieDetails?.director || 'Unknown'} />
                      <DetailRow label="Production Status" value={tvDetails?.status || movieDetails?.status || 'Active'} />
                      <DetailRow label="First Release" value={firstReleaseDate ? new Date(firstReleaseDate).toLocaleDateString(undefined, { dateStyle: 'long' }) : 'TBA'} />
                      {mediaType === 'tv' && latestSeason && (
                        <DetailRow label="Volume" value={`Season ${latestSeason.season_number} · ${latestSeason.episode_count} Episodes`} />
                      )}
                    </div>
                  </div>

                  <div className="space-y-4 pt-4 border-t border-white/5">
                    <h3 className="text-[10px] font-black uppercase tracking-[0.3em] text-white/20">Genre Matrix</h3>
                    <div className="flex flex-wrap gap-2">
                      {(mediaType === 'movie' ? movieDetails?.genres : tvDetails?.genres)?.map((genre) => (
                        <span key={genre.id} className="px-3 py-1.5 rounded-xl bg-white/5 border border-white/10 text-[10px] font-bold text-white/60">
                          {genre.name}
                        </span>
                      ))}
                    </div>
                  </div>
                </section>

                <div className="p-8 rounded-[2rem] bg-white/[0.06] border border-white/10 shadow-inner">
                  <div className="flex items-center gap-3 mb-4">
                    <div className="size-8 rounded-xl bg-white/10 flex items-center justify-center">
                      <LayoutGrid className="size-4 text-white/60" />
                    </div>
                    <span className="text-[10px] font-black uppercase tracking-widest text-white/30">Quick Action</span>
                  </div>
                  <p className="text-[11px] text-white/30 font-medium leading-relaxed mb-6">
                    Set a dynamic reminder to receive real-time notifications when this content becomes available on your platform.
                  </p>
                  <Button 
                    className="w-full bg-white/10 hover:bg-white text-white hover:text-black font-black uppercase tracking-widest text-[10px] rounded-2xl h-12 transition-all"
                    onClick={() => onOpenChange(false)}
                  >
                    Close Inspection
                  </Button>
                </div>
              </div>
            </div>
          </ScrollArea>
        </div>
      </DialogContent>
    </Dialog>
    <ImdbDetailsPanel
      open={imdbPanelOpen}
      onOpenChange={setImdbPanelOpen}
      tmdbId={tmdbId}
      mediaType={mediaType}
    />
    </>
  )
}

function StatTile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="min-w-0 rounded-2xl border border-white/10 bg-white/[0.06] p-4 shadow-inner group hover:bg-white/[0.08] transition-colors">
      <div className="text-[9px] font-black uppercase tracking-[0.2em] text-white/20">{label}</div>
      <div className="mt-1 truncate text-xl font-black text-white tracking-tight">{value}</div>
      {sub && <div className="mt-1 truncate text-[10px] font-bold text-white/30 uppercase tracking-widest">{sub}</div>}
    </div>
  )
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[9px] font-black uppercase tracking-[0.2em] text-white/20">{label}</span>
      <span className="text-sm font-bold text-white/80">{value}</span>
    </div>
  )
}

function EpisodeCard({ episode, tvDetails, tmdbId, selectedSeason, onSetReminder }: {
  episode: TmdbSeasonDetails['episodes'][number]
  tvDetails: TmdbShowDetails
  tmdbId: number
  selectedSeason: number | null
  onSetReminder: TmdbDetailsModalProps['onSetReminder']
}) {
  return (
    <div
      className="group flex flex-col sm:flex-row gap-6 p-5 rounded-[2rem] bg-white/[0.05] border border-white/[0.05] hover:bg-white/[0.08] hover:border-white/10 transition-all duration-300"
    >
      <div className="shrink-0 w-full sm:w-48 aspect-video rounded-2xl overflow-hidden bg-neutral-950 border border-white/5 relative">
        {episode.still_path ? (
          <img
            src={getTmdbImageUrl(episode.still_path, 'w500') || ''}
            alt={episode.name}
            className="w-full h-full object-cover transition-transform duration-500 group-hover:scale-105"
          />
        ) : (
          <div className="w-full h-full flex items-center justify-center text-white/5">
            <PlayCircle className="size-10" />
          </div>
        )}
        <div className="absolute inset-0 bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity" />
      </div>

      <div className="flex-1 min-w-0 flex flex-col justify-between py-1">
        <div>
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h4 className="text-base font-black text-white tracking-tight truncate">
                {episode.episode_number}. {episode.name}
              </h4>
              <div className="flex items-center gap-3 mt-1.5">
                <div className="flex items-center gap-1.5 text-[10px] font-bold text-white/30 uppercase tracking-widest">
                  <Calendar className="size-3" />
                  {episode.air_date ? new Date(episode.air_date).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) : 'TBA'}
                </div>
              </div>
            </div>
            <Button
              variant="ghost"
              size="icon"
              className="size-10 rounded-2xl opacity-0 group-hover:opacity-100 transition-all bg-white/5 hover:bg-white text-white hover:text-black shadow-xl"
              onClick={(e) => {
                e.stopPropagation()
                onSetReminder({
                  tmdbId: String(tmdbId),
                  mediaType: 'tv',
                  title: `${tvDetails.name} - ${episode.name}`,
                  posterPath: episode.still_path || tvDetails.poster_path,
                  seasonNumber: selectedSeason,
                  episodeNumber: episode.episode_number,
                  releaseDate: episode.air_date,
                  trackingMode: 'single',
                  trackingSeasonNumber: selectedSeason,
                })
              }}
            >
              <Bell className="size-4" />
            </Button>
          </div>
          <p className="text-[13px] text-white/40 line-clamp-2 mt-3 leading-relaxed font-medium italic">
            {episode.overview || 'No plot details provided for this episode.'}
          </p>
        </div>

        {isFutureReleaseTarget(episode.air_date) && (
          <div className="mt-4 pt-4 border-t border-white/5">
            <CountdownTimer
              target={episode.air_date}
              compact
              className="bg-white/5 border-white/5"
            />
          </div>
        )}
      </div>
    </div>
  )
}
