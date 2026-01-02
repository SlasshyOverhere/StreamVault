import { invoke } from '@tauri-apps/api/tauri';


export interface MediaItem {
    id: number;
    title: string;
    year?: number;
    overview?: string;
    poster_path?: string;
    file_path?: string;
    media_type: 'movie' | 'tvshow' | 'tvepisode';
    duration_seconds?: number;
    resume_position_seconds?: number;
    last_watched?: string;
    season_number?: number;
    episode_number?: number;
    progress_percent?: number;
    parent_id?: number;
    tmdb_id?: string;
    episode_title?: string;
    still_path?: string;
}

export interface Config {
    mpv_path?: string;
    ffprobe_path?: string;
    media_folders: string[];
    tmdb_api_key?: string;
}

export interface ResumeInfo {
    has_progress: boolean;
    position: number;
    duration: number;
    time_str: string;
    progress_percent: number;
}

export interface StreamInfo {
    stream_url: string;
    file_path: string;
    title: string;
    poster?: string;
    duration_seconds?: number;
    resume_position_seconds?: number;
}

// Get library items (movies or TV shows)
export const getLibrary = async (type: 'movie' | 'tv', search: string = ''): Promise<MediaItem[]> => {
    try {
        const items = await invoke<MediaItem[]>('get_library', {
            mediaType: type,
            search: search || null
        });
        return items;
    } catch (error) {
        console.error('Failed to get library:', error);
        return [];
    }
};

// Get watch history
export const getWatchHistory = async (): Promise<MediaItem[]> => {
    try {
        const items = await invoke<MediaItem[]>('get_watch_history', { limit: 50 });
        return items;
    } catch (error) {
        console.error('Failed to get watch history:', error);
        return [];
    }
};

// Remove a single item from watch history
export const removeFromWatchHistory = async (id: number): Promise<void> => {
    try {
        await invoke('remove_from_watch_history', { mediaId: id });
    } catch (error) {
        console.error('Failed to remove from watch history:', error);
        throw error;
    }
};

// Clear all watch history
export const clearAllWatchHistory = async (): Promise<void> => {
    try {
        await invoke('clear_all_watch_history');
    } catch (error) {
        console.error('Failed to clear watch history:', error);
        throw error;
    }
};

// ==================== STREAMING HISTORY ====================

// Streaming history item for online content (Videasy, etc.)
export interface StreamingHistoryItem {
    id: number;
    tmdb_id: string;
    media_type: 'movie' | 'tv';
    title: string;
    poster_path?: string;
    season?: number;
    episode?: number;
    resume_position_seconds: number;
    duration_seconds: number;
    progress_percent: number;
    last_watched: string;
}

// Save streaming progress
export const saveStreamingProgress = async (
    tmdbId: string,
    mediaType: 'movie' | 'tv',
    title: string,
    posterPath?: string,
    season?: number,
    episode?: number,
    position: number = 0,
    duration: number = 0
): Promise<void> => {
    try {
        await invoke('save_streaming_progress', {
            tmdbId,
            mediaType,
            title,
            posterPath: posterPath || null,
            season: season || null,
            episode: episode || null,
            position,
            duration
        });
    } catch (error) {
        console.error('Failed to save streaming progress:', error);
        throw error;
    }
};

// Get streaming history
export const getStreamingHistory = async (limit: number = 50): Promise<StreamingHistoryItem[]> => {
    try {
        const items = await invoke<StreamingHistoryItem[]>('get_streaming_history', { limit });
        return items;
    } catch (error) {
        console.error('Failed to get streaming history:', error);
        return [];
    }
};

// Get streaming resume info for a specific content
export const getStreamingResumeInfo = async (
    tmdbId: string,
    mediaType: 'movie' | 'tv',
    season?: number,
    episode?: number
): Promise<StreamingHistoryItem | null> => {
    try {
        const info = await invoke<StreamingHistoryItem | null>('get_streaming_resume_info', {
            tmdbId,
            mediaType,
            season: season || null,
            episode: episode || null
        });
        return info;
    } catch (error) {
        console.error('Failed to get streaming resume info:', error);
        return null;
    }
};

// Remove a single item from streaming history
export const removeFromStreamingHistory = async (id: number): Promise<void> => {
    try {
        await invoke('remove_from_streaming_history', { id });
    } catch (error) {
        console.error('Failed to remove from streaming history:', error);
        throw error;
    }
};

