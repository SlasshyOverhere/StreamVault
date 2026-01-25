import { useEffect, useState } from "react"
import { listen, UnlistenFn } from "@tauri-apps/api/event"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Play, ChevronLeft, Clock, Check, Loader2, Star, Timer, ChevronDown, ChevronUp, RefreshCw } from "lucide-react"
import {
    MediaItem, getEpisodes, playMedia, getResumeInfo,
    getCachedImageUrl, ResumeInfo, getTvSeasonEpisodes, TmdbEpisodeInfo,
    getTmdbImageUrl, refreshSeriesMetadata
} from "@/services/api"
import { useToast } from "@/components/ui/use-toast"
import { PlayerModal } from "@/components/PlayerModal"
import { ResumeDialog } from "@/components/ResumeDialog"
import { motion } from "framer-motion"
import { cn } from "@/lib/utils"

interface EpisodeBrowserProps {
    show: MediaItem
    onBack: () => void
}

// Component to handle episode thumbnail loading from local cache or TMDB
function EpisodeThumbnailImage({
    localStillPath,
    tmdbStillUrl,
    episodeTitle,
    episodeNumber
}: {
    localStillPath?: string;
    tmdbStillUrl: string | null;
    episodeTitle: string;
    episodeNumber: number;
}) {
    const [imageUrl, setImageUrl] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        const loadImage = async () => {
            setLoading(true);
            setImageUrl(null);

            if (localStillPath) {
                // Load from local cache - handle paths with or without 'image_cache/' prefix
                let filename = localStillPath;
                if (filename.startsWith('image_cache/')) {
                    filename = filename.replace('image_cache/', '');
                }
                console.log(`[EpisodeThumbnail] Loading local image for E${episodeNumber}: ${filename}`);

                try {
                    const cachedUrl = await getCachedImageUrl(filename);
                    if (cachedUrl) {
                        console.log(`[EpisodeThumbnail] Successfully loaded local image for E${episodeNumber}`);
                        setImageUrl(cachedUrl);
                        setLoading(false);
                        return;
                    } else {
                        console.log(`[EpisodeThumbnail] getCachedImageUrl returned null for E${episodeNumber}`);
                    }
                } catch (error) {
                    console.log(`[EpisodeThumbnail] Failed to load local image for E${episodeNumber}:`, error);
                }
            } else {
                console.log(`[EpisodeThumbnail] No localStillPath for E${episodeNumber}`);
            }

            // Fall back to TMDB URL
            if (tmdbStillUrl) {
                console.log(`[EpisodeThumbnail] Falling back to TMDB URL for E${episodeNumber}`);
                setImageUrl(tmdbStillUrl);
            }
            setLoading(false);
        };
        loadImage();
    }, [localStillPath, tmdbStillUrl, episodeNumber]);

    if (loading) {
        return (
            <div className="w-full h-full flex items-center justify-center bg-gradient-to-br from-muted to-muted/50">
                <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/50" />
            </div>
        );
    }

    if (imageUrl) {
        return (
            <img
                src={imageUrl}
                alt={episodeTitle}
                className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-105"
            />
        );
    }

    return (
        <div className="w-full h-full flex items-center justify-center bg-gradient-to-br from-muted to-muted/50">
            <span className="text-2xl font-bold text-muted-foreground/50">
                {episodeNumber > 0 ? episodeNumber : '?'}
            </span>
        </div>
    );
}

