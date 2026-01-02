import { useState, useCallback, useEffect } from 'react';
import { Search, Film, Tv, Loader2, Play, Star, TrendingUp, Clock } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/tauri';
import { EpisodeSelector } from './EpisodeSelector';
import { useToast } from './ui/use-toast';
import { saveStreamingProgress, getStreamingHistory, StreamingHistoryItem, openVideasyPlayer } from '@/services/api';
import { cn } from '@/lib/utils';

// Videasy player base URL - opens directly in browser
const VIDEASY_PLAYER_BASE = 'https://player.videasy.net';
const TMDB_IMAGE_BASE = 'https://image.tmdb.org/t/p';
const SLASSHY_COLOR = '8B5CF6'; // Slasshy brand purple

// Build Videasy player URL with all features
function buildVideasyUrl(
    mediaType: 'movie' | 'tv',
    tmdbId: number,
    season?: number,
    episode?: number
): string {
    let baseUrl: string;
    let queryString: string;

    if (mediaType === 'movie') {
        baseUrl = `${VIDEASY_PLAYER_BASE}/movie/${tmdbId}`;
        queryString = `overlay=true&color=${SLASSHY_COLOR}`;
    } else {
        const s = season || 1;
        const ep = episode || 1;
        baseUrl = `${VIDEASY_PLAYER_BASE}/tv/${tmdbId}/${s}/${ep}`;
        queryString = `nextEpisode=true&autoplayNextEpisode=true&episodeSelector=true&overlay=true&color=${SLASSHY_COLOR}`;
    }

    return `${baseUrl}?${queryString}`;
}

interface TmdbSearchResult {
    id: number;
    title?: string;
    name?: string;
    media_type: 'movie' | 'tv';
    poster_path?: string;
    backdrop_path?: string;
    overview?: string;
    release_date?: string;
    first_air_date?: string;
    vote_average?: number;
}

interface TmdbSearchResponse {
    results: TmdbSearchResult[];
    total_results: number;
}