// Clear all streaming history
export const clearAllStreamingHistory = async (): Promise<void> => {
    try {
        await invoke('clear_all_streaming_history');
    } catch (error) {
        console.error('Failed to clear streaming history:', error);
        throw error;
    }
};

// Clear all app data (reset to fresh state)
export const clearAllAppData = async (): Promise<void> => {
    try {
        // Clear localStorage
        localStorage.clear();
        // Clear database and image cache via backend
        await invoke('clear_all_app_data');
    } catch (error) {
        console.error('Failed to clear app data:', error);
        throw error;
    }
};

// Delete response type
export interface DeleteResponse {
    success: boolean;
    deleted_count: number;
    failed_count: number;
    message: string;
}

// Episode info for delete selection
export interface EpisodeDeleteInfo {
    id: number;
    title: string;
    season_number?: number;
    episode_number?: number;
    file_path?: string;
}

// Delete media files permanently from disk
export const deleteMediaFiles = async (mediaIds: number[]): Promise<DeleteResponse> => {
    try {
        const response = await invoke<DeleteResponse>('delete_media_files', { mediaIds });
        return response;
    } catch (error) {
        console.error('Failed to delete media files:', error);
        throw error;
    }
};

// Get episodes for delete selection modal
export const getEpisodesForDelete = async (seriesId: number): Promise<EpisodeDeleteInfo[]> => {
    try {
        const episodes = await invoke<EpisodeDeleteInfo[]>('get_episodes_for_delete', { seriesId });
        return episodes;
    } catch (error) {
        console.error('Failed to get episodes for delete:', error);
        return [];
    }
};

// Delete a TV series and optionally its files
export const deleteSeries = async (seriesId: number, deleteFiles: boolean): Promise<DeleteResponse> => {
    try {
        const response = await invoke<DeleteResponse>('delete_series', { seriesId, deleteFiles });
        return response;
    } catch (error) {
        console.error('Failed to delete series:', error);
        throw error;
    }
};


// Get episodes for a TV show
export const getEpisodes = async (seriesId: number): Promise<MediaItem[]> => {
    try {
        const items = await invoke<MediaItem[]>('get_episodes', { seriesId });
        return items;
    } catch (error) {
        console.error('Failed to get episodes:', error);
        return [];
    }
};

// Get configuration
export const getConfig = async (): Promise<Config> => {
    try {
        const config = await invoke<Config>('get_config');
        return config;
    } catch (error) {
        console.error('Failed to get config:', error);
        return { media_folders: [] };
    }
};

// Save configuration
export const saveConfig = async (config: Config): Promise<void> => {
    try {
        await invoke('save_config', { newConfig: config });
    } catch (error) {
        console.error('Failed to save config:', error);
        throw error;
    }
};

// Scan library
export const scanLibrary = async (): Promise<void> => {
    try {
        await invoke('scan_library');
    } catch (error) {
        console.error('Failed to start scan:', error);
        throw error;
    }
};

// Get resume info for a media item
export const getResumeInfo = async (id: number): Promise<ResumeInfo> => {
    try {
        const info = await invoke<ResumeInfo>('get_resume_info', { mediaId: id });
        return info;
    } catch (error) {
        console.error('Failed to get resume info:', error);
        return {
            has_progress: false,
            position: 0,
            duration: 0,
            time_str: '00:00:00',
            progress_percent: 0
        };
    }
};

// Get media info by ID
export const getMediaInfo = async (id: number): Promise<MediaItem> => {
    try {
        const media = await invoke<MediaItem>('get_media_info', { mediaId: id });
        return media;
    } catch (error) {
        console.error('Failed to get media info:', error);
        throw error;
    }
};

// Get stream info for built-in player
export const getStreamUrl = async (id: number): Promise<StreamInfo> => {
    try {
        const info = await invoke<StreamInfo>('get_stream_info', { mediaId: id });
        return info;
    } catch (error) {
        console.error('Failed to get stream info:', error);
        throw error;
    }
};

// Update watch progress
export const updateWatchProgress = async (id: number, currentTime: number, duration: number): Promise<void> => {
    try {
        await invoke('update_progress', {
            mediaId: id,
            currentTime,
            duration
        });
    } catch (error) {
        console.warn('Failed to update progress:', error);
    }
};

// Clear progress for a media item
export const clearProgress = async (id: number): Promise<void> => {
    try {
        await invoke('clear_progress', { mediaId: id });
    } catch (error) {
        console.error('Failed to clear progress:', error);
        throw error;
    }
};

