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
    // Cloud storage fields
    is_cloud?: boolean;
    cloud_file_id?: string;
}

export interface Config {
    mpv_path?: string;
    vlc_path?: string;
    ffprobe_path?: string;
    ffmpeg_path?: string;
    tmdb_api_key?: string;
    // Cloud cache settings
    cloud_cache_enabled?: boolean;
    cloud_cache_dir?: string;
    cloud_cache_max_mb?: number;
    cloud_cache_expiry_hours?: number;
    // Cloud auto-scan interval in minutes
    cloud_scan_interval_minutes?: number;
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
    // Cloud streaming fields
    is_cloud?: boolean;
    access_token?: string;
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

// Get library items filtered by cloud status
export const getLibraryFiltered = async (
    type: 'movie' | 'tv',
    search: string = '',
    isCloud?: boolean
): Promise<MediaItem[]> => {
    try {
        const items = await invoke<MediaItem[]>('get_library_filtered', {
            mediaType: type,
            search: search || null,
            isCloud: isCloud ?? null
        });
        return items;
    } catch (error) {
        console.error('Failed to get filtered library:', error);
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

// Mark media as complete (100% watched)
export const markAsComplete = async (mediaId: number): Promise<{ message: string }> => {
    try {
        return await invoke('mark_as_complete', { mediaId });
    } catch (error) {
        console.error('Failed to mark as complete:', error);
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

// Cleanup response type
export interface CleanupResponse {
    success: boolean;
    removed_count: number;
    message: string;
}

// Cleanup orphaned metadata - removes entries and posters for missing files
export const cleanupMissingMetadata = async (): Promise<CleanupResponse> => {
    try {
        return await invoke<CleanupResponse>('cleanup_missing_metadata');
    } catch (error) {
        console.error('Failed to cleanup missing metadata:', error);
        throw error;
    }
};

// Repair broken file paths - finds files in media folders and updates database
export const repairFilePaths = async (): Promise<{ message: string }> => {
    try {
        return await invoke<{ message: string }>('repair_file_paths');
    } catch (error) {
        console.error('Failed to repair file paths:', error);
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

// Delete just the cloud folder for a TV series (fallback if automatic deletion fails)
export const deleteSeriesCloudFolder = async (seriesId: number): Promise<{ message: string }> => {
    try {
        const response = await invoke<{ message: string }>('delete_series_cloud_folder', { seriesId });
        return response;
    } catch (error) {
        console.error('Failed to delete cloud folder:', error);
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
        return {};
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

// Get stream info with automatic transcoding support for incompatible formats
export const getStreamUrlWithTranscode = async (id: number): Promise<StreamInfo> => {
    try {
        const info = await invoke<StreamInfo>('get_stream_info_with_transcode', { mediaId: id });
        return info;
    } catch (error) {
        console.error('Failed to get stream info with transcode:', error);
        throw error;
    }
};

// Check if a file needs transcoding for HTML5 playback
export const checkNeedsTranscode = async (filePath: string): Promise<boolean> => {
    try {
        return await invoke<boolean>('check_needs_transcode', { filePath });
    } catch (error) {
        console.error('Failed to check transcode needs:', error);
        return false;
    }
};

// Transcode response type
export interface TranscodeResponse {
    session_id: number;
    stream_url: string;
}

// Start transcoding a video file
export const startTranscodeStream = async (filePath: string, startTime?: number): Promise<TranscodeResponse> => {
    try {
        return await invoke<TranscodeResponse>('start_transcode_stream', {
            filePath,
            startTime: startTime || null
        });
    } catch (error) {
        console.error('Failed to start transcode stream:', error);
        throw error;
    }
};

// Stop a transcoding session
export const stopTranscodeStream = async (sessionId: number): Promise<void> => {
    try {
        await invoke('stop_transcode_stream', { sessionId });
    } catch (error) {
        console.error('Failed to stop transcode stream:', error);
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

// Play media with VLC (external player)
export const playWithVlc = async (id: number, resume: boolean): Promise<void> => {
    try {
        await invoke('play_with_vlc', { mediaId: id, resume });
    } catch (error) {
        console.error('Failed to play with VLC:', error);
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
export type PlayerPreference = 'mpv' | 'vlc' | 'builtin' | 'ask';

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
    // Cloud storage fields
    is_cloud?: boolean;
    cloud_file_id?: string;
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
const STREAMVAULT_COLOR = '8B5CF6'; // StreamVault brand purple

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

    const color = options?.color || STREAMVAULT_COLOR;
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

// ==================== ONBOARDING ====================

const ONBOARDING_KEY = 'streamvault_onboarding_completed';
const ONBOARDING_VERSION = '1'; // Increment to show onboarding again after major updates

// Check if user has completed onboarding
export const hasCompletedOnboarding = (): boolean => {
    try {
        const completed = localStorage.getItem(ONBOARDING_KEY);
        return completed === ONBOARDING_VERSION;
    } catch {
        return false;
    }
};

// Mark onboarding as complete
export const completeOnboarding = (): void => {
    try {
        localStorage.setItem(ONBOARDING_KEY, ONBOARDING_VERSION);
    } catch (error) {
        console.error('Failed to save onboarding state:', error);
    }
};

// Reset onboarding (for testing or after major updates)
export const resetOnboarding = (): void => {
    try {
        localStorage.removeItem(ONBOARDING_KEY);
    } catch (error) {
        console.error('Failed to reset onboarding:', error);
    }
};

// ==================== TAB VISIBILITY ====================

const TAB_VISIBILITY_KEY = 'streamvault_tab_visibility';

export interface TabVisibility {
    showLocal: boolean;
    showCloud: boolean;
}

// Get tab visibility settings
export const getTabVisibility = (): TabVisibility => {
    try {
        const stored = localStorage.getItem(TAB_VISIBILITY_KEY);
        if (stored) {
            return JSON.parse(stored);
        }
    } catch (error) {
        console.error('Failed to get tab visibility:', error);
    }
    // Default: cloud-only mode (no local tab)
    return { showLocal: false, showCloud: true };
};

// Save tab visibility settings
export const setTabVisibility = (visibility: TabVisibility): void => {
    try {
        localStorage.setItem(TAB_VISIBILITY_KEY, JSON.stringify(visibility));
    } catch (error) {
        console.error('Failed to save tab visibility:', error);
    }
};

// ==================== CLOUD CACHE ====================

export interface CloudCacheInfo {
    enabled: boolean;
    cache_dir: string | null;
    total_size_bytes: number;
    total_size_mb: number;
    file_count: number;
    max_size_mb: number;
    expiry_hours: number;
}

// Get cloud cache info and statistics
export const getCloudCacheInfo = async (): Promise<CloudCacheInfo> => {
    try {
        return await invoke<CloudCacheInfo>('get_cloud_cache_info');
    } catch (error) {
        console.error('Failed to get cloud cache info:', error);
        return {
            enabled: false,
            cache_dir: null,
            total_size_bytes: 0,
            total_size_mb: 0,
            file_count: 0,
            max_size_mb: 1024,
            expiry_hours: 24,
        };
    }
};

// Clean up expired cache files
export const cleanupCloudCache = async (): Promise<{ message: string }> => {
    try {
        return await invoke<{ message: string }>('cleanup_cloud_cache');
    } catch (error) {
        console.error('Failed to cleanup cloud cache:', error);
        throw error;
    }
};

// Clear all cloud cache
export const clearCloudCache = async (): Promise<{ message: string }> => {
    try {
        return await invoke<{ message: string }>('clear_cloud_cache');
    } catch (error) {
        console.error('Failed to clear cloud cache:', error);
        throw error;
    }
};

// ==================== GOOGLE DRIVE ====================

export interface DriveAccountInfo {
    email: string;
    display_name: string | null;
    photo_url: string | null;
    storage_used: number | null;
    storage_limit: number | null;
}

// Check if connected to Google Drive
export const isGdriveConnected = async (): Promise<boolean> => {
    try {
        return await invoke<boolean>('gdrive_is_connected');
    } catch (error) {
        console.error('Failed to check GDrive connection:', error);
        return false;
    }
};

// Get Google Drive account info including storage stats
export const getGdriveAccountInfo = async (): Promise<DriveAccountInfo | null> => {
    try {
        return await invoke<DriveAccountInfo>('gdrive_get_account_info');
    } catch (error) {
        console.error('Failed to get GDrive account info:', error);
        return null;
    }
};

// ==================== AUTO-UPDATE ====================

export interface UpdateInfo {
    available: boolean;
    current_version: string;
    latest_version: string;
    release_notes: string;
    download_url: string | null;
    published_at: string | null;
}

// Check for updates from GitHub releases
export const checkForUpdates = async (): Promise<UpdateInfo> => {
    try {
        return await invoke<UpdateInfo>('check_for_updates');
    } catch (error) {
        console.error('Failed to check for updates:', error);
        throw error;
    }
};

// Download update to temp directory (returns installer path)
export const downloadUpdate = async (url: string): Promise<string> => {
    try {
        return await invoke<string>('download_update', { url });
    } catch (error) {
        console.error('Failed to download update:', error);
        throw error;
    }
};

// Install update and restart app
export const installUpdate = async (installerPath: string): Promise<void> => {
    try {
        await invoke('install_update', { installerPath });
    } catch (error) {
        console.error('Failed to install update:', error);
        throw error;
    }
};

// Get current app version
export const getAppVersion = async (): Promise<string> => {
    try {
        return await invoke<string>('get_app_version');
    } catch (error) {
        console.error('Failed to get app version:', error);
        return '0.0.0';
    }
};