export function StreamView() {
    const [searchQuery, setSearchQuery] = useState('');
    const [results, setResults] = useState<TmdbSearchResult[]>([]);
    const [isSearching, setIsSearching] = useState(false);
    const [selectedItem, setSelectedItem] = useState<TmdbSearchResult | null>(null);
    const [isEpisodeSelectorOpen, setIsEpisodeSelectorOpen] = useState(false);
    const [recentStreams, setRecentStreams] = useState<StreamingHistoryItem[]>([]);
    const { toast } = useToast();

    // Load recent streams on mount
    useEffect(() => {
        loadRecentStreams();
    }, []);

    const loadRecentStreams = async () => {
        try {
            const history = await getStreamingHistory(6);
            setRecentStreams(history);
        } catch (error) {
            console.error('Failed to load recent streams:', error);
        }
    };

    // Open streaming URL in the in-app Videasy player with progress sync
    const openInPlayer = useCallback(async (
        url: string,
        title: string,
        tmdbId: string,
        mediaType: 'movie' | 'tv',
        posterPath?: string,
        season?: number,
        episode?: number
    ) => {
        try {
            // Save initial entry to streaming history
            await saveStreamingProgress(
                tmdbId,
                mediaType,
                title,
                posterPath ? `${TMDB_IMAGE_BASE}/w342${posterPath}` : undefined,
                season,
                episode,
                0, // position - will be updated by player
                0  // duration - will be updated by player
            );

            // Open in the in-app Videasy webview player
            await openVideasyPlayer(
                url,
                tmdbId,
                mediaType,
                title,
                posterPath,
                season,
                episode
            );

            toast({
                title: "Opening Player",
                description: `Now streaming "${title}"`,
            });

            // Refresh recent streams
            loadRecentStreams();
        } catch (error) {
            console.error('Failed to open player:', error);
            toast({
                title: "Failed to Open Player",
                description: "Could not open the streaming player",
                variant: "destructive"
            });
        }
    }, [toast]);

    // Handle Enter key to trigger search
    const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
        if (e.key === 'Enter' && searchQuery.trim()) {
            e.preventDefault();
            searchTmdb(searchQuery);
        }
    };

    // Handle search button click
    const handleSearchClick = () => {
        if (searchQuery.trim()) {
            searchTmdb(searchQuery);
        }
    };

    const searchTmdb = async (query: string) => {
        setIsSearching(true);
        try {
            // Use the backend to search TMDB (it has the API key)
            const response = await invoke<TmdbSearchResponse>('search_tmdb', { query });
            setResults(response.results.filter((r: TmdbSearchResult) => r.media_type === 'movie' || r.media_type === 'tv'));
        } catch (error: any) {
            console.error('TMDB search failed:', error);
            const errorMessage = typeof error === 'string' ? error : (error?.message || 'Unknown error');
            toast({
                title: "Search Failed",
                description: errorMessage.includes('API key')
                    ? "TMDB API key not configured. Please add it in Settings."
                    : `Search error: ${errorMessage}`,
                variant: "destructive"
            });
        } finally {
            setIsSearching(false);
        }
    };

    const handlePlayClick = useCallback((item: TmdbSearchResult) => {
        setSelectedItem(item);

        if (item.media_type === 'tv') {
            setIsEpisodeSelectorOpen(true);
        } else {
            const title = item.title || item.name || 'Unknown';
            const url = buildVideasyUrl('movie', item.id);
            openInPlayer(url, title, item.id.toString(), 'movie', item.poster_path);
        }
    }, [openInPlayer]);

    const handleEpisodeSelect = useCallback((season: number, episode: number) => {
        if (selectedItem) {
            const title = selectedItem.name || selectedItem.title || 'Unknown';
            const displayTitle = `${title} S${String(season).padStart(2, '0')}E${String(episode).padStart(2, '0')}`;
            const url = buildVideasyUrl('tv', selectedItem.id, season, episode);
            openInPlayer(url, displayTitle, selectedItem.id.toString(), 'tv', selectedItem.poster_path, season, episode);
        }
        setIsEpisodeSelectorOpen(false);
        setSelectedItem(null);
    }, [selectedItem, openInPlayer]);

    const handleEpisodeSelectorClose = useCallback(() => {
        setIsEpisodeSelectorOpen(false);
        setSelectedItem(null);
    }, []);

    const handleRecentClick = (item: StreamingHistoryItem) => {
        const url = item.media_type === 'movie'
            ? buildVideasyUrl('movie', parseInt(item.tmdb_id))
            : buildVideasyUrl('tv', parseInt(item.tmdb_id), item.season || 1, item.episode || 1);

        const displayTitle = item.media_type === 'tv' && item.season && item.episode
            ? `${item.title} S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`
            : item.title;

        // Extract the poster path without the full URL prefix
        const posterPath = item.poster_path?.includes('/t/p/')
            ? item.poster_path.split('/t/p/')[1]?.replace('w342', '').replace('w300', '')
            : undefined;

        openInPlayer(
            url,
            displayTitle,
            item.tmdb_id,
            item.media_type as 'movie' | 'tv',
            posterPath,
            item.season || undefined,
            item.episode || undefined
        );
    };

    const getTitle = (item: TmdbSearchResult) => item.title || item.name || 'Unknown';
    const getYear = (item: TmdbSearchResult) => {
        const date = item.release_date || item.first_air_date;
        return date ? new Date(date).getFullYear() : null;
    };

    return (
        <>
            <div className="h-full flex flex-col gap-8">
                {/* Search Section */}
                <div className="max-w-3xl mx-auto w-full pt-4">
                    <div className="text-center mb-6">
                        <div className="inline-flex items-center justify-center p-3 rounded-2xl bg-primary/10 mb-4">
                            <Play className="w-8 h-8 text-primary fill-primary" />
                        </div>
                        <h2 className="text-2xl font-bold text-foreground mb-2">Stream Online</h2>
                        <p className="text-sm text-muted-foreground">Search and stream movies & TV shows instantly</p>
                    </div>

                    {/* Search Bar */}
                    <div className="flex gap-3">
                        <div className="search-container flex-1">
                            <Search className="search-icon" />
                            <input
                                type="text"
                                placeholder="Search for movies or TV shows..."
                                value={searchQuery}
                                onChange={(e) => setSearchQuery(e.target.value)}
                                onKeyDown={handleKeyDown}
                                className="search-input"
                                autoFocus
                            />
                            {isSearching && (
                                <Loader2 className="absolute right-4 top-1/2 -translate-y-1/2 w-5 h-5 animate-spin text-primary" />
                            )}
                        </div>
                        <button
                            onClick={handleSearchClick}
                            disabled={!searchQuery.trim() || isSearching}
                            className="btn-primary px-6 flex items-center gap-2"
                        >
                            <Search className="w-4 h-4" />
                            <span className="hidden sm:inline">Search</span>
                        </button>
                    </div>
                </div>

                {/* Content Area */}
                <div className="flex-1 overflow-auto">
                    {/* Initial State - Show Recent or Placeholder */}
                    {!searchQuery && results.length === 0 && (
                        <AnimatePresence mode="wait">
                            {recentStreams.length > 0 ? (
                                <motion.section
                                    initial={{ opacity: 0 }}
                                    animate={{ opacity: 1 }}
                                    exit={{ opacity: 0 }}
                                >
                                    <div className="section-header">
                                        <h3 className="section-title">
                                            <Clock className="w-5 h-5 text-primary" />
                                            Recently Streamed
                                        </h3>
                                    </div>
                                    <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-4">
                                        {recentStreams.map((item, index) => (
                                            <motion.div
                                                key={item.id}
                                                initial={{ opacity: 0, y: 20 }}
                                                animate={{ opacity: 1, y: 0 }}
                                                transition={{ delay: index * 0.05 }}
                                                onClick={() => handleRecentClick(item)}
                                                className="media-card group"
                                            >
                                                <div className="aspect-[2/3] relative overflow-hidden">
                                                    {item.poster_path ? (
                                                        <img
                                                            src={item.poster_path}
                                                            alt={item.title}
                                                            className="media-card-poster"
                                                            loading="lazy"
                                                        />
                                                    ) : (
                                                        <div className="w-full h-full flex items-center justify-center bg-muted">
                                                            {item.media_type === 'movie' ? (
                                                                <Film className="w-10 h-10 text-muted-foreground" />
                                                            ) : (
                                                                <Tv className="w-10 h-10 text-muted-foreground" />
                                                            )}
                                                        </div>
                                                    )}
                                                    <div className="media-card-overlay flex items-center justify-center">
                                                        <div className="w-12 h-12 rounded-full bg-primary flex items-center justify-center">
                                                            <Play className="w-5 h-5 text-white fill-white ml-0.5" />
                                                        </div>
                                                    </div>
                                                    {/* Badge */}
                                                    <div className={cn(
                                                        "absolute top-2 right-2 p-1 rounded-md",
                                                        item.media_type === 'movie'
                                                            ? "bg-blue-500"
                                                            : "bg-primary"
                                                    )}>
                                                        {item.media_type === 'movie' ? (
                                                            <Film className="w-3 h-3 text-white" />
                                                        ) : (
                                                            <Tv className="w-3 h-3 text-white" />
                                                        )}
                                                    </div>
                                                </div>
                                                <div className="p-2">
                                                    <h4 className="font-medium text-sm text-foreground line-clamp-1">{item.title}</h4>
                                                    {item.media_type === 'tv' && item.season && item.episode && (
                                                        <p className="text-xs text-muted-foreground">
                                                            S{item.season} E{item.episode}
                                                        </p>
                                                    )}
                                                </div>
                                            </motion.div>
                                        ))}
                                    </div>
                                </motion.section>
                            ) : (
                                <motion.div
                                    initial={{ opacity: 0 }}
                                    animate={{ opacity: 1 }}
                                    exit={{ opacity: 0 }}
                                    className="empty-state"
                                >
                                    <div className="empty-state-icon">
                                        <TrendingUp className="w-12 h-12 text-muted-foreground" />
                                    </div>
                                    <h3 className="empty-state-title">Search for Content</h3>
                                    <p className="empty-state-description">
                                        Enter a movie or TV show title above to search and start streaming
                                    </p>
                                </motion.div>
                            )}
                        </AnimatePresence>
                    )}

                    {/* No Results */}
                    {searchQuery && results.length === 0 && !isSearching && (
                        <div className="empty-state">
                            <div className="empty-state-icon">
                                <Search className="w-12 h-12 text-muted-foreground" />
                            </div>
                            <h3 className="empty-state-title">No results found</h3>
                            <p className="empty-state-description">
                                No results for "{searchQuery}". Try a different search term.
                            </p>
                        </div>
                    )}

                    {/* Search Results */}
                    <AnimatePresence mode="popLayout">
                        {results.length > 0 && (
                            <motion.div
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                            >
                                <div className="section-header">
                                    <h3 className="section-title">
                                        Results ({results.length})
                                    </h3>
                                </div>
                                <div className="grid-media">
                                    {results.map((item, index) => (
                                        <motion.div
                                            key={item.id}
                                            initial={{ opacity: 0, y: 20 }}
                                            animate={{ opacity: 1, y: 0 }}
                                            exit={{ opacity: 0, scale: 0.95 }}
                                            transition={{ delay: index * 0.03 }}
                                            className="media-card group"
                                            onClick={() => handlePlayClick(item)}
                                        >
                                            {/* Poster */}
                                            <div className="aspect-[2/3] relative overflow-hidden">
                                                {item.poster_path ? (
                                                    <img
                                                        src={`${TMDB_IMAGE_BASE}/w342${item.poster_path}`}
                                                        alt={getTitle(item)}
                                                        className="media-card-poster"
                                                        loading="lazy"
                                                    />
                                                ) : (
                                                    <div className="w-full h-full flex items-center justify-center bg-muted">
                                                        {item.media_type === 'movie' ? (
                                                            <Film className="w-12 h-12 text-muted-foreground" />
                                                        ) : (
                                                            <Tv className="w-12 h-12 text-muted-foreground" />
                                                        )}
                                                    </div>
                                                )}

                                                {/* Overlay */}
                                                <div className="media-card-overlay flex flex-col justify-end p-3">
                                                    {/* Rating & Type */}
                                                    <div className="flex items-center gap-2 mb-2">
                                                        <span className={cn(
                                                            "px-2 py-0.5 rounded text-xs font-bold uppercase",
                                                            item.media_type === 'movie'
                                                                ? 'bg-blue-500 text-white'
                                                                : 'bg-primary text-white'
                                                        )}>
                                                            {item.media_type}
                                                        </span>
                                                        {item.vote_average && item.vote_average > 0 && (
                                                            <div className="flex items-center gap-1 text-xs text-yellow-400">
                                                                <Star className="w-3 h-3 fill-yellow-400" />
                                                                {item.vote_average.toFixed(1)}
                                                            </div>
                                                        )}
                                                    </div>
                                                    <h4 className="text-sm font-semibold text-white line-clamp-2">{getTitle(item)}</h4>
                                                    {getYear(item) && (
                                                        <p className="text-xs text-white/70">{getYear(item)}</p>
                                                    )}
                                                </div>

                                                {/* Play button */}
                                                <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity duration-300">
                                                    <div className="relative">
                                                        <div className="absolute inset-0 rounded-full bg-primary/30 blur-xl scale-150" />
                                                        <div className="relative w-14 h-14 rounded-full bg-primary flex items-center justify-center shadow-elevation-2">
                                                            <Play className="w-6 h-6 text-white fill-white ml-0.5" />
                                                        </div>
                                                    </div>
                                                </div>

                                                {/* Type badge (always visible) */}
                                                <div className={cn(
                                                    "absolute top-2 right-2 p-1.5 rounded-lg",
                                                    item.media_type === 'movie'
                                                        ? 'bg-blue-500'
                                                        : 'bg-primary'
                                                )}>
                                                    {item.media_type === 'movie' ? (
                                                        <Film className="w-3 h-3 text-white" />
                                                    ) : (
                                                        <Tv className="w-3 h-3 text-white" />
                                                    )}
                                                </div>
                                            </div>

                                            {/* Info below card */}
                                            <div className="p-2">
                                                <h4 className="font-medium text-sm text-foreground line-clamp-1">{getTitle(item)}</h4>
                                                <p className="text-xs text-muted-foreground">{getYear(item) || 'Unknown year'}</p>
                                            </div>
                                        </motion.div>
                                    ))}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            </div>

            {/* Episode Selector for TV Shows */}
            {isEpisodeSelectorOpen && selectedItem && selectedItem.media_type === 'tv' && (
                <EpisodeSelector
                    tvId={selectedItem.id}
                    title={getTitle(selectedItem)}
                    posterPath={selectedItem.poster_path}
                    backdropPath={selectedItem.backdrop_path}
                    onSelect={handleEpisodeSelect}
                    onClose={handleEpisodeSelectorClose}
                />
            )}
        </>
    );
}
