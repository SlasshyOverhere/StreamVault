import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { Calendar, Clock, Play, Tv, Check, Loader2, Timer, ChevronDown, Star, User, AudioLines, Captions, SlidersHorizontal, X, RefreshCw, Download, Share2, FileText, Copy, EyeOff, Eye } from "lucide-react"
import {
  MediaItem, getCachedImageUrl, getMovieDetails, getTmdbImageUrl,
  searchTmdb, getEpisodes, getTvSeasonEpisodes, TmdbEpisodeInfo, TmdbMovieDetails, TmdbShowDetails, getTvDetails, getMediaInfo, refreshSeriesMetadata, updateEpisodeDuration,
  getSeriesAudioPreference, setSeriesAudioPreference, getSeriesSubtitlePreference, setSeriesSubtitlePreference, getAudioTracks, getSubtitleTracks,
  getCachedSeriesAudioTracks, setCachedSeriesAudioTracks,
  getCachedSeriesSubtitleTracks, setCachedSeriesSubtitleTracks,
  getMediaTechnicalDetails,
  ImdbEpisodeRating, getEpisodeImdbRatings,
  getSeriesSpoilerEnabled, setSeriesSpoilerEnabled,
  setPendingSubtitlePath,
  type AudioTrackOption, type SubtitleTrackOption, type MediaTechnicalDetails
} from "@/services/api"
import { Dialog, DialogContent, DialogDescription, DialogPortal, DialogTitle } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { isMediaMarkedWatched } from "@/utils/playbackProgress"
import { KebabMenu } from "@/components/KebabMenu"
import { useToast } from "@/components/ui/use-toast"
import { ScrollArea } from "@/components/ui/scroll-area"
import { ShareDialog } from "@/components/ShareDialog"
import { EpisodeThumbnailImage } from "@/components/EpisodeThumbnailImage"
import { getZipCompressionLabel } from "@/utils/zip"
import { ImdbDetailsPanel } from "@/components/ImdbDetailsPanel"
import { CastMemberPanel } from "@/components/CastMemberPanel"
import { SubtitleSelector } from "@/components/SubtitleSelector"

interface ContentDetailsModalProps {
  open: boolean
  item: MediaItem | null
  onOpenChange: (open: boolean) => void
  onPrimaryAction: (item: MediaItem) => void | Promise<void>
  onSecondaryAction?: (item: MediaItem) => void | Promise<void>
  onDownloadAction?: (item: MediaItem) => void | Promise<void>
  downloadActionLabel?: string
  secondaryActionLabel?: string
  onEpisodeSecondaryAction?: (item: MediaItem) => void | Promise<void>
  episodeSecondaryActionLabel?: string
  onEpisodeUnwatchAction?: (item: MediaItem) => void | Promise<void>
  onMetadataRefresh?: (itemId: number) => Promise<MediaItem | null> | MediaItem | null
}

const heroArtworkCache = new Map<number, string | null>()
const runtimeMinutesCache = new Map<number, number | null>()
const movieDetailsCache = new Map<number, TmdbMovieDetails | null>()
const tvDetailsCache = new Map<number, TmdbShowDetails | null>()
const technicalDetailsCache = new Map<number, MediaTechnicalDetails | null>()
const tvSeasonEpisodesCache = new Map<number, Map<number, Map<number, TmdbEpisodeInfo>>>()
const AUTO_AUDIO_VALUE = "__auto__"
const CUSTOM_AUDIO_VALUE = "__custom__"
const AUTO_SUBTITLE_VALUE = "__subtitle_auto__"
const OFF_SUBTITLE_VALUE = "__subtitle_off__"
const CUSTOM_SUBTITLE_VALUE = "__subtitle_custom__"
const MANUAL_AUDIO_OPTION = {
  label: "Manual",
  value: CUSTOM_AUDIO_VALUE,
} as const

const resolveAudioPreferenceValue = (
  selectedValue: string,
  customValue: string,
) => {
  if (selectedValue === AUTO_AUDIO_VALUE) {
    return null
  }

  if (selectedValue === CUSTOM_AUDIO_VALUE) {
    const normalized = customValue.trim()
    return normalized.length > 0 ? normalized : null
  }

  return selectedValue
}

const resolveSubtitlePreferenceValue = (
  selectedValue: string,
  customValue: string,
) => {
  if (selectedValue === AUTO_SUBTITLE_VALUE) {
    return null
  }

  if (selectedValue === OFF_SUBTITLE_VALUE) {
    return "sid:no"
  }

  if (selectedValue === CUSTOM_SUBTITLE_VALUE) {
    const normalized = customValue.trim()
    return normalized.length > 0 ? normalized : null
  }

  return selectedValue
}

const resolveLocalImage = async (path?: string): Promise<string | null> => {
  if (!path || typeof path !== "string") return null
  if (path.startsWith("http") || path.startsWith("asset://")) return path
  const filename = path.replace("image_cache/", "")
  return getCachedImageUrl(filename)
}

const formatEpisodeSize = (bytes?: number | null): string | null => {
  if (bytes == null || !Number.isFinite(bytes) || bytes <= 0) return null

  const units = ["B", "KB", "MB", "GB", "TB"]
  let value = bytes
  let unitIndex = 0

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }

  const decimals = value >= 100 ? 0 : value >= 10 ? 1 : 2
  return `${value.toFixed(decimals)} ${units[unitIndex]}`
}

const formatFps = (fps?: number | null): string | null => {
  if (fps == null || !Number.isFinite(fps) || fps <= 0) return null
  const rounded = fps >= 100 ? fps.toFixed(0) : fps >= 10 ? fps.toFixed(3) : fps.toFixed(2)
  return `${rounded.replace(/\.?0+$/, "")} fps`
}

const parseMediaHints = (value?: string | null): Partial<MediaTechnicalDetails> => {
  if (!value) return {}

  const text = value.toLowerCase()
  const resolutionMatch = text.match(/(?:^|[^0-9])(2160p|1440p|1080p|720p|480p)(?:[^0-9]|$)/i)
  const fpsMatch = text.match(/(\d{2,3}(?:\.\d+)?)\s*fps/i)
  const extensionMatch = value.match(/\.([a-z0-9]{2,5})(?:$|\s)/i)

  return {
    resolutionLabel: resolutionMatch?.[1]?.toLowerCase() ?? undefined,
    fps: fpsMatch ? Number.parseFloat(fpsMatch[1]) : undefined,
    extension: extensionMatch?.[1]?.toUpperCase() ?? undefined,
    container: extensionMatch?.[1]?.toUpperCase() ?? undefined,
  }
}

const getExtensionLabel = (path?: string | null): string | null => {
  if (!path) return null
  const lastDot = path.lastIndexOf(".")
  if (lastDot < 0 || lastDot === path.length - 1) return null
  return path.slice(lastDot + 1).toUpperCase()
}

const buildDisplayMediaParts = (
  item: Pick<MediaItem, "title" | "file_path" | "zip_entry_path" | "file_size_bytes" | "zip_uncompressed_size" | "zip_compressed_size" | "parent_zip_id">,
  details?: MediaTechnicalDetails | null,
  includeSize: boolean = true,
): string[] => {
  const hinted = parseMediaHints(item.title || item.file_path || item.zip_entry_path)
  const size = details?.fileSizeBytes ?? (
    item.parent_zip_id
      ? item.zip_uncompressed_size ?? item.zip_compressed_size ?? item.file_size_bytes ?? null
      : item.file_size_bytes ?? item.zip_uncompressed_size ?? item.zip_compressed_size ?? null
  )

  return [
    details?.resolutionLabel ?? hinted.resolutionLabel ?? null,
    formatFps(details?.fps ?? hinted.fps),
    details?.container ?? hinted.container ?? getExtensionLabel(item.zip_entry_path || item.file_path),
    includeSize ? formatEpisodeSize(size) : null,
  ].filter(Boolean) as string[]
}

const getPreferredEpisodeSize = (episode: MediaItem): number | null => {
  if (episode.parent_zip_id) {
    return episode.zip_uncompressed_size ?? episode.zip_compressed_size ?? episode.file_size_bytes ?? null
  }

  return episode.file_size_bytes ?? episode.zip_uncompressed_size ?? episode.zip_compressed_size ?? null
}