export function EpisodeBrowser({ show, onBack }: EpisodeBrowserProps) {
    const [episodes, setEpisodes] = useState<MediaItem[]>([])
    const [loading, setLoading] = useState(true)
    const [posterUrl, setPosterUrl] = useState<string | null>(null)
    const [selectedSeason, setSelectedSeason] = useState<number>(1)
    const { toast } = useToast()

    // TMDB episode metadata
    const [tmdbEpisodes, setTmdbEpisodes] = useState<Map<number, TmdbEpisodeInfo>>(new Map())
    const [expandedEpisode, setExpandedEpisode] = useState<number | null>(null)

    // Player selection state
    const [playerModalOpen, setPlayerModalOpen] = useState(false)
    const [pendingPlayEpisode, setPendingPlayEpisode] = useState<MediaItem | null>(null)
    const [pendingResumeTime, setPendingResumeTime] = useState(0)

    // Resume dialog state
    const [resumeDialogOpen, setResumeDialogOpen] = useState(false)
    const [resumeDialogData, setResumeDialogData] = useState<{
        episode: MediaItem;
        resumeInfo: ResumeInfo;
    } | null>(null)

    // Metadata refresh state
    const [isRefreshing, setIsRefreshing] = useState(false)

    useEffect(() => {
        loadEpisodes()
        loadPoster()

        let unlistenMpvEnded: UnlistenFn | undefined;
        let unlistenMarkedComplete: UnlistenFn | undefined;

        const setupListener = async () => {
            unlistenMpvEnded = await listen('mpv-playback-ended', () => {
                loadEpisodes();
            });
            // Listen for mark complete events from the dialog
            unlistenMarkedComplete = await listen('media-marked-complete', () => {
                loadEpisodes();
            });
        };

        setupListener();

        return () => {
            unlistenMpvEnded?.();
            unlistenMarkedComplete?.();
        };
    }, [show.id])

    // Load TMDB episode metadata when season changes - only if local data is incomplete
    useEffect(() => {
        // Check if local episodes already have the metadata we need
        const seasonEpisodes = episodes.filter(ep => (ep.season_number || 1) === selectedSeason);
        const hasLocalMetadata = seasonEpisodes.length > 0 && seasonEpisodes.some(ep => ep.episode_title || ep.still_path);

        // Only fetch from TMDB if we don't have local metadata
        if (show.tmdb_id && selectedSeason > 0 && !hasLocalMetadata && seasonEpisodes.length === 0) {
            loadTmdbEpisodes()
        }
    }, [show.tmdb_id, selectedSeason, episodes])

    const loadTmdbEpisodes = async () => {
        if (!show.tmdb_id) return

        try {
            const tmdbId = parseInt(show.tmdb_id)
            const seasonDetails = await getTvSeasonEpisodes(tmdbId, selectedSeason)

            if (seasonDetails) {
                const episodeMap = new Map<number, TmdbEpisodeInfo>()
                seasonDetails.episodes.forEach(ep => {
                    episodeMap.set(ep.episode_number, ep)
                })
                setTmdbEpisodes(episodeMap)
            }
        } catch (error) {
            console.error("Failed to load TMDB episode metadata", error)
        }
    }

    const loadEpisodes = async () => {
        try {
            const data = await getEpisodes(show.id)
            setEpisodes(data)
            // Set initial season ONLY on first load (when episodes is empty)
            if (data.length > 0 && episodes.length === 0) {
                const firstSeason = data.reduce((min, ep) =>
                    ep.season_number && ep.season_number < min ? ep.season_number : min,
                    data[0].season_number || 1
                )
                setSelectedSeason(firstSeason)
            }
        } catch (error) {
            console.error("Failed to load episodes", error)
            toast({ title: "Error", description: "Failed to load episodes", variant: "destructive" })
        } finally {
            setLoading(false)
        }
    }

    const loadPoster = async () => {
        if (show.poster_path) {
            const filename = show.poster_path.replace('image_cache/', '');
            const url = await getCachedImageUrl(filename);
            if (url) {
                setPosterUrl(url);
            }
        }
    }

    const handleRefreshMetadata = async () => {
        if (!show.tmdb_id || isRefreshing) return;

        setIsRefreshing(true);
        try {
            const tmdbId = parseInt(show.tmdb_id);
            const result = await refreshSeriesMetadata(tmdbId, show.title);
            toast({ title: "Metadata Refreshed", description: result });
            // Reload episodes to get updated metadata
            await loadEpisodes();
        } catch (error) {
            toast({ title: "Error", description: "Failed to refresh metadata", variant: "destructive" });
        } finally {
            setIsRefreshing(false);
        }
    }

    // Get unique seasons
    const seasons = [...new Set(episodes.map(ep => ep.season_number || 1))].sort((a, b) => a - b)

    // Filter episodes by selected season
    const filteredEpisodes = episodes
        .filter(ep => (ep.season_number || 1) === selectedSeason)
        .sort((a, b) => (a.episode_number || 0) - (b.episode_number || 0))

    const handlePlay = async (episode: MediaItem) => {
        try {
            const resumeInfo = await getResumeInfo(episode.id);

            if (resumeInfo.has_progress && resumeInfo.progress_percent < 95) {
                setResumeDialogData({ episode, resumeInfo });
                setResumeDialogOpen(true);
            } else {
                await startPlayback(episode, 0);
            }
        } catch (e) {
            toast({ title: "Error", description: "Failed to start playback", variant: "destructive" })
        }
    }

    const handleResumeChoice = async (resume: boolean) => {
        if (!resumeDialogData) return;
        const { episode, resumeInfo } = resumeDialogData;
        const resumeTime = resume ? resumeInfo.position : 0;
        await startPlayback(episode, resumeTime);
    }

    const startPlayback = async (episode: MediaItem, resumeTime: number) => {
        try {
            await playMedia(episode.id, resumeTime > 0);
            toast({
                title: "Playing",
                description: `Now playing S${String(episode.season_number).padStart(2, '0')}E${String(episode.episode_number).padStart(2, '0')}`
            })
        } catch (e) {
            toast({ title: "Error", description: "Failed to start playback", variant: "destructive" })
        }
    }

    const handlePlayerSelect = async (player: 'mpv' | 'vlc' | 'builtin' | 'stream') => {
        if (!pendingPlayEpisode) return;

        // Only MPV is supported now
        if (player === 'mpv') {
            try {
                await playMedia(pendingPlayEpisode.id, pendingResumeTime > 0);
                toast({
                    title: "Playing",
                    description: `Now playing S${String(pendingPlayEpisode.season_number).padStart(2, '0')}E${String(pendingPlayEpisode.episode_number).padStart(2, '0')}`
                })
            } catch (e) {
                toast({ title: "Error", description: "Failed to start playback", variant: "destructive" })
            }
        }

        setPendingPlayEpisode(null);
        setPendingResumeTime(0);
    }

    const imageSrc = posterUrl || `https://placehold.co/400x600/1a1a2e/3a3a4e?text=${encodeURIComponent(show.title.slice(0, 2))}`;

    return (
        <>
            <motion.div
                initial={{ opacity: 0, x: 20 }}
                animate={{ opacity: 1, x: 0 }}
                className="h-full flex flex-col overflow-hidden"
            >
                {/* Back Button - Fixed at top */}
                <button
                    onClick={onBack}
                    className="flex items-center gap-2 text-muted-foreground hover:text-foreground transition-colors mb-4 w-fit flex-shrink-0"
                >
                    <ChevronLeft className="w-4 h-4" />
                    <span className="text-sm font-medium">
                        {show.is_cloud ? 'Back to Cloud TV Shows' : 'Back to TV Shows'}
                    </span>
                </button>

                {/* Main Content - Two column layout */}
                <div className="flex flex-col lg:flex-row gap-4 flex-1 min-h-0">
                    {/* Left: Show Info - Fixed/Sticky sidebar */}
                    <div className="w-full lg:w-48 xl:w-56 flex-shrink-0 lg:h-full lg:overflow-y-auto">
                        {/* Poster - smaller on lg screens */}
                        <div className="rounded-xl overflow-hidden shadow-elevation-2 mb-3 lg:mb-4">
                            <img
                                src={imageSrc}
                                alt={show.title}
                                className="w-full aspect-[2/3] object-cover max-h-[200px] lg:max-h-none"
                            />
                        </div>

                        {/* Title & Info */}
                        <h1 className="text-base lg:text-lg xl:text-xl font-bold text-foreground mb-1 lg:mb-2 line-clamp-2">{show.title}</h1>
                        {show.year && (
                            <p className="text-sm text-muted-foreground mb-2 lg:mb-3">{show.year}</p>
                        )}

                        {/* Stats */}
                        <div className="flex gap-1.5 lg:gap-2 mb-2 lg:mb-3 flex-wrap">
                            <div className="px-2 py-0.5 lg:px-2.5 lg:py-1 rounded-lg bg-muted text-xs">
                                {seasons.length} Season{seasons.length !== 1 ? 's' : ''}
                            </div>
                            <div className="px-2 py-0.5 lg:px-2.5 lg:py-1 rounded-lg bg-muted text-xs">
                                {episodes.length} Ep{episodes.length !== 1 ? 's' : ''}
                            </div>
                            {show.tmdb_id && (
                                <button
                                    onClick={handleRefreshMetadata}
                                    disabled={isRefreshing}
                                    className="px-2 py-0.5 lg:px-2.5 lg:py-1 rounded-lg bg-muted hover:bg-muted/80 text-xs flex items-center gap-1 transition-colors disabled:opacity-50"
                                    title="Refresh metadata and images from TMDB"
                                >
                                    <RefreshCw className={cn("w-3 h-3", isRefreshing && "animate-spin")} />
                                    {isRefreshing ? "..." : "↻"}
                                </button>
                            )}
                        </div>

                        {/* Overview - hidden on smaller screens */}
                        {show.overview && (
                            <p className="text-xs text-muted-foreground leading-relaxed line-clamp-3 hidden xl:block">
                                {show.overview}
                            </p>
                        )}
                    </div>

                    {/* Right: Episodes Panel - This is the scrolling area */}
                    <div className="flex-1 flex flex-col min-h-0 h-full">
                        {/* Season Tabs - Fixed at top of episode panel */}
                        {seasons.length > 1 && (
                            <div className="flex gap-2 mb-3 flex-wrap flex-shrink-0">
                                {seasons.map((season) => (
                                    <button
                                        key={season}
                                        onClick={() => setSelectedSeason(season)}
                                        className={cn(
                                            "px-3 py-1.5 rounded-lg text-xs lg:text-sm font-medium transition-all duration-200",
                                            selectedSeason === season
                                                ? "bg-white text-black"
                                                : "bg-muted text-muted-foreground hover:text-foreground hover:bg-muted/80"
                                        )}
                                    >
                                        Season {season}
                                    </button>
                                ))}
                            </div>
                        )}

                        {/* Episode List */}
                        <div className="flex-1 rounded-xl border border-border bg-card overflow-hidden min-h-0">
                            <ScrollArea className="h-full">
                                {loading ? (
                                    <div className="p-8 flex items-center justify-center text-muted-foreground">
                                        <Loader2 className="w-6 h-6 animate-spin mr-2" />
                                        Loading episodes...
                                    </div>
                                ) : filteredEpisodes.length === 0 ? (
                                    <div className="p-8 text-center text-muted-foreground">
                                        No episodes found for Season {selectedSeason}
                                    </div>
                                ) : (
                                    <div className="divide-y divide-border">
                                        {filteredEpisodes.map((episode, index) => {
                                            const progress = episode.progress_percent ||
                                                (episode.resume_position_seconds && episode.duration_seconds
                                                    ? (episode.resume_position_seconds / episode.duration_seconds) * 100
                                                    : 0);
                                            const isFinished = progress >= 95;
                                            const hasProgress = progress > 0 && !isFinished;

                                            // Use local data first, fall back to TMDB data
                                            const tmdbData = tmdbEpisodes.get(episode.episode_number || 0);
                                            // Prefer local still_path over TMDB
                                            const localStillPath = episode.still_path;
                                            const stillUrl = localStillPath
                                                ? null // Will be loaded from cache below
                                                : getTmdbImageUrl(tmdbData?.still_path, 'w300');
                                            // Use local episode title first, then TMDB, then fallback
                                            const episodeTitle = episode.episode_title || tmdbData?.name || episode.title || `Episode ${episode.episode_number}`;
                                            const isExpanded = expandedEpisode === episode.id;

                                            // Debug logging for episode data
                                            console.log(`[Episode S${episode.season_number}E${episode.episode_number}] still_path: ${episode.still_path}, episode_title: ${episode.episode_title}`);

                                            return (
                                                <motion.div
                                                    key={episode.id}
                                                    initial={{ opacity: 0, y: 10 }}
                                                    animate={{ opacity: 1, y: 0 }}
                                                    transition={{ delay: index * 0.02 }}
                                                    className="hover:bg-muted/30 transition-colors"
                                                >
                                                    <div
                                                        onClick={() => handlePlay(episode)}
                                                        className="p-3 lg:p-4 cursor-pointer group"
                                                    >
                                                        <div className="flex gap-3 lg:gap-4">
                                                            {/* Episode Thumbnail */}
                                                            <div className="relative flex-shrink-0 w-28 md:w-36 lg:w-40 aspect-video rounded-lg overflow-hidden bg-muted">
                                                                <EpisodeThumbnailImage
                                                                    localStillPath={localStillPath}
                                                                    tmdbStillUrl={stillUrl}
                                                                    episodeTitle={episodeTitle}
                                                                    episodeNumber={episode.episode_number || 0}
                                                                />

                                                                {/* Play overlay */}
                                                                <div className="absolute inset-0 bg-black/40 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
                                                                    <div className="w-8 h-8 lg:w-10 lg:h-10 rounded-full bg-white flex items-center justify-center">
                                                                        <Play className="w-4 h-4 lg:w-5 lg:h-5 text-black fill-black ml-0.5" />
                                                                    </div>
                                                                </div>

                                                                {/* Progress bar on thumbnail */}
                                                                {hasProgress && (
                                                                    <div className="absolute bottom-0 left-0 right-0 h-1 bg-black/50">
                                                                        <div
                                                                            className="h-full bg-white"
                                                                            style={{ width: `${progress}%` }}
                                                                        />
                                                                    </div>
                                                                )}

                                                                {/* Watched badge */}
                                                                {isFinished && (
                                                                    <div className="absolute top-1.5 right-1.5 lg:top-2 lg:right-2 px-1.5 py-0.5 rounded bg-gray-500 text-white text-[10px] lg:text-xs font-medium flex items-center gap-1">
                                                                        <Check className="w-2.5 h-2.5 lg:w-3 lg:h-3" />
                                                                    </div>
                                                                )}
                                                            </div>

                                                            {/* Episode Info */}
                                                            <div className="flex-1 min-w-0 py-0.5 lg:py-1">
                                                                {/* Header row */}
                                                                <div className="flex items-start justify-between gap-2">
                                                                    <div className="flex-1 min-w-0">
                                                                        <div className="flex items-center gap-2 mb-0.5 lg:mb-1">
                                                                            <span className="text-[10px] lg:text-xs font-medium text-white">
                                                                                Episode {episode.episode_number}
                                                                            </span>
                                                                            {isFinished && (
                                                                                <span className="flex items-center gap-1 text-[10px] lg:text-xs px-1.5 py-0.5 rounded bg-green-500/20 text-green-400 border border-green-500/30">
                                                                                    <Check className="w-2.5 h-2.5 lg:w-3 lg:h-3" />
                                                                                    Watched
                                                                                </span>
                                                                            )}
                                                                            {hasProgress && (
                                                                                <span className="flex items-center gap-1 text-[10px] lg:text-xs px-1.5 py-0.5 rounded bg-white/15 text-white border border-white/20">
                                                                                    <Clock className="w-2.5 h-2.5 lg:w-3 lg:h-3" />
                                                                                    {Math.round(progress)}%
                                                                                </span>
                                                                            )}
                                                                        </div>
                                                                        <h4 className="font-semibold text-foreground line-clamp-1 text-sm lg:text-base">
                                                                            {episodeTitle}
                                                                        </h4>
                                                                    </div>

                                                                    {/* Play button - hidden on small screens */}
                                                                    <Button
                                                                        size="sm"
                                                                        className="flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity hidden md:flex"
                                                                        onClick={(e) => {
                                                                            e.stopPropagation();
                                                                            handlePlay(episode);
                                                                        }}
                                                                    >
                                                                        <Play className="w-4 h-4 fill-current mr-1" />
                                                                        Play
                                                                    </Button>
                                                                </div>

                                                                {/* Metadata row - show duration from local file if available */}
                                                                {((episode.duration_seconds && episode.duration_seconds >= 60) || (tmdbData?.vote_average && tmdbData.vote_average > 0)) && (
                                                                    <div className="flex items-center gap-2 lg:gap-3 mt-1 lg:mt-1.5 text-[10px] lg:text-xs text-muted-foreground">
                                                                        {episode.duration_seconds && episode.duration_seconds >= 60 && (
                                                                            <span className="flex items-center gap-1">
                                                                                <Timer className="w-2.5 h-2.5 lg:w-3 lg:h-3" />
                                                                                {Math.round(episode.duration_seconds / 60)} min
                                                                            </span>
                                                                        )}
                                                                        {tmdbData?.vote_average && tmdbData.vote_average > 0 && (
                                                                            <span className="flex items-center gap-1">
                                                                                <Star className="w-2.5 h-2.5 lg:w-3 lg:h-3 text-gray-400 fill-gray-400" />
                                                                                {tmdbData.vote_average.toFixed(1)}
                                                                            </span>
                                                                        )}
                                                                    </div>
                                                                )}

                                                                {/* Overview/Description - hidden on small screens */}
                                                                {(episode.overview || tmdbData?.overview) && (
                                                                    <div className="mt-1.5 lg:mt-2 hidden md:block">
                                                                        <p className={cn(
                                                                            "text-xs lg:text-sm text-muted-foreground",
                                                                            isExpanded ? "" : "line-clamp-2"
                                                                        )}>
                                                                            {episode.overview || tmdbData?.overview}
                                                                        </p>
                                                                        {((episode.overview || tmdbData?.overview) || '').length > 150 && (
                                                                            <button
                                                                                onClick={(e) => {
                                                                                    e.stopPropagation();
                                                                                    setExpandedEpisode(isExpanded ? null : episode.id);
                                                                                }}
                                                                                className="text-[10px] lg:text-xs text-white hover:underline mt-1 flex items-center gap-0.5"
                                                                            >
                                                                                {isExpanded ? (
                                                                                    <>Show less <ChevronUp className="w-2.5 h-2.5 lg:w-3 lg:h-3" /></>
                                                                                ) : (
                                                                                    <>Show more <ChevronDown className="w-2.5 h-2.5 lg:w-3 lg:h-3" /></>
                                                                                )}
                                                                            </button>
                                                                        )}
                                                                    </div>
                                                                )}
                                                            </div>
                                                        </div>
                                                    </div>
                                                </motion.div>
                                            )
                                        })}
                                    </div>
                                )}
                            </ScrollArea>
                        </div>
                    </div>
                </div>
            </motion.div>

            {/* Player Selection Modal */}
            <PlayerModal
                open={playerModalOpen}
                onOpenChange={setPlayerModalOpen}
                onSelectPlayer={handlePlayerSelect}
                title={pendingPlayEpisode ? `S${String(pendingPlayEpisode.season_number).padStart(2, '0')}E${String(pendingPlayEpisode.episode_number).padStart(2, '0')} - ${pendingPlayEpisode.title}` : ''}
                hasTmdbId={!!show.tmdb_id}
            />

            {/* Resume Dialog */}
            {resumeDialogData && (
                <ResumeDialog
                    open={resumeDialogOpen}
                    onOpenChange={setResumeDialogOpen}
                    title={show.title}
                    mediaType={resumeDialogData.episode.media_type}
                    seasonEpisode={`S${String(resumeDialogData.episode.season_number).padStart(2, '0')}E${String(resumeDialogData.episode.episode_number).padStart(2, '0')}`}
                    currentPosition={resumeDialogData.resumeInfo.position}
                    duration={resumeDialogData.resumeInfo.duration}
                    posterUrl={posterUrl || undefined}
                    onResume={() => handleResumeChoice(true)}
                    onStartOver={() => handleResumeChoice(false)}
                />
            )}
        </>
    )
}