// Play media with MPV (external player)
export const playMedia = async (id: number, resume: boolean): Promise<void> => {
    try {
        await invoke('play_with_mpv', { mediaId: id, resume });
    } catch (error) {
        console.error('Failed to play with MPV:', error);
        throw error;
    }
};

// Fix match - update metadata from TMDB
export const fixMatch = async (id: number, tmdbId: string, type: 'movie' | 'tv'): Promise<void> => {
    try {
        await invoke('fix_match', {
            mediaId: id,
            tmdbId,
            mediaType: type
        });
    } catch (error) {
        console.error('Failed to fix match:', error);
        throw error;
    }
};

// Get cached image URL (converts local path to asset protocol URL)
export const getCachedImageUrl = async (imageName: string): Promise<string | null> => {
    try {
        const filePath = await invoke<string>('get_cached_image_path', { imageName });
        // Use Tauri's convertFileSrc for proper path conversion
        const { convertFileSrc } = await import('@tauri-apps/api/tauri');
        return convertFileSrc(filePath);
    } catch (error) {
        console.warn('[Image] Failed to get cached image:', imageName, error);
        return null;
    }
};

// Helper to get poster URL from media item
export const getPosterUrl = (item: MediaItem): string | null => {
    if (!item.poster_path) return null;

    // If it's already a full URL, return as-is
    if (item.poster_path.startsWith('http') || item.poster_path.startsWith('asset://')) {
        return item.poster_path;
    }

    // For now, return null - components should call getCachedImageUrl() themselves
    // since it's async and this function is synchronous
    return null;
};

// Player preferences
export type PlayerPreference = 'mpv' | 'builtin' | 'ask';

export const getPlayerPreference = (): PlayerPreference => {
    return (localStorage.getItem('playerPreference') as PlayerPreference) || 'ask';
};

export const setPlayerPreference = (preference: PlayerPreference): void => {
    localStorage.setItem('playerPreference', preference);
};

// MPV Status types
export interface MpvStatus {
    is_playing: boolean;
    media_id: number;
    title?: string;
    position?: number;
    duration?: number;
    paused?: boolean;
}

export interface MpvSession {
    media_id: number;
    pid: number;
    title: string;
    start_time: number;
}

// Get MPV playback status for a media item
export const getMpvStatus = async (mediaId: number): Promise<MpvStatus> => {
    try {
        const status = await invoke<MpvStatus>('get_mpv_status', { mediaId });
        return status;
    } catch (error) {
        console.error('Failed to get MPV status:', error);
        return { is_playing: false, media_id: mediaId };
    }
};

// Get all active MPV sessions
export const getActiveMpvSessions = async (): Promise<MpvSession[]> => {
    try {
        const sessions = await invoke<MpvSession[]>('get_active_mpv_sessions');
        return sessions;
    } catch (error) {
        console.error('Failed to get active MPV sessions:', error);
        return [];
    }
};

// ==================== TMDB EPISODE METADATA ====================

// Episode info from TMDB with rich metadata
export interface TmdbEpisodeInfo {
    episode_number: number;
    name: string;
    overview?: string;
    still_path?: string;
    air_date?: string;
    runtime?: number;
    vote_average?: number;
}

// Season details with episodes from TMDB
export interface TmdbSeasonDetails {
    season_number: number;
    name: string;
    episodes: TmdbEpisodeInfo[];
}

// TV show details with seasons from TMDB
export interface TmdbShowDetails {
    id: number;
    name: string;
    poster_path?: string;
    backdrop_path?: string;
    overview?: string;
    number_of_seasons: number;
    seasons: {
        season_number: number;
        name: string;
        episode_count: number;
        overview?: string;
        poster_path?: string;
        air_date?: string;
    }[];
}

// Get TV show details including seasons from TMDB
export const getTvDetails = async (tvId: number): Promise<TmdbShowDetails | null> => {
    try {
        const details = await invoke<TmdbShowDetails>('get_tv_details', { tvId });
        return details;
    } catch (error) {
        console.error('Failed to get TV details:', error);
        return null;
    }
};