export function ContentDetailsModal({
  open,
  item,
  onOpenChange,
  onPrimaryAction,
  onSecondaryAction,
  onDownloadAction,
  downloadActionLabel,
  secondaryActionLabel,
  onEpisodeSecondaryAction,
  episodeSecondaryActionLabel,
  onEpisodeUnwatchAction,
  onMetadataRefresh,
}: ContentDetailsModalProps) {
  const { toast } = useToast()
  const [heroImageUrl, setHeroImageUrl] = useState<string | null>(null)
  const [posterImageUrl, setPosterImageUrl] = useState<string | null>(null)
  const [runtimeMinutesOverride, setRuntimeMinutesOverride] = useState<number | null>(null)
  const [director, setDirector] = useState<string | null>(null)
  const [creator, setCreator] = useState<string | null>(null)
  const [movieVoteAverage, setMovieVoteAverage] = useState<number | null>(null)
  const [technicalDetails, setTechnicalDetails] = useState<MediaTechnicalDetails | null>(null)

  const [episodes, setEpisodes] = useState<MediaItem[]>([])
  const [loadingEpisodes, setLoadingEpisodes] = useState(false)
  const [selectedSeason, setSelectedSeason] = useState<number>(1)
  const [tmdbEpisodesBySeason, setTmdbEpisodesBySeason] = useState<Map<number, Map<number, TmdbEpisodeInfo>>>(new Map())
  const [imdbEpisodeRatings, setImdbEpisodeRatings] = useState<Record<number, ImdbEpisodeRating>>({})
  const [selectedAudioPreference, setSelectedAudioPreference] = useState<string>(AUTO_AUDIO_VALUE)
  const [customAudioPreference, setCustomAudioPreference] = useState("")
  const [detectedAudioTracks, setDetectedAudioTracks] = useState<AudioTrackOption[]>([])
  const [audioTracksLoading, setAudioTracksLoading] = useState(false)
  const [audioTracksStatus, setAudioTracksStatus] = useState<string>("")
  const [detectedSubtitleTracks, setDetectedSubtitleTracks] = useState<SubtitleTrackOption[]>([])
  const [subtitleTracksLoading, setSubtitleTracksLoading] = useState(false)
  const [subtitleTracksStatus, setSubtitleTracksStatus] = useState<string>("")
  const [selectedSubtitlePreference, setSelectedSubtitlePreference] = useState<string>(AUTO_SUBTITLE_VALUE)
  const [customSubtitlePreference, setCustomSubtitlePreference] = useState("")
  const [playbackSettingsOpen, setPlaybackSettingsOpen] = useState(false)
  const [subtitlePanelOpen, setSubtitlePanelOpen] = useState(false)
  const [selectedSubtitlePath, setSelectedSubtitlePath] = useState<string | null>(null)
  const [shareFileId, setShareFileId] = useState<string | null>(null)
  const [shareFileName, setShareFileName] = useState<string>("")
  const [isRefreshingMetadata, setIsRefreshingMetadata] = useState(false)
  const [imdbPanelImdbId, setImdbPanelImdbId] = useState<string | null>(null)
  const [castPanelName, setCastPanelName] = useState<string | null>(null)
  const [refreshCounter, setRefreshCounter] = useState(0)

  const [activeItem, setActiveItem] = useState<MediaItem | null>(null)
  const lastItemIdRef = useRef<number | null>(null)
  const refreshGenerationRef = useRef(0)
  const [showEpisodeUrls, setShowEpisodeUrls] = useState(false)
  const [spoilerEnabled, setSpoilerEnabled] = useState(true)
  const [revealedEpisodes, setRevealedEpisodes] = useState<Set<number>>(new Set())

  const handleEpisodeMarkWatched = async (episode: MediaItem) => {
    if (!onEpisodeSecondaryAction) return

    // Optimistic update: update UI immediately
    const markedAt = new Date().toISOString()
    setEpisodes((currentEpisodes) =>
      currentEpisodes.map((currentEpisode) =>
        currentEpisode.id === episode.id
          ? {
              ...currentEpisode,
              progress_percent: 100,
              resume_position_seconds: 0,
              duration_seconds: currentEpisode.duration_seconds ?? episode.duration_seconds,
              last_watched: markedAt,
            }
          : currentEpisode,
      ),
    )

    setActiveItem((currentActiveItem) =>
      currentActiveItem?.id === episode.id
        ? {
            ...currentActiveItem,
            progress_percent: 100,
            resume_position_seconds: 0,
            duration_seconds: currentActiveItem.duration_seconds ?? episode.duration_seconds,
            last_watched: markedAt,
          }
        : currentActiveItem,
    )

    // Fire server call in background
    void onEpisodeSecondaryAction(episode)
  }

  const handleEpisodeUnwatched = (episode: MediaItem) => {
    // Optimistic update: clear watch state immediately
    setEpisodes((currentEpisodes) =>
      currentEpisodes.map((currentEpisode) =>
        currentEpisode.id === episode.id
          ? {
              ...currentEpisode,
              progress_percent: 0,
              resume_position_seconds: 0,
              last_watched: undefined,
            }
          : currentEpisode,
      ),
    )

    setActiveItem((currentActiveItem) =>
      currentActiveItem?.id === episode.id
        ? {
            ...currentActiveItem,
            progress_percent: 0,
            resume_position_seconds: 0,
            last_watched: undefined,
          }
        : currentActiveItem,
    )

    // Fire server call in background
    void onEpisodeUnwatchAction?.(episode)
  }

  if (!item && lastItemIdRef.current !== null) {
    lastItemIdRef.current = null
    refreshGenerationRef.current += 1
    setActiveItem(null)
  } else if (item && item.id !== lastItemIdRef.current) {
    lastItemIdRef.current = item.id
    refreshGenerationRef.current += 1
    setActiveItem(item)
  }

  const handleRefreshMetadata = useCallback(async () => {
    if (!item || item.media_type !== "tvshow" || !item.tmdb_id || isRefreshingMetadata) return

    const tmdbId = Number.parseInt(item.tmdb_id, 10)
    if (!Number.isFinite(tmdbId) || tmdbId <= 0) {
      toast({
        title: "Refresh unavailable",
        description: "This series does not have a valid TMDB match yet.",
        variant: "destructive",
      })
      return
    }

    const generation = refreshGenerationRef.current
    const itemId = item.id

    setIsRefreshingMetadata(true)
    heroArtworkCache.delete(itemId)
    tvDetailsCache.delete(tmdbId)
    tvSeasonEpisodesCache.delete(tmdbId)

    try {
      const result = await refreshSeriesMetadata(tmdbId, item.title)
      if (refreshGenerationRef.current !== generation) return

      const refreshedItem =
        (await Promise.resolve(onMetadataRefresh?.(itemId))) ||
        (await getMediaInfo(itemId))
      if (refreshGenerationRef.current !== generation) return

      const refreshedEpisodes = await getEpisodes(itemId)
      if (refreshGenerationRef.current !== generation) return

      setActiveItem(refreshedItem)
      setEpisodes(refreshedEpisodes)
      setTmdbEpisodesBySeason(new Map())
      setImdbEpisodeRatings({})
      setRefreshCounter(c => c + 1)

      toast({
        title: "Metadata refreshed",
        description: result,
      })
    } catch (error) {
      if (refreshGenerationRef.current !== generation) return
      console.error("Failed to refresh metadata in content details modal:", error)
      toast({
        title: "Error",
        description: "Failed to refresh metadata",
        variant: "destructive",
      })
    } finally {
      if (refreshGenerationRef.current === generation) {
        setIsRefreshingMetadata(false)
      }
    }
  }, [isRefreshingMetadata, item, onMetadataRefresh, toast])

  const seriesPreferenceId = useMemo(() => {
    if (!item) return null
    if (item.media_type === "tvshow") return item.id
    if (item.media_type === "tvepisode") return item.parent_id ?? null
    return null
  }, [item])

  const lastSpoilerSeriesRef = useRef<number | null>(null)
  if (seriesPreferenceId !== lastSpoilerSeriesRef.current) {
    lastSpoilerSeriesRef.current = seriesPreferenceId
    if (seriesPreferenceId) {
      setSpoilerEnabled(getSeriesSpoilerEnabled(seriesPreferenceId))
      setRevealedEpisodes(new Set())
    }
  }

  const toggleSpoiler = useCallback(() => {
    if (!seriesPreferenceId) return
    setSpoilerEnabled(prev => {
      const next = !prev
      setSeriesSpoilerEnabled(seriesPreferenceId, next)
      return next
    })
  }, [seriesPreferenceId])

  const handleToggleSpoiler = useCallback((ep: MediaItem) => {
    setRevealedEpisodes(prev => {
      const next = new Set(prev)
      if (next.has(ep.id)) {
        next.delete(ep.id)
      } else {
        next.add(ep.id)
      }
      return next
    })
  }, [])

  useEffect(() => {
    if (!seriesPreferenceId) {
      setSelectedAudioPreference(AUTO_AUDIO_VALUE)
      setCustomAudioPreference("")
      return
    }

    const storedPreference = getSeriesAudioPreference(seriesPreferenceId)
    const normalizedStoredAudio = storedPreference?.trim().toLowerCase()

    let audioPresetMatch = undefined
    if (normalizedStoredAudio) {
      const preferenceParts = new Set(
        normalizedStoredAudio
          .split(",")
          .map((part) => part.trim())
          .filter(Boolean)
      )

      audioPresetMatch = detectedAudioTracks.find((option) => {
        return (
          option.mpv_value?.trim().toLowerCase() === normalizedStoredAudio ||
          option.language_code?.trim().toLowerCase() === normalizedStoredAudio ||
          preferenceParts.has(option.language_code?.trim().toLowerCase() || "")
        )
      })
    }

    if (audioPresetMatch) {
      setSelectedAudioPreference(audioPresetMatch.mpv_value || AUTO_AUDIO_VALUE)
      setCustomAudioPreference("")
      return
    }

    if (storedPreference) {
      setSelectedAudioPreference(CUSTOM_AUDIO_VALUE)
      setCustomAudioPreference(storedPreference)
      return
    }

    setSelectedAudioPreference(AUTO_AUDIO_VALUE)
    setCustomAudioPreference("")
  }, [detectedAudioTracks, seriesPreferenceId])

  useEffect(() => {
    if (!seriesPreferenceId) {
      setSelectedSubtitlePreference(AUTO_SUBTITLE_VALUE)
      setCustomSubtitlePreference("")
      return
    }

    const storedPreference = getSeriesSubtitlePreference(seriesPreferenceId)
    const normalizedStored = storedPreference?.trim().toLowerCase()

    if (normalizedStored === "sid:no" || normalizedStored === "no") {
      setSelectedSubtitlePreference(OFF_SUBTITLE_VALUE)
      setCustomSubtitlePreference("")
      return
    }

    let subtitlePresetMatch = undefined
    if (normalizedStored) {
      const preferenceParts = new Set(
        normalizedStored
          .split(",")
          .map((part) => part.trim())
          .filter(Boolean)
      )

      subtitlePresetMatch = detectedSubtitleTracks.find((option) => {
        return (
          option.mpv_value?.trim().toLowerCase() === normalizedStored ||
          option.language_code?.trim().toLowerCase() === normalizedStored ||
          preferenceParts.has(option.language_code?.trim().toLowerCase() || "")
        )
      })
    }

    if (subtitlePresetMatch) {
      setSelectedSubtitlePreference(subtitlePresetMatch.mpv_value || AUTO_SUBTITLE_VALUE)
      setCustomSubtitlePreference("")
      return
    }

    if (storedPreference) {
      setSelectedSubtitlePreference(CUSTOM_SUBTITLE_VALUE)
      setCustomSubtitlePreference(storedPreference)
      return
    }

    setSelectedSubtitlePreference(AUTO_SUBTITLE_VALUE)
    setCustomSubtitlePreference("")
  }, [detectedSubtitleTracks, seriesPreferenceId])

  // Reset and load episodes
  useEffect(() => {
    if (!open) {
      setEpisodes([])
      setLoadingEpisodes(false)
      setTmdbEpisodesBySeason(new Map())
      setPlaybackSettingsOpen(false)
      setSubtitlePanelOpen(false)
      setSelectedSubtitlePath(null)
      return
    }

    if (!item || item.media_type !== "tvshow") {
      setEpisodes([])
      setLoadingEpisodes(false)
      return
    }

    const loadEpisodes = async () => {
      setLoadingEpisodes(true)
      try {
        const data = await getEpisodes(item.id)
        setEpisodes(data)
        if (data.length > 0) {
          const firstSeason = data.reduce((min, ep) =>
            ep.season_number && ep.season_number < min ? ep.season_number : min,
            data[0].season_number || 1
          )
          setSelectedSeason(firstSeason)
        }
      } catch (error) {
        console.error("Failed to load episodes in modal:", error)
      } finally {
        setLoadingEpisodes(false)
      }
    }

    void loadEpisodes()
  }, [open, item?.id])

  // Fetch TMDB episode metadata
  useEffect(() => {
    if (!open || !item || !item.tmdb_id || item.media_type !== "tvshow") return

    const tmdbId = parseInt(item.tmdb_id!)
    const cachedShowSeasons = tvSeasonEpisodesCache.get(tmdbId)
    const cachedSeason = cachedShowSeasons?.get(selectedSeason)
    if (cachedSeason) {
      setTmdbEpisodesBySeason(prev => {
        if (prev.get(selectedSeason) === cachedSeason) return prev
        const next = new Map(prev)
        next.set(selectedSeason, cachedSeason)
        return next
      })
      return
    }

    const loadTmdbMetadata = async () => {
      try {
        const data = await getTvSeasonEpisodes(tmdbId, selectedSeason)
        if (data) {
          const episodeMap = new Map<number, TmdbEpisodeInfo>()
          data.episodes.forEach(ep => {
            episodeMap.set(ep.episode_number, ep)
          })
          const nextShowSeasons = new Map(tvSeasonEpisodesCache.get(tmdbId) ?? new Map())
          nextShowSeasons.set(selectedSeason, episodeMap)
          tvSeasonEpisodesCache.set(tmdbId, nextShowSeasons)
          setTmdbEpisodesBySeason(prev => {
            const next = new Map(prev)
            next.set(selectedSeason, episodeMap)
            return next
          })

          // Write TMDB runtime back to DB for episodes missing duration
          for (const tmdbEp of data.episodes) {
            if (!tmdbEp.runtime || tmdbEp.runtime <= 0) continue
            const localEp = episodes.find(
              e => (e.season_number || 1) === selectedSeason && e.episode_number === tmdbEp.episode_number
            )
            if (localEp && (!localEp.duration_seconds || localEp.duration_seconds <= 0)) {
              updateEpisodeDuration(localEp.id, tmdbEp.runtime * 60)
            }
          }

          // Fetch IMDb ratings for these episodes
          const epNums = data.episodes
            .map(e => e.episode_number)
            .filter(n => n > 0)
          if (epNums.length > 0) {
            const ratings = await getEpisodeImdbRatings(tmdbId, selectedSeason, epNums, item?.imdb_id)
            if (Object.keys(ratings).length > 0) {
              setImdbEpisodeRatings(p => ({ ...p, ...ratings }))
            }
          }
        }
      } catch (error) {
        console.error("Failed to load TMDB episode metadata:", error)
      }
    }

    void loadTmdbMetadata()
  }, [open, item?.id, selectedSeason, refreshCounter])

  // Instant artwork reset and load (useLayoutEffect to prevent flash of stale image on reopen)
  useLayoutEffect(() => {
    const target = activeItem ?? item
    if (!target) {
      setHeroImageUrl(null)
      setPosterImageUrl(null)
      return
    }

    // Reset immediately
    setHeroImageUrl(null)
    setPosterImageUrl(null)
    setRuntimeMinutesOverride(null)
    setDirector(null)
    setCreator(null)
    setMovieVoteAverage(null)
    setTechnicalDetails(null)

    const cachedHero = heroArtworkCache.get(target.id)
    if (cachedHero !== undefined) {
      setHeroImageUrl(cachedHero)
    }

    const cachedRuntime = runtimeMinutesCache.get(target.id)
    if (cachedRuntime !== undefined) {
      setRuntimeMinutesOverride(cachedRuntime)
    }

    let cancelled = false

    const loadArtworkAndDetails = async () => {
      if (!open || !target) return

      const expectedType = target.media_type === "movie" ? "movie" : "tv"
      const itemTmdbId = Number.parseInt(target.tmdb_id || "", 10)
      const hasItemTmdbId = Number.isFinite(itemTmdbId) && itemTmdbId > 0
      let nextHero = cachedHero ?? null

      const posterPromise = resolveLocalImage(target.poster_path)
      const detailsPromise = hasItemTmdbId
        ? target.media_type === "movie"
          ? (movieDetailsCache.has(itemTmdbId)
              ? Promise.resolve(movieDetailsCache.get(itemTmdbId) ?? null)
              : getMovieDetails(itemTmdbId).then((details) => {
                  movieDetailsCache.set(itemTmdbId, details)
                  return details
                }))
          : (tvDetailsCache.has(itemTmdbId)
              ? Promise.resolve(tvDetailsCache.get(itemTmdbId) ?? null)
              : getTvDetails(itemTmdbId).then((details) => {
                  tvDetailsCache.set(itemTmdbId, details)
                  return details
                }))
        : Promise.resolve(null)

      const [poster, details] = await Promise.all([posterPromise, detailsPromise])
      if (!cancelled) setPosterImageUrl(poster)

      if (!cancelled && details) {
        if (target.media_type === "movie") {
          const movieDetails = details as TmdbMovieDetails
          if (movieDetails.runtime) {
            runtimeMinutesCache.set(target.id, movieDetails.runtime)
            setRuntimeMinutesOverride(movieDetails.runtime)
          }
          if (movieDetails.director) {
            setDirector(movieDetails.director)
          }
          if (movieDetails.vote_average != null) {
            setMovieVoteAverage(movieDetails.vote_average)
          }
          if (!nextHero && movieDetails.backdrop_path) {
            nextHero = await resolveLocalImage(movieDetails.backdrop_path)
          }
        } else if (target.media_type === "tvshow") {
          const showDetails = details as TmdbShowDetails
          if (showDetails.creator) {
            setCreator(showDetails.creator)
          }
          if (!nextHero && showDetails.backdrop_path) {
            nextHero = await resolveLocalImage(showDetails.backdrop_path)
          }
        }
      }

      if (!nextHero) {
        try {
          if (target.media_type === "tvepisode" && target.still_path) {
            nextHero = await resolveLocalImage(target.still_path)
          }
          if (!nextHero && !hasItemTmdbId) {
            const response = await searchTmdb(target.title)
            const results = Array.isArray(response?.results) ? response.results : []
            const exactMatch = results.find(r => String(r.id) === target.tmdb_id && r.media_type === expectedType)
            const chosen = exactMatch ?? results.find(r => r.media_type === expectedType && !!r.backdrop_path)
            nextHero = getTmdbImageUrl(chosen?.backdrop_path, "original")
          }
        } catch (error) {
          console.warn("[ContentDetails] Failed to fetch hero image fallback:", error);
        }
      }

      if (!nextHero) nextHero = poster
      heroArtworkCache.set(target.id, nextHero)
      if (!cancelled) setHeroImageUrl(nextHero)
    }

    void loadArtworkAndDetails()
    return () => { cancelled = true }
  }, [activeItem, item, open])

  const castList = useMemo(() => {
    const target = activeItem ?? item
    if (!target?.cast_names || typeof target.cast_names !== "string") return []
    return target.cast_names.split(",").map(s => s.trim()).filter(Boolean).slice(0, 8)
  }, [activeItem, item])

  // Memoize seasons calculation to prevent redundant set creation and sorting on every render
  const seasons = useMemo(() => {
    return Array.from(new Set(episodes.map(ep => ep.season_number || 1))).sort((a, b) => a - b)
  }, [episodes])

  // Memoize episodes filtering and sorting to prevent redundant array operations on every render
  const filteredEpisodes = useMemo(() => {
    return episodes.filter(ep => (ep.season_number || 1) === selectedSeason).sort((a, b) => (a.episode_number || 0) - (b.episode_number || 0))
  }, [episodes, selectedSeason])

  const selectedSeasonHasZipEpisodes = useMemo(() => (
    filteredEpisodes.some((episode) => !!episode.parent_zip_id)
  ), [filteredEpisodes])

  useEffect(() => {
    const targetItem = activeItem ?? item

    if (!open || !targetItem) {
      setTechnicalDetails(null)
      return
    }

    const technicalMediaId =
      targetItem.media_type === "tvshow"
        ? filteredEpisodes[0]?.id ?? null
        : targetItem.id

    if (!technicalMediaId) {
      setTechnicalDetails(null)
      return
    }

    const probeTarget =
      targetItem.media_type === "tvshow"
        ? filteredEpisodes[0] ?? null
        : targetItem

    const hinted = parseMediaHints(
      probeTarget?.title || probeTarget?.file_path || probeTarget?.zip_entry_path,
    )
    if (Object.keys(hinted).length > 0) {
      setTechnicalDetails((current) => ({
        ...(current ?? {}),
        ...hinted,
        sampleFromEpisode: targetItem.media_type === "tvshow" ? true : current?.sampleFromEpisode,
      }))
    }

    const cached = technicalDetailsCache.get(technicalMediaId)
    if (cached !== undefined) {
      setTechnicalDetails(
        targetItem.media_type === "tvshow" && cached
          ? { ...cached, sampleFromEpisode: true }
          : cached,
      )
      return
    }

    let cancelled = false

    const loadTechnicalDetails = async () => {
      const details = await getMediaTechnicalDetails(technicalMediaId)
      technicalDetailsCache.set(technicalMediaId, details)

      if (!cancelled) {
        setTechnicalDetails(
          targetItem.media_type === "tvshow" && details
            ? { ...details, sampleFromEpisode: true }
            : details,
        )
      }
    }

    void loadTechnicalDetails()

    return () => {
      cancelled = true
    }
  }, [activeItem, filteredEpisodes, item, open])

  useEffect(() => {
    if (!open || !item || item.media_type !== "tvshow") {
      setDetectedAudioTracks([])
      setAudioTracksLoading(false)
      setAudioTracksStatus("")
      setDetectedSubtitleTracks([])
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus("")
      return
    }

    if (filteredEpisodes.length === 0) {
      setDetectedAudioTracks([])
      setAudioTracksLoading(false)
      setAudioTracksStatus("")
      setDetectedSubtitleTracks([])
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus("")
      return
    }

    const cachedTracks = getCachedSeriesAudioTracks(item.id)
    if (cachedTracks) {
      setDetectedAudioTracks(cachedTracks)
      setAudioTracksLoading(false)
      setAudioTracksStatus(
        selectedSeasonHasZipEpisodes
          ? "Learned from playback and updates as you watch."
          : "Detected earlier for this series.",
      )
      return
    }

    if (selectedSeasonHasZipEpisodes) {
      setDetectedAudioTracks([])
      setAudioTracksLoading(false)
      setAudioTracksStatus("Play one ZIP episode once. Later episodes can expand this list.")
      return
    }

    let cancelled = false

    const loadAudioTracks = async () => {
      setAudioTracksLoading(true)
      setAudioTracksStatus("Detecting audio tracks...")

      const sampleEpisode = filteredEpisodes[0]
      const tracks = await getAudioTracks(sampleEpisode.id)
      if (cancelled) return

      const nextTracks = tracks.toSorted((left, right) =>
        left.label.localeCompare(right.label),
      )

      setDetectedAudioTracks(nextTracks)
      setCachedSeriesAudioTracks(item.id, nextTracks)
      setAudioTracksStatus(
        nextTracks.length > 0
          ? `Detected once from episode ${sampleEpisode.episode_number || 1}.`
          : "No labeled audio tracks were found in the sampled episode.",
      )
      setAudioTracksLoading(false)
    }

    void loadAudioTracks()

    return () => {
      cancelled = true
    }
  }, [filteredEpisodes, item, open, selectedSeason, selectedSeasonHasZipEpisodes])

  useEffect(() => {
    if (!open || !item || item.media_type !== "tvshow") {
      setDetectedSubtitleTracks([])
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus("")
      return
    }

    if (filteredEpisodes.length === 0) {
      setDetectedSubtitleTracks([])
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus("")
      return
    }

    const cachedTracks = getCachedSeriesSubtitleTracks(item.id)
    if (cachedTracks) {
      setDetectedSubtitleTracks(cachedTracks)
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus(
        selectedSeasonHasZipEpisodes
          ? "Learned from playback and updates as you watch."
          : "Detected earlier for this series.",
      )
      return
    }

    if (selectedSeasonHasZipEpisodes) {
      setDetectedSubtitleTracks([])
      setSubtitleTracksLoading(false)
      setSubtitleTracksStatus("Play one ZIP episode once. Later episodes can expand this list.")
      return
    }

    let cancelled = false

    const loadSubtitleTracks = async () => {
      setSubtitleTracksLoading(true)
      setSubtitleTracksStatus("Detecting subtitle tracks...")

      const sampleEpisode = filteredEpisodes[0]
      const tracks = await getSubtitleTracks(sampleEpisode.id)
      if (cancelled) return

      const nextTracks = tracks.toSorted((left, right) =>
        left.label.localeCompare(right.label),
      )

      setDetectedSubtitleTracks(nextTracks)
      setCachedSeriesSubtitleTracks(item.id, nextTracks)
      setSubtitleTracksStatus(
        nextTracks.length > 0
          ? `Detected once from episode ${sampleEpisode.episode_number || 1}.`
          : "No labeled subtitle tracks were found in the sampled episode.",
      )
      setSubtitleTracksLoading(false)
    }

    void loadSubtitleTracks()

    return () => {
      cancelled = true
    }
  }, [filteredEpisodes, item, open, selectedSeason, selectedSeasonHasZipEpisodes])

  if (!activeItem && !item) return null
  const displayItem = activeItem ?? item
  if (!displayItem) return null

  const isShow = displayItem.media_type === "tvshow"
  const isEpisode = displayItem.media_type === "tvepisode"
  const runtimeMinutes = (displayItem.duration_seconds && displayItem.duration_seconds > 0
    ? Math.max(1, Math.round(displayItem.duration_seconds / 60))
    : null) ?? runtimeMinutesOverride

  const runtimeLabel = runtimeMinutes
    ? (runtimeMinutes >= 60 ? `${Math.floor(runtimeMinutes / 60)}h ${runtimeMinutes % 60}m` : `${runtimeMinutes}m`)
    : "N/A"
  const zipCompressionLabel = displayItem.parent_zip_id
    ? getZipCompressionLabel(displayItem.zip_compression_method)
    : null
  const showSampleMediaItem = isShow
    ? filteredEpisodes[0] ?? episodes[0] ?? null
    : null
  const mediaFormatParts = buildDisplayMediaParts(
    showSampleMediaItem ?? displayItem,
    technicalDetails,
  )

  const displayTitle = isEpisode && displayItem.season_number && displayItem.episode_number
    ? `S${String(displayItem.season_number).padStart(2, "0")}E${String(displayItem.episode_number).padStart(2, "0")} · ${displayItem.title}`
    : displayItem.title

  const handleAudioPreferenceSelect = (value: string) => {
    setSelectedAudioPreference(value)
    if (!seriesPreferenceId) return

    setSeriesAudioPreference(
      seriesPreferenceId,
      resolveAudioPreferenceValue(
        value,
        value === CUSTOM_AUDIO_VALUE ? customAudioPreference : "",
      ),
    )
  }

  const handleCustomAudioPreferenceChange = (value: string) => {
    setCustomAudioPreference(value)
    if (!seriesPreferenceId || selectedAudioPreference !== CUSTOM_AUDIO_VALUE) return

    setSeriesAudioPreference(
      seriesPreferenceId,
      resolveAudioPreferenceValue(CUSTOM_AUDIO_VALUE, value),
    )
  }

  const handleSubtitlePreferenceSelect = (value: string) => {
    setSelectedSubtitlePreference(value)
    if (!seriesPreferenceId) return

    setSeriesSubtitlePreference(
      seriesPreferenceId,
      resolveSubtitlePreferenceValue(
        value,
        value === CUSTOM_SUBTITLE_VALUE ? customSubtitlePreference : "",
      ),
    )
  }

  const handleCustomSubtitlePreferenceChange = (value: string) => {
    setCustomSubtitlePreference(value)
    if (!seriesPreferenceId || selectedSubtitlePreference !== CUSTOM_SUBTITLE_VALUE) return

    setSeriesSubtitlePreference(
      seriesPreferenceId,
      resolveSubtitlePreferenceValue(CUSTOM_SUBTITLE_VALUE, value),
    )
  }

  const selectedDetectedAudioTrack = detectedAudioTracks.find(
    (track) => track.mpv_value === selectedAudioPreference,
  )
  const selectedAudioSummary = selectedAudioPreference === CUSTOM_AUDIO_VALUE
    ? (customAudioPreference.trim() || "Manual")
    : selectedDetectedAudioTrack?.label || (selectedAudioPreference === AUTO_AUDIO_VALUE ? "Auto" : null)
  const selectedDetectedSubtitleTrack = detectedSubtitleTracks.find(
    (track) => track.mpv_value === selectedSubtitlePreference,
  )
  const selectedSubtitleSummary = selectedSubtitlePreference === CUSTOM_SUBTITLE_VALUE
    ? (customSubtitlePreference.trim() || "Manual")
    : selectedSubtitlePreference === OFF_SUBTITLE_VALUE
      ? "Off"
      : selectedDetectedSubtitleTrack?.label || (selectedSubtitlePreference === AUTO_SUBTITLE_VALUE ? "Auto" : null)
  const playbackControlStatus = audioTracksLoading || subtitleTracksLoading
    ? "Reading tracks..."
    : selectedSeasonHasZipEpisodes
      ? "Learns more tracks as ZIP episodes are played."
      : (audioTracksStatus || subtitleTracksStatus || "Ready for this season.")

  const audioControls = isShow ? (
    <div className="w-full overflow-hidden rounded-lg border border-white/12 bg-black/38 shadow-[0_18px_60px_rgba(0,0,0,0.34)] backdrop-blur-2xl">
      <div className="flex items-center justify-between border-b border-white/10 px-3.5 py-2.5">
        <div>
          <p className="text-[10px] font-bold uppercase tracking-[0.28em] text-white/48">Playback</p>
          <p className="mt-1 max-w-[260px] truncate text-[11px] font-medium text-white/36">
            {playbackControlStatus}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-md border border-white/10 bg-white/8 px-2 py-1 text-[10px] font-semibold text-white/54">
            Season {selectedSeason}
          </span>
          <button
            type="button"
            onClick={() => setPlaybackSettingsOpen(false)}
            className="grid size-7 place-items-center rounded-md border border-white/10 bg-white/8 text-white/58 transition-colors hover:bg-white/14 hover:text-white"
            aria-label="Close playback settings"
          >
            <X className="size-3.5" />
          </button>
        </div>
      </div>

      <div className="divide-y divide-white/10">
        <div className="grid gap-2.5 px-3.5 py-3 sm:grid-cols-[96px_minmax(0,1fr)] sm:items-center">
          <div className="flex items-center gap-2 text-[11px] font-bold uppercase tracking-[0.18em] text-white/58">
            <AudioLines className="size-4 text-white/58" />
            Audio
          </div>
          <div className="min-w-0 space-y-2">
            <div className="relative">
              <select
                value={selectedAudioPreference}
                onChange={(event) => handleAudioPreferenceSelect(event.target.value)}
                aria-label="Audio track"
                title={selectedAudioSummary || "Auto"}
                className="h-10 w-full appearance-none rounded-md border border-white/12 bg-white/[0.07] px-3 pr-9 text-sm font-semibold text-white outline-none transition-colors hover:border-white/24 focus:border-white/45"
              >
                <option value={AUTO_AUDIO_VALUE} className="bg-[#101114] text-white">Auto</option>
                {detectedAudioTracks.map((track) => (
                  <option
                    key={`${track.stream_index}-${track.mpv_value || track.label}`}
                    value={track.mpv_value || ""}
                    disabled={!track.mpv_value}
                    className="bg-[#101114] text-white"
                  >
                    {track.label}
                  </option>
                ))}
                <option value={MANUAL_AUDIO_OPTION.value} className="bg-[#101114] text-white">Manual</option>
              </select>
              <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-4 -translate-y-1/2 text-white/50" />
            </div>

            {selectedAudioPreference === CUSTOM_AUDIO_VALUE && (
              <Input
                value={customAudioPreference}
                onChange={(e) => handleCustomAudioPreferenceChange(e.target.value)}
                placeholder="Language code, e.g. hi, hin, hindi"
                className="h-9 rounded-md border-white/12 bg-white/[0.07] px-3 text-sm text-white placeholder:text-white/32"
              />
            )}
          </div>
        </div>

        <div className="grid gap-2.5 px-3.5 py-3 sm:grid-cols-[96px_minmax(0,1fr)] sm:items-center">
          <div className="flex items-center gap-2 text-[11px] font-bold uppercase tracking-[0.18em] text-white/58">
            <Captions className="size-4 text-white/58" />
            Subtitles
          </div>
          <div className="min-w-0 space-y-2">
            <div className="relative">
              <select
                value={selectedSubtitlePreference}
                onChange={(event) => handleSubtitlePreferenceSelect(event.target.value)}
                aria-label="Subtitle track"
                title={selectedSubtitleSummary || "Auto"}
                className="h-10 w-full appearance-none rounded-md border border-white/12 bg-white/[0.07] px-3 pr-9 text-sm font-semibold text-white outline-none transition-colors hover:border-white/24 focus:border-white/45"
              >
                <option value={AUTO_SUBTITLE_VALUE} className="bg-[#101114] text-white">Auto</option>
                <option value={OFF_SUBTITLE_VALUE} className="bg-[#101114] text-white">Off</option>
                {detectedSubtitleTracks.map((track) => (
                  <option
                    key={`${track.stream_index}-${track.mpv_value || track.label}`}
                    value={track.mpv_value || ""}
                    disabled={!track.mpv_value}
                    className="bg-[#101114] text-white"
                  >
                    {track.label}
                  </option>
                ))}
                <option value={CUSTOM_SUBTITLE_VALUE} className="bg-[#101114] text-white">Manual</option>
              </select>
              <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-4 -translate-y-1/2 text-white/50" />
            </div>

            {selectedSubtitlePreference === CUSTOM_SUBTITLE_VALUE && (
              <Input
                value={customSubtitlePreference}
                onChange={(e) => handleCustomSubtitlePreferenceChange(e.target.value)}
                placeholder="Language code, e.g. eng or sid:2"
                className="h-9 rounded-md border-white/12 bg-white/[0.07] px-3 text-sm text-white placeholder:text-white/32"
              />
            )}
          </div>
        </div>
      </div>
    </div>
  ) : null

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange} modal={false}>
      <DialogPortal>
        <button
          type="button"
          tabIndex={-1}
          className="fixed inset-x-0 bottom-0 top-9 z-40 bg-black/52 backdrop-blur-md"
          onClick={() => onOpenChange(false)}
          onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onOpenChange(false) }}
        />
      </DialogPortal>
      <DialogContent
        onInteractOutside={(e) => e.preventDefault()}
        className="max-w-[1080px] w-[96vw] max-h-[92vh] h-auto bg-[#090a0d] border-white/10 text-white p-0 overflow-hidden flex flex-col shadow-2xl [&>button]:z-[100] [&>button]:bg-black/50 [&>button]:rounded-full [&>button]:p-1.5 [&>button]:hover:bg-black/80"
      >
        <DialogTitle className="sr-only">{displayTitle}</DialogTitle>
        <DialogDescription className="sr-only">Details for {displayTitle}</DialogDescription>

        <section className={cn(
          "relative w-full overflow-hidden flex flex-col bg-[#090a0d] shrink-0",
          isShow ? "h-[90vh]" : "h-[76vh] min-h-[520px]"
        )}>
          {/* Hero Backdrop */}
          <div className="absolute inset-0 z-0">
            {heroImageUrl ? (
              <>
                <img 
                  src={heroImageUrl} 
                  alt={displayTitle + " poster"}
                  className="w-full h-full object-cover transition-opacity duration-500" 
                />
                <div className="absolute inset-0 bg-gradient-to-t from-[#090a0d] via-[#090a0d]/85 to-black/60 backdrop-blur-[1px]" />
              </>
            ) : (
              <div className="w-full h-full bg-gradient-to-br from-[#11141d] to-[#07080b]" />
            )}
          </div>

          {/* Content Layer */}
          <div className="relative z-10 flex flex-col h-full min-h-0">
            <div className={cn("p-6 sm:px-10 shrink-0", isShow ? "pb-0 pt-7" : "mt-auto sm:py-10 pb-12")}>
              <div className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
                {posterImageUrl && !isShow && (
                  <div className="hidden sm:block w-[160px] aspect-[2/3] rounded-2xl overflow-hidden shadow-2xl border border-white/10 shrink-0 scale-100 hover:scale-[1.02] transition-transform duration-500">
                    <img src={posterImageUrl} alt={displayTitle + " poster"} className="w-full h-full object-cover" />
                  </div>
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2.5 mb-2.5">
                    <span className="px-2 py-0.5 rounded bg-white/5 text-[8px] font-bold uppercase tracking-[0.3em] text-white/40 backdrop-blur-md border border-white/5">
                      {isShow ? "TV Series" : isEpisode ? "TV Episode" : "Movie"}
                    </span>
                    {mediaFormatParts.length > 0 && !isShow && (
                      <span className="text-[9px] font-medium text-white/20 uppercase tracking-widest flex items-center gap-1.5">
                        <div className="size-0.5 rounded-full bg-white/10" />
                        {mediaFormatParts.join(" · ")}
                      </span>
                    )}
                  </div>
                  
                  <h2 className={cn(
                    "leading-[1.1] tracking-tight text-white mb-5 italic",
                    isShow ? "text-3xl sm:text-5xl" : "text-4xl sm:text-6xl"
                  )} style={{ fontFamily: 'Georgia, serif' }}>{displayTitle}</h2>
                  
                  <div className={cn(
                    "flex flex-wrap items-center gap-x-6 gap-y-3 text-[11.5px] font-semibold text-white/40",
                    isShow ? "mb-6" : "mb-8"
                  )}>
                    <div className="flex items-center gap-2">
                      <Calendar className="size-3.5 text-white/25" />
                      <span className="text-white/80">{displayItem.year || "N/A"}</span>
                    </div>
                    {!isShow && movieVoteAverage != null && (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation()
                          setImdbPanelImdbId(item?.imdb_id || `tmdb:${item?.tmdb_id}`)
                        }}
                        className="flex items-center gap-1.5 px-2 py-1 rounded-lg bg-white/10 text-white text-xs font-bold cursor-pointer hover:bg-white/20 transition-colors"
                      >
                        <Star className="size-3 fill-current text-yellow-500" />
                        {movieVoteAverage.toFixed(1)}
                      </button>
                    )}
                    {!isShow && (
                      <div className="flex items-center gap-2">
                        <Clock className="size-3.5 text-white/25" />
                        <span className="text-white/80">{runtimeLabel}</span>
                      </div>
                    )}
                    {isShow && (
                      <div className="flex items-center gap-2">
                        <Tv className="size-3.5 text-white/25" />
                        <span className="text-white/80">{seasons.length} Seasons</span>
                      </div>
                    )}
                    {(director || creator) && (
                      <button
                        type="button"
                        onClick={() => setCastPanelName(isShow ? creator : director)}
                        className="flex items-center gap-2 cursor-pointer hover:opacity-80 transition-opacity"
                      >
                        <User className="size-3.5 text-white/25" />
                        <span className="text-white/80">{isShow ? creator : director}</span>
                        <span className="text-[9px] uppercase tracking-widest opacity-25">{isShow ? "Creator" : "Director"}</span>
                      </button>
                    )}
                    {zipCompressionLabel && (
                      <div className="px-2.5 py-1 rounded bg-white/5 border border-white/5 text-[9px] font-bold text-white/50 tracking-widest ml-1">
                        ZIP: {zipCompressionLabel}
                      </div>
                    )}
                  </div>
                  
                  {!isShow && castList.length > 0 && (
                    <div className="flex flex-col gap-2.5 mb-8">
                      <p className="text-[9px] font-bold uppercase tracking-[0.3em] text-white/20">Cast</p>
                      <div className="flex flex-wrap gap-x-3 gap-y-1.5">
                        {castList.slice(0, 8).map(name => (
                          <button
                            type="button"
                            key={name}
                            onClick={() => setCastPanelName(name)}
                            className="text-[12px] font-medium text-white/40 hover:text-white/80 transition-colors cursor-pointer"
                          >
                            {name}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {!isShow && (
                    <div className="shrink-0 mb-2 flex items-center gap-4">
                      <Button
                        onClick={() => {
                          if (selectedSubtitlePath && displayItem.id) setPendingSubtitlePath(displayItem.id, selectedSubtitlePath)
                          onPrimaryAction(displayItem)
                        }}
                        className="h-14 px-10 rounded-full text-base font-bold tracking-tight shadow-[0_15px_40px_rgba(255,255,255,0.1)] hover:shadow-[0_20px_50px_rgba(255,255,255,0.15)] hover:scale-[1.02] active:scale-95 transition-all duration-500 bg-white text-black hover:bg-white"
                      >
                        <Play className="size-4 mr-2.5 fill-current" /> Play Now
                      </Button>

                      <div className="flex items-center gap-2 p-1 rounded-full bg-white/5 border border-white/10 shadow-sm">
                        {displayItem.is_cloud && displayItem.cloud_file_id && (
                          <button
                            type="button"
                            onClick={() => {
                              setShareFileId(displayItem.cloud_file_id || null)
                              setShareFileName(displayItem.title)
                            }}
                            className="size-10 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative"
                            title="Share"
                          >
                            <Share2 className="size-4" />
                            <span className="absolute bottom-full mb-3 left-1/2 -translate-x-1/2 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Share</span>
                          </button>
                        )}
                        {onDownloadAction && downloadActionLabel && displayItem.is_cloud && (
                          <button
                            type="button"
                            onClick={() => onDownloadAction(displayItem)}
                            className="size-10 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative"
                            title="Download"
                          >
                            <Download className="size-4" />
                            <span className="absolute bottom-full mb-3 left-1/2 -translate-x-1/2 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Download</span>
                          </button>
                        )}
                        {onSecondaryAction && secondaryActionLabel && (
                          <button
                            type="button"
                            onClick={() => onSecondaryAction(displayItem)}
                            className="size-10 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative"
                            title={secondaryActionLabel}
                          >
                            <Check className="size-4" />
                            <span className="absolute bottom-full mb-3 left-1/2 -translate-x-1/2 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">{secondaryActionLabel}</span>
                          </button>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            </div>

            {isShow && (
              <div className="flex-1 min-h-0 flex flex-col px-6 pb-6 sm:px-10 sm:pb-8 pt-0">
                <div className="flex items-center gap-3 mb-4 shrink-0">
                  <div className="flex min-w-0 flex-1 gap-2 overflow-x-auto no-scrollbar">
                    {seasons.map(s => (
                      <button
                        type="button"
                        key={s}
                        onClick={() => setSelectedSeason(s)}
                        className={cn(
                          "inline-flex h-8 items-center justify-center whitespace-nowrap rounded-[999px] px-4 text-[9px] leading-none font-bold uppercase tracking-[0.16em] border backdrop-blur-xl transition-all duration-300 shrink-0",
                          selectedSeason === s
                            ? "bg-white text-black border-white shadow-lg"
                            : "bg-white/10 text-white/75 border-white/10 hover:bg-white/15 hover:text-white hover:border-white/20"
                        )}
                      >
                        Season {s}
                      </button>
                    ))}
                  </div>

                  {/* The DOCK - Horizontal Action Bar */}
                  <div className="hidden sm:flex items-center gap-1.5 p-1 rounded-full bg-white/5 border border-white/10 shrink-0 shadow-sm">
                    {item?.tmdb_id && (
                      <button
                        type="button"
                        onClick={() => void handleRefreshMetadata()}
                        disabled={isRefreshingMetadata}
                        className="size-8 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative disabled:cursor-not-allowed disabled:opacity-45"
                        title="Refresh Metadata"
                      >
                        <RefreshCw className={cn("size-3.5", isRefreshingMetadata && "animate-spin")} />
                        <span className="absolute bottom-full mb-3 right-0 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Refresh</span>
                      </button>
                    )}
                    {filteredEpisodes.some(ep => ep.file_path || ep.zip_entry_path) && (
                      <button
                        type="button"
                        onClick={() => setShowEpisodeUrls(true)}
                        className="size-8 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative"
                        title="Show Episode Files"
                      >
                        <FileText className="size-3.5" />
                        <span className="absolute bottom-full mb-3 right-0 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Files</span>
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => setPlaybackSettingsOpen(true)}
                      className="size-8 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/50 hover:text-white transition-all duration-300 group relative"
                      title="Audio & Subtitles"
                    >
                      <SlidersHorizontal className="size-3.5" />
                      <span className="absolute bottom-full mb-3 right-0 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Audio</span>
                    </button>
                    {item?.imdb_id && (
                      <button
                        type="button"
                        onClick={() => setSubtitlePanelOpen(true)}
                        className={cn(
                          "size-8 flex items-center justify-center rounded-full transition-all duration-300 group relative",
                          selectedSubtitlePath
                            ? "bg-white/15 hover:bg-white/25 text-white"
                            : "bg-white/5 hover:bg-white/10 text-white/50 hover:text-white",
                        )}
                        title="OpenSubtitles"
                      >
                        <Captions className="size-3.5" />
                        <span className="absolute bottom-full mb-3 right-0 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Subs</span>
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={toggleSpoiler}
                      className={cn(
                        "size-8 flex items-center justify-center rounded-full transition-all duration-300 group relative",
                        spoilerEnabled
                          ? "bg-white/15 hover:bg-white/25 text-white"
                          : "bg-white/5 hover:bg-white/10 text-white/50 hover:text-white",
                      )}
                      title={`Spoiler ${spoilerEnabled ? "On" : "Off"}`}
                    >
                      {spoilerEnabled ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                      <span className="absolute bottom-full mb-3 right-0 px-1.5 py-0.5 rounded bg-black/80 text-[9px] text-white font-bold opacity-0 group-hover:opacity-100 pointer-events-none transition-opacity uppercase tracking-widest whitespace-nowrap">Spoiler {spoilerEnabled ? "On" : "Off"}</span>
                    </button>
                  </div>
                </div>
                
                <div className="flex-1 min-h-0 relative">
                  <div className="h-full w-full overflow-y-auto no-scrollbar">
                    {loadingEpisodes ? (
                      <div className="py-20 flex flex-col items-center text-white/30">
                        <Loader2 className="size-12 animate-spin mb-4" />
                        <p className="font-medium tracking-wide">Loading episodes…</p>
                      </div>
                    ) : filteredEpisodes.length === 0 ? (
                      <div className="py-20 text-center text-white/30">
                        <p className="font-medium tracking-wide">No episodes found for this season.</p>
                      </div>
                    ) : (
                      <div className="grid grid-cols-1 gap-3 pb-8">
                        {filteredEpisodes.map(ep => {
                          const tmdbData = tmdbEpisodesBySeason.get(selectedSeason)?.get(ep.episode_number || 0)
                          const imdbRatingData = imdbEpisodeRatings[ep.episode_number || 0]
                          const rating = imdbRatingData?.imdb_rating ?? tmdbData?.vote_average
                          const airDate = tmdbData?.air_date
                          const runtime = tmdbData?.runtime
                          const isWatched = isMediaMarkedWatched(ep)
                          const isSpoiler = spoilerEnabled && !isWatched && !revealedEpisodes.has(ep.id)
                          const episodeSizeLabel = formatEpisodeSize(getPreferredEpisodeSize(ep))
                          const episodeZipCompressionLabel = ep.parent_zip_id
                            ? getZipCompressionLabel(ep.zip_compression_method)
                            : null
                          const episodeMediaParts = buildDisplayMediaParts(
                            ep,
                            selectedSeasonHasZipEpisodes ? technicalDetails : null,
                            false,
                          )

                          return (
                            <button
                              type="button"
                              key={ep.id}
                              tabIndex={0}
                              onClick={() => {
                                if (selectedSubtitlePath && ep.id) setPendingSubtitlePath(ep.id, selectedSubtitlePath)
                                onPrimaryAction(ep)
                              }}
                              onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") {
                                if (selectedSubtitlePath && ep.id) setPendingSubtitlePath(ep.id, selectedSubtitlePath)
                                onPrimaryAction(ep)
                              } }}
                              className="group flex gap-4 p-3 rounded-[1.6rem] bg-white/[0.03] border border-white/[0.05] hover:bg-white/[0.08] hover:border-white/10 transition-all duration-300 cursor-pointer shadow-sm hover:shadow-2xl"
                            >
                              <div className="relative w-36 sm:w-48 aspect-video rounded-2xl overflow-hidden shrink-0 bg-white/5 shadow-lg">
                                <div className={cn("w-full h-full", isSpoiler && "blur-md")}>
                                  <EpisodeThumbnailImage
                                    localStillPath={ep.still_path || imdbRatingData?.still_url || undefined}
                                    tmdbStillUrl={getTmdbImageUrl(ep.still_path || tmdbData?.still_path, 'w300')}
                                    episodeTitle={tmdbData?.name || imdbRatingData?.title || ep.title}
                                    episodeNumber={ep.episode_number || 0}
                                  />
                                </div>
                                <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity bg-black/40">
                                  <div className="size-12 rounded-full bg-white flex items-center justify-center shadow-2xl scale-90 group-hover:scale-100 transition-transform duration-300">
                                    <Play className="size-6 text-black fill-black ml-1" />
                                  </div>
                                </div>
                                {ep.progress_percent ? (
                                  <div className="absolute bottom-0 left-0 right-0 h-1.5 bg-black/40">
                                    <div
                                      className="h-full bg-white shadow-[0_0_8px_rgba(255,255,255,0.8)]"
                                      style={{ width: `${ep.progress_percent}%` }}
                                    />
                                  </div>
                                ) : null}
                                {isWatched ? (
                                  <div className="absolute top-3 right-3 p-1.5 rounded-xl bg-black/60 backdrop-blur-md text-white border border-white/10 shadow-lg">
                                    <Check className="size-4" />
                                  </div>
                                ) : null}
                              </div>
                              <div className="flex-1 min-w-0 py-0.5">
                                <div className="flex justify-between items-start mb-1.5">
                                  <div className="min-w-0">
                                    <div className="flex items-center gap-2 mb-1 flex-wrap">
                                      <p className="text-[10px] font-bold text-white/40 uppercase tracking-[0.2em]">EPISODE {ep.episode_number}</p>
                                      {episodeZipCompressionLabel && (
                                        <span className="rounded-lg border border-white/10 bg-white/10 px-2 py-1 text-[10px] font-bold uppercase tracking-[0.12em] text-white/80">
                                          ZIP: {episodeZipCompressionLabel}
                                        </span>
                                      )}
                                    </div>
                                    <h4 className="text-base font-bold text-white line-clamp-1 group-hover:text-white transition-colors tracking-tight">{tmdbData?.name || imdbRatingData?.title || ep.title}</h4>
                                  </div>
                                  <div className="flex items-center gap-3 shrink-0 mt-0.5">
                                    {isWatched && (
                                      <div className="inline-flex items-center gap-1.5 rounded-full border border-green-500/35 bg-green-500/18 px-3 py-1.5 text-[10px] font-bold uppercase tracking-[0.12em] text-green-300">
                                        <Check className="size-3.5" />
                                        Watched
                                      </div>
                                    )}
                                    <KebabMenu items={[
                                      ...(isWatched && onEpisodeUnwatchAction ? [{ icon: Check, label: "Unmark Watched", onClick: () => handleEpisodeUnwatched(ep) }] : []),
                                      ...(!isWatched && onEpisodeSecondaryAction ? [{ icon: Check, label: episodeSecondaryActionLabel || "Mark Watched", onClick: () => void handleEpisodeMarkWatched(ep) }] : []),
                                      ...(onDownloadAction && ep.is_cloud ? [{ icon: Download, label: "Download", onClick: () => void onDownloadAction(ep) }] : []),
                                      ...(ep.is_cloud && ep.cloud_file_id ? [{ icon: Share2, label: "Share", onClick: () => { setShareFileId(ep.cloud_file_id || null); setShareFileName(ep.episode_title || ep.title) } }] : []),
                                    ]} />
                                    {rating && rating > 0 && (() => {
                                      const clickableId = imdbRatingData?.imdb_id || item?.imdb_id
                                      return (
                                        <button
                                          type="button"
                                          onClick={(e) => {
                                            e.stopPropagation()
                                            if (clickableId) setImdbPanelImdbId(clickableId)
                                          }}
                                          className={cn(
                                            "flex items-center gap-1.5 text-xs font-bold text-white bg-white/10 px-2 py-1 rounded-lg transition-colors",
                                            clickableId && "cursor-pointer hover:bg-white/20"
                                          )}
                                        >
                                          <Star className="size-3 fill-current text-yellow-500" />
                                          {rating.toFixed(1)}
                                        </button>
                                      )
                                    })()}
                                    {(ep.duration_seconds || runtime) && (
                                      <div className="flex items-center gap-1.5 text-xs font-bold text-white/60">
                                        <Timer className="size-3.5 opacity-70" />
                                        {Math.round((ep.duration_seconds || (runtime ? runtime * 60 : 0)) / 60)}m
                                      </div>
                                    )}
                                  </div>
                                </div>
                                
                                {(airDate || episodeSizeLabel || episodeMediaParts.length > 0) && (
                                  <div className="mb-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] font-bold uppercase tracking-[0.15em] text-white/30">
                                    {airDate && (
                                      <span className="inline-flex items-center gap-2">
                                        <Calendar className="size-3 opacity-50" />
                                        {new Date(airDate).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })}
                                      </span>
                                    )}
                                    {episodeMediaParts.length > 0 && (
                                      <span className="inline-flex items-center gap-2 font-extrabold text-white/72">
                                        {airDate && <span className="text-white/18">•</span>}
                                        <span>{episodeMediaParts.join(" · ")}</span>
                                      </span>
                                    )}
                                    {episodeSizeLabel && (
                                      <span className="inline-flex items-center gap-2 font-extrabold text-white">
                                        {(airDate || episodeMediaParts.length > 0) && <span className="text-white/18">•</span>}
                                        <span>{episodeSizeLabel}</span>
                                      </span>
                                    )}
                                  </div>
                                )}

                                {spoilerEnabled && !isWatched && (
                                  <button
                                    type="button"
                                    onClick={(e) => {
                                      e.stopPropagation()
                                      handleToggleSpoiler(ep)
                                    }}
                                    className={cn(
                                      "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full text-[9px] font-bold uppercase tracking-wider transition-all duration-200 w-fit",
                                      revealedEpisodes.has(ep.id)
                                        ? "bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 text-white/50 hover:text-white"
                                        : "bg-white/10 hover:bg-white/20 border border-white/20 hover:border-white/30 text-white/70 hover:text-white",
                                    )}
                                  >
                                    {revealedEpisodes.has(ep.id) ? (
                                      <>
                                        <Eye className="size-3" />
                                        Hide Spoilers
                                      </>
                                    ) : (
                                      <>
                                        <EyeOff className="size-3" />
                                        Show Spoilers
                                      </>
                                    )}
                                  </button>
                                )}

                                <p className={cn("text-sm text-white/50 line-clamp-2 leading-snug group-hover:text-white/70 transition-colors", isSpoiler && "blur-sm")}>{ep.overview || tmdbData?.overview || imdbRatingData?.plot || "No description available."}</p>
                              </div>
                            </button>
                          )
                        })}
                      </div>
                    )}
                  </div>
                  
                  {/* Subtle bottom fade to indicate more content */}
                  {filteredEpisodes.length > 3 && (
                    <div className="absolute bottom-0 left-0 right-0 h-20 bg-gradient-to-t from-[#090a0d] via-[#090a0d]/40 to-transparent pointer-events-none z-20" />
                  )}
                </div>
              </div>
            )}
          </div>

          {audioControls && playbackSettingsOpen && (
            <button
              type="button"
              tabIndex={-1}
              className="absolute inset-0 z-40 flex items-center justify-center bg-black/45 px-4 backdrop-blur-[3px]"
              onClick={() => setPlaybackSettingsOpen(false)}
              onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") setPlaybackSettingsOpen(false) }}
            >
              <div
                className="w-full max-w-[430px]"
                onClick={(event) => event.stopPropagation()}
              >
                {audioControls}
              </div>
            </button>
          )}

          {subtitlePanelOpen && item && (
            <button
              type="button"
              tabIndex={-1}
              className="absolute inset-0 z-40 flex items-center justify-center bg-black/45 px-4 backdrop-blur-[3px]"
              onClick={() => setSubtitlePanelOpen(false)}
              onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") setSubtitlePanelOpen(false) }}
            >
              <div
                className="w-full max-w-[430px]"
                onClick={(event) => event.stopPropagation()}
              >
                <SubtitleSelector
                  item={item}
                  onSubtitleReady={(path) => {
                    setSelectedSubtitlePath(path)
                    setSubtitlePanelOpen(false)
                    toast({ title: "Subtitle ready", description: "Will be used on next playback." })
                  }}
                />
              </div>
            </button>
          )}

          {shareFileId && (
            <ShareDialog
              open={true}
              onOpenChange={() => setShareFileId(null)}
              fileId={shareFileId}
              fileName={shareFileName}
            />
          )}
        </section>
      </DialogContent>

      <Dialog open={showEpisodeUrls} onOpenChange={setShowEpisodeUrls}>
        <DialogContent className="sm:max-w-2xl max-h-[80vh] !h-[80vh] flex flex-col">
          <DialogTitle className="text-lg font-bold text-white px-1 shrink-0">
            Episode Files: {item?.title} (Season {selectedSeason})
          </DialogTitle>
          <DialogDescription className="sr-only">
            File names for each episode in season {selectedSeason}
          </DialogDescription>
          <ScrollArea className="flex-1 min-h-0 -mx-6 px-6">
            <div className="flex flex-col gap-2 py-2">
              {filteredEpisodes
                .filter(ep => ep.file_path || ep.zip_entry_path)
                .sort((a, b) => (a.episode_number || 0) - (b.episode_number || 0))
                .map(ep => {
                  const episodeLabel = `S${String(ep.season_number || selectedSeason).padStart(2, '0')}E${String(ep.episode_number || 0).padStart(2, '0')} — ${ep.episode_title || ep.title}`
                  const fileName = (() => {
                    const p = ep.file_path || ep.zip_entry_path
                    if (!p) return ''
                    const norm = p.replace(/\\/g, '/')
                    const idx = norm.lastIndexOf('/')
                    return idx >= 0 ? norm.slice(idx + 1) : norm
                  })()
                  return (
                    <div key={ep.id} className="flex items-start gap-2 p-3 rounded-lg bg-white/[0.03] border border-white/[0.06] hover:bg-white/[0.06] transition-colors">
                      <div className="flex-1 min-w-0">
                        <p className="text-sm font-semibold text-white/90 truncate">{episodeLabel}</p>
                        <p className="text-xs text-white/50 break-all mt-0.5 select-all">{fileName}</p>
                      </div>
                      <button
                        type="button"
                        onClick={() => {
                          navigator.clipboard.writeText(fileName)
                        }}
                        className="flex items-center gap-1 shrink-0 h-8 px-2.5 rounded-md bg-white/10 hover:bg-white/15 text-white/70 hover:text-white text-xs font-medium transition-colors"
                        title="Copy file name"
                      >
                        <Copy className="size-3.5" />
                      </button>
                    </div>
                  )
                })}
              {filteredEpisodes.filter(ep => ep.file_path || ep.zip_entry_path).length === 0 && (
                <p className="text-sm text-white/40 text-center py-8">No file path info available for episodes in this season.</p>
              )}
            </div>
          </ScrollArea>
        </DialogContent>
      </Dialog>
    </Dialog>
    {imdbPanelImdbId && (
      <ImdbDetailsPanel
        open={true}
        onOpenChange={() => setImdbPanelImdbId(null)}
        imdbId={imdbPanelImdbId}
        tmdbId={item?.tmdb_id ? Number(item.tmdb_id) : undefined}
        mediaType={item?.media_type === "tvshow" ? "tv" : "movie"}
      />
    )}
    {castPanelName && (
      <CastMemberPanel
        open={true}
        onOpenChange={() => setCastPanelName(null)}
        castName={castPanelName}
        onItemClick={(clickedItem) => {
          setCastPanelName(null)
          onPrimaryAction(clickedItem)
        }}
      />
    )}
    </>
  )
}