// Get episodes for a specific season from TMDB (with full metadata)
export const getTvSeasonEpisodes = async (tvId: number, seasonNumber: number): Promise<TmdbSeasonDetails | null> => {
    try {
        const seasonDetails = await invoke<TmdbSeasonDetails>('get_tv_season_episodes', { tvId, seasonNumber });
        return seasonDetails;
    } catch (error) {
        console.error('Failed to get season episodes:', error);
        return null;
    }
};

// Force refresh episode metadata for a TV series (re-downloads images)
export const refreshSeriesMetadata = async (tvId: number, seriesTitle: string): Promise<string> => {
    try {
        const result = await invoke<string>('refresh_series_metadata', { tvId, seriesTitle });
        return result;
    } catch (error) {
        console.error('Failed to refresh series metadata:', error);
        throw error;
    }
};

// TMDB image URL helper
const TMDB_IMAGE_BASE = 'https://image.tmdb.org/t/p';

export const getTmdbImageUrl = (path: string | undefined, size: 'w92' | 'w185' | 'w300' | 'w500' | 'original' = 'w300'): string | null => {
    if (!path) return null;
    return `${TMDB_IMAGE_BASE}/${size}${path}`;
};

// Videasy streaming URL helpers
const VIDEASY_BASE_URL = 'https://player.videasy.net';
const SLASSHY_COLOR = '8B5CF6'; // Slasshy brand purple

/**
 * Get the Videasy player URL for a media item with all enhanced features
 * IMPORTANT: Videasy requires parameters in a specific order to work correctly
 * - TV shows: nextEpisode, autoplayNextEpisode, episodeSelector, overlay, color
 * - Movies: overlay, color
 * @param tmdbId TMDB ID of the media
 * @param mediaType Type of media (movie or tv)
 * @param season Season number (for TV episodes)
 * @param episode Episode number (for TV episodes)
 * @param options Optional customization options
 * @returns Full Videasy player URL or null if tmdbId is not provided
 */
export function getVideasyUrl(
    tmdbId: string | undefined,
    mediaType: 'movie' | 'tv',
    season?: number,
    episode?: number,
    options?: {
        color?: string;
        progress?: number;
        overlay?: boolean;
        nextEpisode?: boolean;
        autoplayNextEpisode?: boolean;
        episodeSelector?: boolean;
    }
): string | null {
    if (!tmdbId) return null;

    const color = options?.color || SLASSHY_COLOR;
    let baseUrl: string;
    let queryString: string;

    if (mediaType === 'movie') {
        baseUrl = `${VIDEASY_BASE_URL}/movie/${tmdbId}`;
        // Movie params in order: overlay, color
        queryString = `overlay=true&color=${color}`;
    } else if (mediaType === 'tv' && season !== undefined && episode !== undefined) {
        baseUrl = `${VIDEASY_BASE_URL}/tv/${tmdbId}/${season}/${episode}`;
        // TV params in required order: nextEpisode, autoplayNextEpisode, episodeSelector, overlay, color
        queryString = `nextEpisode=true&autoplayNextEpisode=true&episodeSelector=true&overlay=true&color=${color}`;
    } else {
        return null;
    }

    // Add optional progress (resume position in seconds)
    if (options?.progress !== undefined && options.progress > 0) {
        queryString += `&progress=${options.progress}`;
    }

    return `${baseUrl}?${queryString}`;
}

/**
 * Helper to get Videasy URL from a MediaItem object
 */
export function getVideasyUrlForItem(item: MediaItem, parentTmdbId?: string): string | null {
    const tmdbId = item.tmdb_id || parentTmdbId;
    if (!tmdbId) return null;

    if (item.media_type === 'movie') {
        return getVideasyUrl(tmdbId, 'movie');
    } else if (item.media_type === 'tvepisode' && item.season_number !== undefined && item.episode_number !== undefined) {
        return getVideasyUrl(tmdbId, 'tv', item.season_number, item.episode_number);
    }

    return null;
}

// ==================== VIDEASY WEBVIEW PLAYER ====================

// Open Videasy in an in-app webview window with progress sync
export const openVideasyPlayer = async (
    url: string,
    tmdbId: string,
    mediaType: 'movie' | 'tv',
    title: string,
    posterPath?: string,
    season?: number,
    episode?: number
): Promise<void> => {
    try {
        await invoke('open_videasy_player', {
            url,
            tmdbId,
            mediaType,
            title,
            posterPath: posterPath || null,
            season: season || null,
            episode: episode || null,
        });
    } catch (error) {
        console.error('Failed to open Videasy player:', error);
        throw error;
    }
};

