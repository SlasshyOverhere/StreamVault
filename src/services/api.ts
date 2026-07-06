import { invoke, convertFileSrc } from "@tauri-apps/api/tauri";

// Cache for image URLs to prevent repeated IPC calls
const imageUrlCache = new Map<string, Promise<string | null>>();

export interface MediaItem {
  id: number;
  title: string;
  year?: number;
  overview?: string;
  cast_names?: string;
  director?: string;
  poster_path?: string;
  file_path?: string;
  media_type: "movie" | "tvshow" | "tvepisode";
  duration_seconds?: number;
  resume_position_seconds?: number;
  last_watched?: string;
  season_number?: number;
  episode_number?: number;
  progress_percent?: number;
  parent_id?: number;
  tmdb_id?: string;
  imdb_id?: string;
  episode_title?: string;
  still_path?: string;
  // Cloud storage fields
  is_cloud?: boolean;
  cloud_file_id?: string;
  cloud_folder_id?: string;
  parent_zip_id?: string;
  zip_entry_path?: string;
  zip_local_header_offset?: number;
  zip_data_start_offset?: number;
  zip_compressed_size?: number;
  zip_uncompressed_size?: number;
  zip_crc32?: string;
  zip_compression_method?: number;
  file_size_bytes?: number;
  ddl_source_id?: string;
  // Frontend-only history presentation fields
  history_group_count?: number;
  history_group_ids?: number[];
  history_group_latest_label?: string;
}

export interface MediaTechnicalDetails {
  width?: number;
  height?: number;
  fps?: number;
  resolutionLabel?: string;
  container?: string;
  extension?: string;
  videoCodec?: string;
  fileSizeBytes?: number;
  sampleFromEpisode?: boolean;
}

export interface WatchHistoryEvent {
  event_id: string;
  media_id?: number | null;
  parent_media_id?: number | null;
  title: string;
  parent_title?: string | null;
  media_type: "movie" | "tvshow" | "tvepisode";
  year?: number;
  overview?: string;
  poster_path?: string;
  still_path?: string;
  tmdb_id?: string;
  parent_tmdb_id?: string;
  episode_title?: string;
  season_number?: number;
  episode_number?: number;
  is_cloud: boolean;
  progress_percent: number;
  resume_position_seconds: number;
  duration_seconds: number;
  completed: boolean;
  started_at: string;
  ended_at: string;
  updated_at: string;
}

export interface WatchHistorySyncStatus {
  synced: boolean;
  merged_remote_events: number;
  uploaded_events: number;
  skipped_reason?: string | null;
}

export interface Config {
  mpv_path?: string;
  vlc_path?: string;
  ffprobe_path?: string;
  ffmpeg_path?: string;
  tmdb_api_key?: string;
  omdb_api_key?: string;
  // Cloud cache settings
  cloud_cache_enabled?: boolean;
  cloud_cache_dir?: string;
  cloud_cache_max_mb?: number;
  cloud_cache_expiry_hours?: number;
  // Cloud auto-scan interval in minutes
  cloud_scan_interval_minutes?: number;
  zip_indexing_enabled?: boolean;
  zip_cache_dir?: string;
  zip_cache_max_gb?: number;
  zip_cache_expiry_days?: number;
  notifications_enabled?: boolean;
  // Override the OAuth backend URL for self-hosting
  dev_backend_url?: string;
  // Player mode: "native" (libmpv embedded) or "external" (mpv.exe spawned)
  player_mode?: "native" | "external";
  // User-configured addon URL for External tab streaming
  addon_url?: string;
  // Multiple addon sources for External tab
  addon_sources?: AddonSource[];
  // Watch Together Cloudflare relay URL (wss://...)
  together_relay_url?: string;
  // Cloudflare API token for one-click relay deploy
  together_cf_token?: string;
  // Cloudflare account ID for one-click relay deploy
  together_cf_account_id?: string;
}

export interface AddonSource {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  is_default: boolean;
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

export interface AudioTrackOption {
  stream_index: number;
  track_id?: number | null;
  language_code?: string | null;
  label: string;
  detail?: string | null;
  mpv_value?: string | null;
}

export type SubtitleTrackOption = AudioTrackOption;

export interface MpvAudioTracksDetectedPayload {
  media_id: number;
  series_id?: number | null;
  season_number?: number | null;
  tracks: AudioTrackOption[];
}

export interface MpvSubtitleTracksDetectedPayload {
  media_id: number;
  series_id?: number | null;
  season_number?: number | null;
  tracks: SubtitleTrackOption[];
}

export interface LibraryStats {
  movies: number;
  shows: number;
  episodes: number;
}

// ==================== ANALYTICS TYPES ====================

export interface AnalyticsOverview {
  total_watch_time_seconds: number;
  movies_completed: number;
  episodes_completed: number;
  total_completion_rate: number;
  current_streak_days: number;
  total_events: number;
}

export interface HeatmapDay {
  date: string;
  watch_seconds: number;
  event_count: number;
}

export interface DailyWatchPoint {
  date: string;
  watch_seconds: number;
  movie_count: number;
  episode_count: number;
}

export interface ContentTypeBreakdown {
  content_type: string;
  count: number;
  total_seconds: number;
}

export interface SourceBreakdown {
  source: string;
  count: number;
  total_seconds: number;
}

export interface TopWatchedItem {
  title: string;
  parent_title?: string | null;
  media_type: string;
  watch_count: number;
  total_seconds: number;
  poster_path?: string | null;
  tmdb_id?: string | null;
}

export interface HourDistribution {
  hour: number;
  event_count: number;
  total_seconds: number;
}

export interface DayOfWeekDistribution {
  day_of_week: number;
  event_count: number;
  total_seconds: number;
}

export interface CompletionFunnel {
  started: number;
  in_progress_25: number;
  mostly_done_75: number;
  completed: number;
}

export interface AnalyticsData {
  overview: AnalyticsOverview;
  heatmap: HeatmapDay[];
  daily_trend: DailyWatchPoint[];
  content_breakdown: ContentTypeBreakdown[];
  source_breakdown: SourceBreakdown[];
  top_watched: TopWatchedItem[];
  hour_distribution: HourDistribution[];
  day_distribution: DayOfWeekDistribution[];
  completion_funnel: CompletionFunnel;
  library_stats: LibraryStats;
  recent_events: WatchHistoryEvent[];
}

export interface DownloadJob {
  id: string;
  mediaId: number;
  title: string;
  fileName: string;
  targetPath: string;
  status: "queued" | "preparing" | "downloading" | "completed" | "failed" | "cancelled";
  progress: number;
  downloadedBytes: number;
  totalBytes: number;
  speedBytesPerSecond?: number | null;
  createdAt: string;
  updatedAt: string;
  error?: string | null;
  sourceKind: string;
  sourceExists: boolean;
  targetExists: boolean;
}

// TMDB search result
export interface TmdbSearchResult {
  id: number;
  title?: string;
  name?: string;
  media_type: "movie" | "tv";
  poster_path?: string;
  backdrop_path?: string;
  overview?: string;
  release_date?: string;
  first_air_date?: string;
  vote_average?: number;
}

export interface TmdbSearchResponse {
  results: TmdbSearchResult[];
  total_results: number;
}

export interface TmdbTrendingItem {
  id: number;
  title: string;
  media_type: "movie" | "tv";
}

export interface TmdbTrendingResponse {
  results: TmdbTrendingItem[];
}

// Get library items (movies or TV shows)
export const getLibrary = async (
  type: "movie" | "tv",
  search: string = "",
): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_library", {
      mediaType: type,
      search: search || null,
    });
    return items;
  } catch (error) {
    console.error("Failed to get library:", error);
    return [];
  }
};

// Get library items filtered by cloud status
export const getLibraryFiltered = async (
  type: "movie" | "tv",
  search: string = "",
  isCloud?: boolean,
): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_library_filtered", {
      mediaType: type,
      search: search || null,
      isCloud: isCloud ?? null,
    });
    return items;
  } catch (error) {
    console.error("Failed to get filtered library:", error);
    return [];
  }
};

// Combined search across all libraries (local + cloud, movies + TV) in one IPC call
export const searchAllLibraries = async (
  search: string,
): Promise<MediaItem[]> => {
  try {
    return await invoke<MediaItem[]>("search_all_libraries", { search });
  } catch (error) {
    console.error("Failed to search all libraries:", error);
    return [];
  }
};

// Get DDL library items
export const getDdlMedia = async (
  type: "movie" | "tv",
  search: string = "",
): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_ddl_media", {
      mediaType: type,
      search: search || null,
    });
    return items;
  } catch (error) {
    console.error("Failed to get DDL media:", error);
    return [];
  }
};

// Get recently added items
export const getRecentlyAdded = async (
  limit: number = 10,
  isCloud?: boolean
): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_recently_added", {
      limit,
      isCloud: isCloud ?? null,
    });
    return items;
  } catch (error) {
    console.error("Failed to get recently added items:", error);
    return [];
  }
};

export const getLibraryStats = async (
  isCloud?: boolean,
): Promise<LibraryStats> => {
  try {
    return await invoke<LibraryStats>("get_library_stats", {
      isCloud: isCloud ?? null,
    });
  } catch (error) {
    console.error("Failed to get library stats:", error);
    return { movies: 0, shows: 0, episodes: 0 };
  }
};

// Get watch history
export const getWatchHistory = async (): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_watch_history", { limit: 50 });
    return items;
  } catch (error) {
    console.error("Failed to get watch history:", error);
    return [];
  }
};

export const getWatchHistoryEvents = async (): Promise<WatchHistoryEvent[]> => {
  try {
    return await invoke<WatchHistoryEvent[]>("get_watch_history_events", {
      limit: 200,
    });
  } catch (error) {
    console.error("Failed to get watch history events:", error);
    return [];
  }
};

export const getAnalyticsData = async (): Promise<AnalyticsData> => {
  try {
    return await invoke<AnalyticsData>("get_analytics_data");
  } catch (error) {
    console.error("Failed to get analytics data:", error);
    return {
      overview: { total_watch_time_seconds: 0, movies_completed: 0, episodes_completed: 0, total_completion_rate: 0, current_streak_days: 0, total_events: 0 },
      heatmap: [],
      daily_trend: [],
      content_breakdown: [],
      source_breakdown: [],
      top_watched: [],
      hour_distribution: [],
      day_distribution: [],
      completion_funnel: { started: 0, in_progress_25: 0, mostly_done_75: 0, completed: 0 },
      library_stats: { movies: 0, shows: 0, episodes: 0 },
      recent_events: [],
    };
  }
};

// Remove a single item from watch history
export const removeFromWatchHistory = async (id: number): Promise<void> => {
  try {
    await invoke("remove_from_watch_history", { mediaId: id });
  } catch (error) {
    console.error("Failed to remove from watch history:", error);
    throw error;
  }
};

export const removeWatchHistoryEntry = async (eventId: string): Promise<void> => {
  try {
    await invoke("remove_watch_history_entry", { eventId });
  } catch (error) {
    console.error("Failed to remove watch history entry:", error);
    throw error;
  }
};

// Clear all watch history
export const clearAllWatchHistory = async (): Promise<void> => {
  try {
    await invoke("clear_all_watch_history");
  } catch (error) {
    console.error("Failed to clear watch history:", error);
    throw error;
  }
};

export const syncWatchHistory = async (): Promise<WatchHistorySyncStatus> => {
  try {
    return await invoke<WatchHistorySyncStatus>("sync_watch_history");
  } catch (error) {
    console.error("Failed to sync watch history:", error);
    throw error;
  }
};

// Mark media as complete (100% watched)
export const markAsComplete = async (
  mediaId: number,
): Promise<{ message: string }> => {
  try {
    return await invoke("mark_as_complete", { mediaId });
  } catch (error) {
    console.error("Failed to mark as complete:", error);
    throw error;
  }
};

// Clear all app data (reset to fresh state)
export const clearAllAppData = async (): Promise<void> => {
  try {
    // Clear localStorage
    localStorage.clear();
    // Clear database and image cache via backend
    await invoke("clear_all_app_data", { confirmed: true });
  } catch (error) {
    console.error("Failed to clear app data:", error);
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
    return await invoke<CleanupResponse>("cleanup_missing_metadata");
  } catch (error) {
    console.error("Failed to cleanup missing metadata:", error);
    throw error;
  }
};

// Repair broken file paths - finds files in media folders and updates database
export const repairFilePaths = async (): Promise<{ message: string }> => {
  try {
    return await invoke<{ message: string }>("repair_file_paths");
  } catch (error) {
    console.error("Failed to repair file paths:", error);
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
  episode_title?: string;
  season_number?: number;
  episode_number?: number;
  file_path?: string;
  parent_zip_id?: string;
  delete_kind?: "episode" | "zip_archive" | "ddl_source";
  archive_episode_count?: number;
  file_size_bytes?: number;
}

// Delete media files permanently from disk
export const deleteMediaFiles = async (
  mediaIds: number[],
): Promise<DeleteResponse> => {
  try {
    const response = await invoke<DeleteResponse>("delete_media_files", {
      mediaIds,
      confirmed: true,
    });
    return response;
  } catch (error) {
    console.error("Failed to delete media files:", error);
    throw error;
  }
};

// Delete ALL media files (cloud from Drive + all media entries from DB)
export const deleteAllMediaFiles = async (): Promise<DeleteResponse> => {
  try {
    const response = await invoke<DeleteResponse>("delete_all_media_files", {
      confirmed: true,
    });
    return response;
  } catch (error) {
    console.error("Failed to delete all media files:", error);
    throw error;
  }
};

// Delete cloud files by Drive file IDs (for selective delete)
export const deleteCloudFilesByDriveIds = async (
  driveFileIds: string[],
): Promise<DeleteResponse> => {
  try {
    const response = await invoke<DeleteResponse>("delete_cloud_files_by_drive_ids", {
      driveFileIds,
      confirmed: true,
    });
    return response;
  } catch (error) {
    console.error("Failed to delete cloud files:", error);
    throw error;
  }
};

// Get episodes for delete selection modal
export const getEpisodesForDelete = async (
  seriesId: number,
): Promise<EpisodeDeleteInfo[]> => {
  try {
    const episodes = await invoke<EpisodeDeleteInfo[]>(
      "get_episodes_for_delete",
      { seriesId },
    );
    return episodes;
  } catch (error) {
    console.error("Failed to get episodes for delete:", error);
    return [];
  }
};

// Delete a TV series and optionally its files
export const deleteSeries = async (
  seriesId: number,
  deleteFiles: boolean,
): Promise<DeleteResponse> => {
  try {
    const response = await invoke<DeleteResponse>("delete_series", {
      seriesId,
      deleteFiles,
      confirmed: true,
    });
    return response;
  } catch (error) {
    console.error("Failed to delete series:", error);
    throw error;
  }
};

// Delete just the cloud folder for a TV series (fallback if automatic deletion fails)
export const deleteSeriesCloudFolder = async (
  seriesId: number,
): Promise<{ message: string }> => {
  try {
    const response = await invoke<{ message: string }>(
      "delete_series_cloud_folder",
      { seriesId },
    );
    return response;
  } catch (error) {
    console.error("Failed to delete cloud folder:", error);
    throw error;
  }
};

// Get episodes for a TV show
export const getEpisodes = async (seriesId: number): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("get_episodes", { seriesId });
    return items;
  } catch (error) {
    console.error("Failed to get episodes:", error);
    return [];
  }
};

// Get configuration
export const getConfig = async (): Promise<Config> => {
  try {
    const config = await invoke<Config>("get_config");
    return config;
  } catch (error) {
    console.error("Failed to get config:", error);
    return {};
  }
};

// Save configuration
export const saveConfig = async (config: Config): Promise<void> => {
  try {
    await invoke("save_config", { newConfig: config, confirmed: true });
    // Notify other components that config changed (e.g., External tab re-checks addon_url)
    window.dispatchEvent(new CustomEvent('config-saved', { detail: config }));
  } catch (error) {
    console.error("Failed to save config:", error);
    throw error;
  }
};

// Auto-detect MPV executable on the system
export const autoDetectMpv = async (): Promise<string | null> => {
  try {
    const path = await invoke<string | null>("auto_detect_mpv");
    return path;
  } catch (error) {
    console.error("Failed to auto-detect MPV:", error);
    throw error;
  }
};

export interface BundledMpvInfo {
  exists: boolean;
  path: string;
}

// Get info about bundled MPV
export const getBundledMpvInfo = async (): Promise<BundledMpvInfo> => {
  try {
    return await invoke<BundledMpvInfo>("get_bundled_mpv_info");
  } catch (error) {
    console.error("Failed to get bundled MPV info:", error);
    return { exists: false, path: "" };
  }
};

// Download bundled MPV from GitHub releases
export const downloadBundledMpv = async (): Promise<string> => {
  try {
    return await invoke<string>("download_bundled_mpv");
  } catch (error) {
    console.error("Failed to download bundled MPV:", error);
    throw error;
  }
};

// Get resume info for a media item
export const getResumeInfo = async (id: number): Promise<ResumeInfo> => {
  try {
    const info = await invoke<ResumeInfo>("get_resume_info", { mediaId: id });
    return info;
  } catch (error) {
    console.error("Failed to get resume info:", error);
    return {
      has_progress: false,
      position: 0,
      duration: 0,
      time_str: "00:00:00",
      progress_percent: 0,
    };
  }
};

// Get media info by ID
export const getMediaInfo = async (id: number): Promise<MediaItem> => {
  try {
    const media = await invoke<MediaItem>("get_media_info", { mediaId: id });
    return media;
  } catch (error) {
    console.error("Failed to get media info:", error);
    throw error;
  }
};

export const getMediaTechnicalDetails = async (
  id: number,
): Promise<MediaTechnicalDetails | null> => {
  try {
    return await invoke<MediaTechnicalDetails>("get_media_technical_details", {
      mediaId: id,
    });
  } catch (error) {
    console.error("Failed to get media technical details:", error);
    return null;
  }
};

// Get stream info for built-in player
export const getStreamUrl = async (id: number): Promise<StreamInfo> => {
  try {
    const info = await invoke<StreamInfo>("get_stream_info", { mediaId: id });
    return info;
  } catch (error) {
    console.error("Failed to get stream info:", error);
    throw error;
  }
};

export const getAudioTracks = async (
  id: number,
): Promise<AudioTrackOption[]> => {
  try {
    return await invoke<AudioTrackOption[]>("get_audio_tracks", {
      mediaId: id,
    });
  } catch (error) {
    console.error("Failed to get audio tracks:", error);
    return [];
  }
};

export const resolveWatchHistoryMedia = async (event: WatchHistoryEvent): Promise<MediaItem> => {
  try {
    return await invoke<MediaItem>("resolve_watch_history_media", { event });
  } catch (error) {
    console.error("Failed to resolve watch history media:", error);
    throw error;
  }
};

export const getSubtitleTracks = async (
  id: number,
): Promise<SubtitleTrackOption[]> => {
  try {
    return await invoke<SubtitleTrackOption[]>("get_subtitle_tracks", {
      mediaId: id,
    });
  } catch (error) {
    console.error("Failed to get subtitle tracks:", error);
    return [];
  }
};

// Get stream info with automatic transcoding support for incompatible formats
export const getStreamUrlWithTranscode = async (
  id: number,
): Promise<StreamInfo> => {
  try {
    const info = await invoke<StreamInfo>("get_stream_info_with_transcode", {
      mediaId: id,
    });
    return info;
  } catch (error) {
    console.error("Failed to get stream info with transcode:", error);
    throw error;
  }
};

// Check if a file needs transcoding for HTML5 playback
export const checkNeedsTranscode = async (
  filePath: string,
): Promise<boolean> => {
  try {
    return await invoke<boolean>("check_needs_transcode", { filePath });
  } catch (error) {
    console.error("Failed to check transcode needs:", error);
    return false;
  }
};

// Transcode response type
export interface TranscodeResponse {
  session_id: number;
  stream_url: string;
}

// Start transcoding a video file
export const startTranscodeStream = async (
  filePath: string,
  startTime?: number,
): Promise<TranscodeResponse> => {
  try {
    return await invoke<TranscodeResponse>("start_transcode_stream", {
      filePath,
      startTime: startTime || null,
    });
  } catch (error) {
    console.error("Failed to start transcode stream:", error);
    throw error;
  }
};

// Stop a transcoding session
export const stopTranscodeStream = async (sessionId: number): Promise<void> => {
  try {
    await invoke("stop_transcode_stream", { sessionId });
  } catch (error) {
    console.error("Failed to stop transcode stream:", error);
  }
};

// Update watch progress
export const updateWatchProgress = async (
  id: number,
  currentTime: number,
  duration: number,
): Promise<void> => {
  try {
    await invoke("update_progress", {
      mediaId: id,
      currentTime,
      duration,
    });
  } catch (error) {
    console.error("Failed to update progress:", error);
  }
};

// Clear progress for a media item
export const clearProgress = async (id: number): Promise<void> => {
  try {
    await invoke("clear_progress", { mediaId: id });
  } catch (error) {
    console.error("Failed to clear progress:", error);
    throw error;
  }
};

// Update duration for a media item (write-back from TMDB)
export const updateEpisodeDuration = async (mediaId: number, durationSeconds: number): Promise<void> => {
  try {
    await invoke("update_episode_duration", { mediaId, durationSeconds });
  } catch (error) {
    console.error("Failed to update episode duration:", error);
  }
};

// Play media with MPV (external player)
export const playMedia = async (
  id: number,
  resume: boolean,
  audioLanguage?: string | null,
  subtitleLanguage?: string | null,
  durationSeconds?: number | null,
  fileSizeBytes?: number | null,
  subtitleFilePath?: string | null,
): Promise<void> => {
  try {
    await invoke("play_with_mpv", {
      mediaId: id,
      resume,
      audioLanguage: audioLanguage?.trim() || null,
      subtitleLanguage: subtitleLanguage?.trim() || null,
      subtitleFilePath: subtitleFilePath?.trim() || null,
      durationSecondsOverride: durationSeconds && durationSeconds > 0 ? durationSeconds : null,
      fileSizeBytesOverride: fileSizeBytes && fileSizeBytes > 0 ? fileSizeBytes : null,
    });
  } catch (error) {
    console.error("Failed to play with MPV:", error);
    throw error;
  }
};

// Play media with native libmpv player (embedded)
export const playMediaNative = async (
  id: number,
  resume: boolean,
  audioLanguage?: string | null,
  subtitleLanguage?: string | null,
): Promise<void> => {
  try {
    await invoke("play_with_native_mpv", {
      mediaId: id,
      resume,
      audioLanguage: audioLanguage?.trim() || null,
      subtitleLanguage: subtitleLanguage?.trim() || null,
    });
  } catch (error) {
    console.error("Failed to play with native MPV:", error);
    throw error;
  }
};

// Play media with VLC (external player)
export const playWithVlc = async (
  id: number,
  resume: boolean,
): Promise<void> => {
  try {
    await invoke("play_with_vlc", { mediaId: id, resume });
  } catch (error) {
    console.error("Failed to play with VLC:", error);
    throw error;
  }
};

// Fix match - update metadata from TMDB
export const fixMatch = async (
  id: number,
  tmdbId: string,
  type: "movie" | "tv",
  imdbId?: string,
): Promise<void> => {
  try {
    const timeoutMs = 45000;
    await Promise.race([
      invoke("fix_match", {
        mediaId: id,
        tmdbId,
        mediaType: type,
        imdbId,
      }),
      new Promise((_, reject) => {
        setTimeout(
          () => reject(new Error("Fix Match timed out. Please try again.")),
          timeoutMs,
        );
      }),
    ]);
  } catch (error) {
    console.error("Failed to fix match:", error);
    throw error;
  }
};

// Search library by cast member name
export const searchMediaByCast = async (castName: string): Promise<MediaItem[]> => {
  try {
    const items = await invoke<MediaItem[]>("search_media_by_cast", { castName });
    return items;
  } catch (error) {
    console.error("Failed to search media by cast:", error);
    return [];
  }
};

// Search TMDB by title
export const searchTmdb = async (
  query: string,
): Promise<TmdbSearchResponse> => {
  try {
    return await invoke<TmdbSearchResponse>("search_tmdb", { query });
  } catch (error) {
    console.error("Failed to search TMDB:", error);
    throw error;
  }
};

export interface HybridSearchResult {
  title: string;
  year: string | null;
  imdb_id: string;
  media_type: string;
  plot: string | null;
  poster_url: string | null;
  genre: string | null;
  director: string | null;
  actors: string | null;
  imdb_rating: number | null;
  tmdb_id: number | null;
  tmdb_poster_path: string | null;
  tmdb_backdrop_path: string | null;
  tmdb_vote_average: number | null;
}

export const searchContent = async (
  query: string,
  year?: number,
  media_type?: string,
): Promise<HybridSearchResult[]> => {
  const response = await invoke<{ results: HybridSearchResult[] }>("search_content", {
    query,
    year,
    mediaType: media_type,
  });
  return response.results;
};

export const getDownloadJobs = async (): Promise<DownloadJob[]> => {
  try {
    return await invoke<DownloadJob[]>("get_download_jobs");
  } catch (error) {
    console.error("Failed to get download jobs:", error);
    return [];
  }
};

export const startMediaDownload = async (mediaId: number): Promise<DownloadJob> => {
  return await invoke<DownloadJob>("start_media_download", { mediaId });
};

export const cancelDownloadJob = async (jobId: string): Promise<DownloadJob> => {
  return await invoke<DownloadJob>("cancel_download_job", { jobId });
};

export const deleteDownloadJob = async (jobId: string): Promise<void> => {
  await invoke("delete_download_job", { jobId });
};

export const clearDownloadHistory = async (): Promise<void> => {
  await invoke("clear_download_history");
};

export const openDownloadJobTarget = async (jobId: string): Promise<void> => {
  await invoke("open_download_job_target", { jobId });
};

export const getTmdbTrending = async (): Promise<TmdbTrendingResponse> => {
  try {
    return await invoke<TmdbTrendingResponse>("get_tmdb_trending");
  } catch (error) {
    console.error("Failed to fetch TMDB trending:", error);
    throw error;
  }
};

// Get cached image URL (converts local path to asset protocol URL)
export const getCachedImageUrl = (
  imageName: string,
): Promise<string | null> => {
  if (imageUrlCache.has(imageName)) {
    return imageUrlCache.get(imageName)!;
  }

  const promise = (async () => {
    try {
      const filePath = await invoke<string>("get_cached_image_path", {
        imageName,
      });
      return convertFileSrc(filePath);
    } catch (error) {
      console.debug("[ImageCache] Cache miss:", imageName);
      return null;
    }
  })();

  imageUrlCache.set(imageName, promise);
  return promise;
};

// Helper to get poster URL from media item
export const getPosterUrl = (item: MediaItem): string | null => {
  if (!item.poster_path) return null;

  // If it's already a full URL, return as-is
  if (
    item.poster_path.startsWith("http") ||
    item.poster_path.startsWith("asset://")
  ) {
    return item.poster_path;
  }

  // For now, return null - components should call getCachedImageUrl() themselves
  // since it's async and this function is synchronous
  return null;
};

// Player preferences
export type PlayerPreference = "mpv" | "vlc" | "builtin" | "ask";

export const getPlayerPreference = (): PlayerPreference => {
  return (
    (localStorage.getItem("playerPreference") as PlayerPreference) || "ask"
  );
};

export const setPlayerPreference = (preference: PlayerPreference): void => {
  localStorage.setItem("playerPreference", preference);
};

const SERIES_AUDIO_PREFERENCE_KEY = "slasshyvault_series_audio_preferences";
const SERIES_SUBTITLE_PREFERENCE_KEY = "slasshyvault_series_subtitle_preferences";
const AUDIO_TRACK_CACHE_KEY = "slasshyvault_detected_audio_tracks_v2";
const SUBTITLE_TRACK_CACHE_KEY = "slasshyvault_detected_subtitle_tracks_v1";

function readMapFromStorage<T>(key: string): Record<string, T> {
  try {
    const stored = localStorage.getItem(key);
    if (!stored) {
      return {};
    }

    const parsed = JSON.parse(stored);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch (error) {
    console.error(`Failed to read map from "${key}":`, error);
    return {};
  }
}

const readSeriesAudioPreferenceMap = (): Record<string, string> =>
  readMapFromStorage<string>(SERIES_AUDIO_PREFERENCE_KEY);

const readSeriesSubtitlePreferenceMap = (): Record<string, string> =>
  readMapFromStorage<string>(SERIES_SUBTITLE_PREFERENCE_KEY);

export const getSeriesAudioPreference = (
  seriesId: number,
): string | null => {
  const stored = readSeriesAudioPreferenceMap()[String(seriesId)];
  const normalized = typeof stored === "string" ? stored.trim() : "";
  return normalized.length > 0 ? normalized : null;
};

export const setSeriesAudioPreference = (
  seriesId: number,
  audioLanguage: string | null,
): void => {
  try {
    const preferences = readSeriesAudioPreferenceMap();
    const normalized = audioLanguage?.trim() || "";

    if (normalized) {
      preferences[String(seriesId)] = normalized;
    } else {
      delete preferences[String(seriesId)];
    }

    localStorage.setItem(
      SERIES_AUDIO_PREFERENCE_KEY,
      JSON.stringify(preferences),
    );
  } catch (error) {
    console.error("Failed to save series audio preference:", error);
  }
};

export const getSeriesSubtitlePreference = (
  seriesId: number,
): string | null => {
  const stored = readSeriesSubtitlePreferenceMap()[String(seriesId)];
  const normalized = typeof stored === "string" ? stored.trim() : "";
  return normalized.length > 0 ? normalized : null;
};

export const setSeriesSubtitlePreference = (
  seriesId: number,
  subtitleLanguage: string | null,
): void => {
  try {
    const preferences = readSeriesSubtitlePreferenceMap();
    const normalized = subtitleLanguage?.trim() || "";

    if (normalized) {
      preferences[String(seriesId)] = normalized;
    } else {
      delete preferences[String(seriesId)];
    }

    localStorage.setItem(
      SERIES_SUBTITLE_PREFERENCE_KEY,
      JSON.stringify(preferences),
    );
  } catch (error) {
    console.error("Failed to save series subtitle preference:", error);
  }
};

const SERIES_SPOILER_PREFERENCE_KEY = "slasshyvault_series_spoiler_preferences";

const readSeriesSpoilerPreferenceMap = (): Record<string, boolean> =>
  readMapFromStorage<boolean>(SERIES_SPOILER_PREFERENCE_KEY);

export const getSeriesSpoilerEnabled = (
  seriesId: number,
): boolean => {
  const stored = readSeriesSpoilerPreferenceMap()[String(seriesId)];
  return stored !== false;
};

export const setSeriesSpoilerEnabled = (
  seriesId: number,
  enabled: boolean,
): void => {
  try {
    const preferences = readSeriesSpoilerPreferenceMap();
    if (enabled) {
      delete preferences[String(seriesId)];
    } else {
      preferences[String(seriesId)] = false;
    }

    localStorage.setItem(
      SERIES_SPOILER_PREFERENCE_KEY,
      JSON.stringify(preferences),
    );
  } catch (error) {
    console.error("Failed to save series spoiler preference:", error);
  }
};

const readAudioTrackCacheMap = (): Record<string, AudioTrackOption[]> =>
  readMapFromStorage<AudioTrackOption[]>(AUDIO_TRACK_CACHE_KEY);

const readSubtitleTrackCacheMap = (): Record<string, SubtitleTrackOption[]> =>
  readMapFromStorage<SubtitleTrackOption[]>(SUBTITLE_TRACK_CACHE_KEY);

export const getCachedSeriesAudioTracks = (
  seriesId: number,
): AudioTrackOption[] | null => {
  const cache = readAudioTrackCacheMap();
  const direct = cache[String(seriesId)];
  if (Array.isArray(direct)) {
    return direct;
  }

  const legacyEntry = Object.entries(cache).find(([key, value]) =>
    key.startsWith(`${seriesId}:`) && Array.isArray(value),
  );
  return legacyEntry?.[1] ?? null;
};

export const setCachedSeriesAudioTracks = (
  seriesId: number,
  tracks: AudioTrackOption[],
): void => {
  try {
    const cache = readAudioTrackCacheMap();
    cache[String(seriesId)] = tracks;
    localStorage.setItem(AUDIO_TRACK_CACHE_KEY, JSON.stringify(cache));
  } catch (error) {
    console.error("Failed to cache detected audio tracks:", error);
  }
};

export const getCachedSeriesSubtitleTracks = (
  seriesId: number,
): SubtitleTrackOption[] | null => {
  const cache = readSubtitleTrackCacheMap();
  const direct = cache[String(seriesId)];
  return Array.isArray(direct) ? direct : null;
};

export const setCachedSeriesSubtitleTracks = (
  seriesId: number,
  tracks: SubtitleTrackOption[],
): void => {
  try {
    const cache = readSubtitleTrackCacheMap();
    cache[String(seriesId)] = tracks;
    localStorage.setItem(SUBTITLE_TRACK_CACHE_KEY, JSON.stringify(cache));
  } catch (error) {
    console.error("Failed to cache detected subtitle tracks:", error);
  }
};

const normalizeAudioTrackText = (value?: string | null): string => {
  return value?.trim().toLowerCase() || "";
};

const audioTrackCacheIdentity = (track: AudioTrackOption): string => {
  const trackId = track.track_id ?? "";
  const mpvValue = normalizeAudioTrackText(track.mpv_value);
  const languageCode = normalizeAudioTrackText(track.language_code);
  const label = normalizeAudioTrackText(track.label);
  const detail = normalizeAudioTrackText(track.detail);

  return [trackId, mpvValue, languageCode, label, detail].join("|");
};

const audioTracksLikelyMatch = (
  left: AudioTrackOption,
  right: AudioTrackOption,
): boolean => {
  if (
    left.track_id != null &&
    right.track_id != null &&
    left.track_id === right.track_id
  ) {
    return true;
  }

  const leftMpvValue = normalizeAudioTrackText(left.mpv_value);
  const rightMpvValue = normalizeAudioTrackText(right.mpv_value);
  if (leftMpvValue && rightMpvValue && leftMpvValue === rightMpvValue) {
    return true;
  }

  const leftLanguage = normalizeAudioTrackText(left.language_code);
  const rightLanguage = normalizeAudioTrackText(right.language_code);
  const leftLabel = normalizeAudioTrackText(left.label);
  const rightLabel = normalizeAudioTrackText(right.label);

  return (
    !!leftLanguage &&
    !!rightLanguage &&
    leftLanguage === rightLanguage &&
    leftLabel === rightLabel
  );
};

function mergeCachedSeriesTracks<T extends AudioTrackOption>(
  seriesId: number,
  tracks: T[],
  getter: (id: number) => T[] | null,
  setter: (id: number, tracks: T[]) => void,
): void {
  const existingTracks = getter(seriesId) ?? [];
  if (existingTracks.length === 0) {
    setter(seriesId, tracks);
    return;
  }

  const merged = [...existingTracks];

  for (const incomingTrack of tracks) {
    const existingIndex = merged.findIndex((cachedTrack) =>
      audioTracksLikelyMatch(cachedTrack, incomingTrack),
    );

    if (existingIndex >= 0) {
      merged[existingIndex] = {
        ...merged[existingIndex],
        ...incomingTrack,
      };
      continue;
    }

    merged.push(incomingTrack);
  }

  const seenIdentities = new Set<string>();
  const deduped = merged.filter((track) => {
    const identity = audioTrackCacheIdentity(track);
    if (seenIdentities.has(identity)) return false;
    seenIdentities.add(identity);
    return true;
  });

  deduped.sort((left, right) => left.label.localeCompare(right.label));
  setter(seriesId, deduped);
}

export const mergeCachedSeriesAudioTracks = (
  seriesId: number,
  tracks: AudioTrackOption[],
): void => {
  mergeCachedSeriesTracks(seriesId, tracks, getCachedSeriesAudioTracks, setCachedSeriesAudioTracks);
};

export const mergeCachedSeriesSubtitleTracks = (
  seriesId: number,
  tracks: SubtitleTrackOption[],
): void => {
  mergeCachedSeriesTracks(seriesId, tracks, getCachedSeriesSubtitleTracks, setCachedSeriesSubtitleTracks);
};

const matchesAudioTrackPreference = (
  track: AudioTrackOption,
  storedPreference: string,
): boolean => {
  const normalizedPreference = storedPreference.trim().toLowerCase();
  if (!normalizedPreference) {
    return false;
  }

  const preferenceParts = normalizedPreference
    .split(",")
    .flatMap((part) => {
      const trimmed = part.trim();
      return trimmed ? [trimmed] : [];
    });

  const languageCode = track.language_code?.trim().toLowerCase();
  const label = track.label.trim().toLowerCase();
  const detail = track.detail?.trim().toLowerCase();
  const mpvValue = track.mpv_value?.trim().toLowerCase();

  return (
    mpvValue === normalizedPreference ||
    languageCode === normalizedPreference ||
    label === normalizedPreference ||
    detail === normalizedPreference ||
    (!!languageCode && preferenceParts.includes(languageCode)) ||
    preferenceParts.includes(label)
  );
};

export const resolveSeriesAudioPreferenceForPlayback = (
  seriesId: number | null | undefined,
  _seasonNumber?: number | null,
): string | null => {
  if (!seriesId) {
    return null;
  }

  const storedPreference = getSeriesAudioPreference(seriesId);
  if (!storedPreference) {
    return null;
  }

  const cachedTracks = getCachedSeriesAudioTracks(seriesId);
  if (!cachedTracks || cachedTracks.length === 0) {
    return storedPreference;
  }

  const matchedTrack = cachedTracks.find((track) =>
    matchesAudioTrackPreference(track, storedPreference),
  );

  return matchedTrack?.mpv_value?.trim() || storedPreference;
};

export const resolveSeriesSubtitlePreferenceForPlayback = (
  seriesId: number | null | undefined,
  _seasonNumber?: number | null,
): string | null => {
  if (!seriesId) {
    return null;
  }

  const storedPreference = getSeriesSubtitlePreference(seriesId);
  if (!storedPreference) {
    return null;
  }

  const cachedTracks = getCachedSeriesSubtitleTracks(seriesId);
  if (!cachedTracks || cachedTracks.length === 0) {
    return storedPreference;
  }

  const matchedTrack = cachedTracks.find((track) =>
    matchesAudioTrackPreference(track, storedPreference),
  );

  return matchedTrack?.mpv_value?.trim() || storedPreference;
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
    const status = await invoke<MpvStatus>("get_mpv_status", { mediaId });
    return status;
  } catch (error) {
    console.error("Failed to get MPV status:", error);
    return { is_playing: false, media_id: mediaId };
  }
};

// Get all active MPV sessions
export const getActiveMpvSessions = async (): Promise<MpvSession[]> => {
  try {
    const sessions = await invoke<MpvSession[]>("get_active_mpv_sessions");
    return sessions;
  } catch (error) {
    console.error("Failed to get active MPV sessions:", error);
    return [];
  }
};

// ==================== TMDB EPISODE METADATA ====================

// Episode info from TMDB with rich metadata
export interface TmdbEpisodeInfo {
  season_number?: number | null;
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

export interface ImdbEpisodeRating {
  imdb_id: string;
  imdb_rating: number | null;
  imdb_votes: number | null;
  still_url?: string | null;
  title?: string | null;
  plot?: string | null;
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
  first_air_date?: string;
  status?: string;
  vote_average?: number;
  networks?: { id?: number; name: string }[];
  genres?: { id: number; name: string }[];
  number_of_episodes?: number;
  number_of_seasons: number;
  seasons: {
    season_number: number;
    name: string;
    episode_count: number;
    overview?: string;
    poster_path?: string;
    air_date?: string;
  }[];
  creator?: string;
  last_episode_to_air?: TmdbEpisodeInfo | null;
  next_episode_to_air?: TmdbEpisodeInfo | null;
}

export interface TmdbMovieDetails {
  id: number;
  title: string;
  poster_path?: string;
  backdrop_path?: string;
  overview?: string;
  release_date?: string;
  status?: string;
  vote_average?: number;
  genres?: { id: number; name: string }[];
  runtime?: number;
  director?: string;
}

export interface MovieReminder {
  id: number;
  tmdb_id: string;
  media_type: "movie" | "tv";
  title: string;
  poster_path?: string | null;
  season_number?: number | null;
  episode_number?: number | null;
  release_date?: string | null;
  reminder_at: string;
  source: "tmdb" | "manual" | string;
  tracking_mode?: "single" | "tv_season" | string;
  tracking_season_number?: number | null;
  notes?: string | null;
  is_active: boolean;
  notified_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface MovieReminderInput {
  tmdbId: string;
  mediaType: "movie" | "tv";
  title: string;
  posterPath?: string | null;
  seasonNumber?: number | null;
  episodeNumber?: number | null;
  releaseDate?: string | null;
  reminderAt: string;
  source?: "tmdb" | "manual" | string;
  trackingMode?: "single" | "tv_season" | string;
  trackingSeasonNumber?: number | null;
  notes?: string | null;
  isActive?: boolean;
}

export interface TmdbReleaseSchedule {
  tmdbId: number;
  mediaType: "movie" | "tv";
  title: string;
  seasonNumber?: number | null;
  episodeNumber?: number | null;
  releaseDate?: string | null;
  suggestedReminderAt?: string | null;
  source: "tmdb" | string;
  precision: "date" | "datetime" | string;
  editable: boolean;
}

export interface WatchlistItem {
  id: number;
  tmdb_id: string;
  media_type: "movie" | "tv";
  title: string;
  poster_path?: string | null;
  release_date?: string | null;
  notes?: string | null;
  is_active: boolean;
  notification_enabled: boolean;
  notification_mode: "single" | "spam" | string;
  notification_interval_minutes?: number | null;
  notify_at?: string | null;
  last_notified_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface WatchlistItemInput {
  tmdbId: string;
  mediaType: "movie" | "tv";
  title: string;
  posterPath?: string | null;
  releaseDate?: string | null;
  notes?: string | null;
  isActive?: boolean;
  notificationEnabled?: boolean;
  notificationMode?: "single" | "spam" | string;
  notificationIntervalMinutes?: number | null;
  notifyAt?: string | null;
}

export interface WatchlistSyncStatus {
  synced: boolean;
  merged_remote_items: number;
  uploaded_items: number;
  skipped_reason?: string | null;
}

export const getMovieDetails = async (
  movieId: number,
  imdbId?: string | null,
): Promise<TmdbMovieDetails | null> => {
  try {
    const details = await invoke<TmdbMovieDetails>("get_movie_details", {
      movieId,
      imdbId: imdbId ?? null,
    });
    return details;
  } catch (error) {
    console.error("Failed to get movie details:", error);
    return null;
  }
};

export const getTmdbReleaseSchedule = async (
  tmdbId: number,
  mediaType: "movie" | "tv",
  seasonNumber?: number | null,
  episodeNumber?: number | null,
  imdbId?: string | null,
): Promise<TmdbReleaseSchedule> => {
  return await invoke<TmdbReleaseSchedule>("get_tmdb_release_schedule", {
    tmdbId,
    mediaType,
    seasonNumber: seasonNumber ?? null,
    episodeNumber: episodeNumber ?? null,
    imdbId: imdbId ?? null,
  });
};

const toReminderBackendInput = (reminder: MovieReminderInput) => ({
  tmdbId: reminder.tmdbId,
  mediaType: reminder.mediaType,
  title: reminder.title,
  posterPath: reminder.posterPath ?? null,
  seasonNumber: reminder.seasonNumber ?? null,
  episodeNumber: reminder.episodeNumber ?? null,
  releaseDate: reminder.releaseDate ?? null,
  reminderAt: reminder.reminderAt,
  source: reminder.source ?? "manual",
  trackingMode: reminder.trackingMode ?? "single",
  trackingSeasonNumber: reminder.trackingSeasonNumber ?? null,
  notes: reminder.notes ?? null,
  isActive: reminder.isActive ?? true,
});

export const getMovieReminders = async (
  includeInactive = false,
): Promise<MovieReminder[]> => {
  return await invoke<MovieReminder[]>("get_movie_reminders", {
    includeInactive,
  });
};

export const createMovieReminder = async (
  reminder: MovieReminderInput,
): Promise<MovieReminder> => {
  return await invoke<MovieReminder>("create_movie_reminder", {
    reminder: toReminderBackendInput(reminder),
  });
};

export const updateMovieReminder = async (
  id: number,
  reminder: MovieReminderInput,
): Promise<MovieReminder> => {
  return await invoke<MovieReminder>("update_movie_reminder", {
    id,
    reminder: toReminderBackendInput(reminder),
  });
};

export const deleteMovieReminder = async (id: number): Promise<void> => {
  await invoke("delete_movie_reminder", { id });
};

export const setMovieReminderActive = async (
  id: number,
  isActive: boolean,
): Promise<MovieReminder> => {
  return await invoke<MovieReminder>("set_movie_reminder_active", {
    id,
    isActive,
  });
};

const toWatchlistBackendInput = (item: WatchlistItemInput) => ({
  tmdbId: item.tmdbId,
  mediaType: item.mediaType,
  title: item.title,
  posterPath: item.posterPath ?? null,
  releaseDate: item.releaseDate ?? null,
  notes: item.notes ?? null,
  isActive: item.isActive ?? true,
  notificationEnabled: item.notificationEnabled ?? false,
  notificationMode: item.notificationMode ?? "single",
  notificationIntervalMinutes: item.notificationIntervalMinutes ?? null,
  notifyAt: item.notifyAt ?? null,
});

export const getWatchlistItems = async (
  includeInactive = false,
): Promise<WatchlistItem[]> => {
  return await invoke<WatchlistItem[]>("get_watchlist_items", {
    includeInactive,
  });
};

export const createOrUpdateWatchlistItem = async (
  item: WatchlistItemInput,
): Promise<WatchlistItem> => {
  return await invoke<WatchlistItem>("create_or_update_watchlist_item", {
    item: toWatchlistBackendInput(item),
  });
};

export const updateWatchlistItem = async (
  id: number,
  item: WatchlistItemInput,
): Promise<WatchlistItem> => {
  return await invoke<WatchlistItem>("update_watchlist_item", {
    id,
    item: toWatchlistBackendInput(item),
  });
};

export const deleteWatchlistItem = async (id: number): Promise<void> => {
  await invoke("delete_watchlist_item", { id });
};

export const syncWatchlist = async (): Promise<WatchlistSyncStatus> => {
  return await invoke<WatchlistSyncStatus>("sync_watchlist");
};

// Get TV show details including seasons from TMDB
export const getTvDetails = async (
  tvId: number,
  imdbId?: string | null,
): Promise<TmdbShowDetails | null> => {
  try {
    const details = await invoke<TmdbShowDetails>("get_tv_details", {
      tvId,
      imdbId: imdbId ?? null,
    });
    return details;
  } catch (error) {
    console.error("Failed to get TV details:", error);
    return null;
  }
};

// Get episodes for a specific season from TMDB (with full metadata)
export const getTvSeasonEpisodes = async (
  tvId: number,
  seasonNumber: number,
  imdbId?: string | null,
): Promise<TmdbSeasonDetails | null> => {
  try {
    const seasonDetails = await invoke<TmdbSeasonDetails>(
      "get_tv_season_episodes",
      { tvId, seasonNumber, imdbId: imdbId ?? null },
    );
    return seasonDetails;
  } catch (error) {
    console.error("Failed to get season episodes:", error);
    return null;
  }
};

// Fetch IMDb ratings for season episodes (uses OMDb API)
export const getEpisodeImdbRatings = async (
  tvId: number,
  seasonNumber: number,
  episodeNumbers: number[],
  showImdbId?: string,
): Promise<Record<number, ImdbEpisodeRating>> => {
  try {
    const ratings = await invoke<Record<number, ImdbEpisodeRating>>(
      "get_episode_imdb_ratings",
      { tvId, seasonNumber, episodeNumbers, showImdbId },
    );
    return ratings;
  } catch (error) {
    console.error("Failed to get IMDb ratings:", error);
    return {};
  }
};

export interface ImdbAward {
  event: string;
  year: number | null;
  category: string;
  is_winner: boolean;
}

export interface ImdbDetails {
  imdb_id: string;
  title: string | null;
  aggregate_rating: number | null;
  vote_count: number | null;
  metacritic_score: number | null;
  metacritic_url: string | null;
  metacritic_review_count: number | null;
  plot: string | null;
  genres: string[] | null;
  directors: string[] | null;
  writers: string[] | null;
  stars: string[] | null;
  runtime_seconds: number | null;
  origin_countries: string[] | null;
  interests: string[] | null;
  start_year: number | null;
  end_year: number | null;
  mpaa_rating: string | null;
  domestic_gross: string | null;
  worldwide_gross: string | null;
  opening_weekend_gross: string | null;
  production_budget: string | null;
  total_nominations: number | null;
  total_wins: number | null;
  awards: ImdbAward[] | null;
  primary_image_url?: string | null;
}

export const getImdbDetails = async (params: {
  imdbId?: string;
  tmdbId?: number;
  mediaType?: string;
}): Promise<ImdbDetails | null> => {
  try {
    return await invoke<ImdbDetails>("get_imdb_details", params);
  } catch (error) {
    console.error("Failed to fetch IMDb details:", error);
    return null;
  }
};

export interface ParentsGuideCategory {
  category: string;
  severity_breakdowns: { severity_level: string; vote_count: number }[] | null;
}

export const getParentsGuide = async (imdbId: string): Promise<ParentsGuideCategory[] | null> => {
  try {
    return await invoke<ParentsGuideCategory[]>("get_parents_guide", { imdbId });
  } catch (error) {
    console.error("Failed to fetch parents guide:", error);
    return null;
  }
};

export interface TmdbReview {
  author: string;
  content: string;
  rating: number | null;
  created_at: string | null;
  url: string | null;
}

export const getTmdbReviews = async (
  tmdbId: number,
  mediaType: string,
): Promise<TmdbReview[]> => {
  try {
    return await invoke<TmdbReview[]>("get_tmdb_reviews", { tmdbId, mediaType });
  } catch (error) {
    console.error("Failed to fetch TMDB reviews:", error);
    return [];
  }
};

// Force refresh episode metadata for a TV series (re-downloads images)
export const refreshSeriesMetadata = async (
  tvId: number,
  seriesTitle: string,
): Promise<string> => {
  try {
    const result = await invoke<string>("refresh_series_metadata", {
      tvId,
      seriesTitle,
    });
    return result;
  } catch (error) {
    console.error("Failed to refresh series metadata:", error);
    throw error;
  }
};

// TMDB image URL helper
const TMDB_IMAGE_BASE = "https://image.tmdb.org/t/p";

export const getTmdbImageUrl = (
  path: string | undefined,
  size: "w92" | "w185" | "w300" | "w500" | "original" = "w300",
): string | null => {
  if (!path) return null;
  return `${TMDB_IMAGE_BASE}/${size}${path}`;
};

// ==================== ONBOARDING ====================

// ==================== TAB VISIBILITY ====================

const TAB_VISIBILITY_KEY = "slasshyvault_tab_visibility";

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
    console.error("Failed to get tab visibility:", error);
  }
  // Default: cloud-only mode (no local tab)
  return { showLocal: false, showCloud: true };
};

// Save tab visibility settings
export const setTabVisibility = (visibility: TabVisibility): void => {
  try {
    localStorage.setItem(TAB_VISIBILITY_KEY, JSON.stringify(visibility));
  } catch (error) {
    console.error("Failed to save tab visibility:", error);
  }
};

// ==================== BETA FEATURES ====================

const BETA_FEATURES_KEY = "slasshyvault_beta_features";
export interface BetaFeatures {
  enabled: boolean;
}

// Check if beta features are enabled
export const isBetaEnabled = (): boolean => {
  try {
    const stored = localStorage.getItem(BETA_FEATURES_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as BetaFeatures;
      return parsed.enabled === true;
    }
  } catch (error) {
    console.error("Failed to get beta features state:", error);
  }
  return false;
};

// Enable or disable beta features
export const setBetaEnabled = (enabled: boolean): void => {
  try {
    localStorage.setItem(BETA_FEATURES_KEY, JSON.stringify({ enabled }));
  } catch (error) {
    console.error("Failed to save beta features state:", error);
  }
};

// ==================== TMDB KEY NOTICE ====================

const TMDB_KEY_NOTICE_DISMISSED_KEY = "slasshyvault_tmdb_key_notice_dismissed";

// Check if the user has explicitly dismissed the TMDB key setup notice
export const isTmdbKeyNoticeDismissed = (): boolean => {
  try {
    return localStorage.getItem(TMDB_KEY_NOTICE_DISMISSED_KEY) === "true";
  } catch (error) {
    console.error("Failed to read TMDB key notice state:", error);
    return false;
  }
};

// Persist dismissal so the notice only shows until the user opts out
export const setTmdbKeyNoticeDismissed = (dismissed: boolean): void => {
  try {
    localStorage.setItem(
      TMDB_KEY_NOTICE_DISMISSED_KEY,
      dismissed ? "true" : "false"
    );
  } catch (error) {
    console.error("Failed to save TMDB key notice state:", error);
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
    return await invoke<CloudCacheInfo>("get_cloud_cache_info");
  } catch (error) {
    console.error("Failed to get cloud cache info:", error);
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
    return await invoke<{ message: string }>("cleanup_cloud_cache");
  } catch (error) {
    console.error("Failed to cleanup cloud cache:", error);
    throw error;
  }
};

// Clear all cloud cache
export const clearCloudCache = async (): Promise<{ message: string }> => {
  try {
    return await invoke<{ message: string }>("clear_cloud_cache");
  } catch (error) {
    console.error("Failed to clear cloud cache:", error);
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
    return await invoke<boolean>("gdrive_is_connected");
  } catch (error) {
    console.error("Failed to check GDrive connection:", error);
    return false;
  }
};

// Get Google Drive account info including storage stats
export const getGdriveAccountInfo =
  async (): Promise<DriveAccountInfo | null> => {
    try {
      return await invoke<DriveAccountInfo>("gdrive_get_account_info");
    } catch (error) {
      console.error("Failed to get GDrive account info:", error);
      return null;
    }
  };

// ==================== STORAGE ANALYTICS ====================

export const getTopSpaceConsumers = async (
  limit: number = 10,
): Promise<MediaItem[]> => {
  try {
    return await invoke<MediaItem[]>("get_top_space_consumers", { limit });
  } catch (error) {
    console.error("Failed to get top space consumers:", error);
    return [];
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
    return await invoke<UpdateInfo>("check_for_updates");
  } catch (error) {
    console.error("Failed to check for updates:", error);
    throw error;
  }
};

// Download update to temp directory (returns installer path)
export const downloadUpdate = async (url: string, pubDate?: string): Promise<string> => {
  try {
    return await invoke<string>("download_update", { url, pubDate });
  } catch (error) {
    console.error("Failed to download update:", error);
    throw error;
  }
};

// Install update and restart app
export const installUpdate = async (installerPath: string): Promise<void> => {
  try {
    await invoke("install_update", { installerPath });
  } catch (error) {
    console.error("Failed to install update:", error);
    throw error;
  }
};

// Get current app version
export const getAppVersion = async (): Promise<string> => {
  try {
    return await invoke<string>("get_app_version");
  } catch (error) {
    console.error("Failed to get app version:", error);
    return "0.0.0";
  }
};

// ==================== WATCH TOGETHER ====================

export interface WatchParticipant {
  id: string;
  nickname: string;
  is_host: boolean;
  is_ready: boolean;
  duration?: number;
}

export interface WatchRoom {
  code: string;
  host_id: string;
  media_title: string;
  media_id: number;
  participants: WatchParticipant[];
  is_playing: boolean;
  state?: string;
  current_position: number;
}

export interface SyncCommand {
  action: "play" | "pause" | "seek";
  position: number;
  from?: string;
  timestamp?: number;
}

export interface WatchEvent {
  type:
    | "room_updated"
    | "sync_command"
    | "participant_changed"
    | "playback_started"
    | "state_update"
    | "error"
    | "disconnected";
  room?: WatchRoom;
  command?: SyncCommand;
  position?: number;
  message?: string;
}

// Create a Watch Together room
export const wtCreateRoom = async (
  mediaId: number,
  title: string,
  mediaMatchKey: string | undefined,
  nickname: string,
  filePath?: string,
): Promise<WatchRoom> => {
  try {
    return await invoke<WatchRoom>("wt_create_room", {
      mediaId,
      title,
      mediaMatchKey: mediaMatchKey ?? null,
      nickname,
      filePath: filePath ?? null,
    });
  } catch (error) {
    console.error("Failed to create watch room:", error);
    throw error;
  }
};

// Join an existing Watch Together room
export const wtJoinRoom = async (
  roomCode: string,
  mediaId: number,
  mediaTitle: string,
  mediaMatchKey: string | undefined,
  nickname: string,
  filePath?: string,
): Promise<WatchRoom> => {
  try {
    return await invoke<WatchRoom>("wt_join_room", {
      roomCode,
      mediaId,
      mediaTitle,
      mediaMatchKey: mediaMatchKey ?? null,
      nickname,
      filePath: filePath ?? null,
    });
  } catch (error) {
    console.error("Failed to join watch room:", error);
    throw error;
  }
};

// Leave the current Watch Together room
export const wtLeaveRoom = async (): Promise<void> => {
  try {
    await invoke("wt_leave_room");
  } catch (error) {
    console.error("Failed to leave watch room:", error);
    throw error;
  }
};

// Set ready status with video duration
export const wtSetReady = async (duration: number): Promise<void> => {
  try {
    await invoke("wt_set_ready", { duration });
  } catch (error) {
    console.error("Failed to set ready status:", error);
    throw error;
  }
};

// Start playback (host only)
export const wtStartPlayback = async (): Promise<void> => {
  try {
    await invoke("wt_start_playback");
  } catch (error) {
    console.error("Failed to start playback:", error);
    throw error;
  }
};

// Send a sync command
export const wtSendSync = async (
  action: string,
  position: number,
): Promise<void> => {
  try {
    await invoke("wt_send_sync", { action, position });
  } catch (error) {
    console.error("Failed to send sync:", error);
    throw error;
  }
};

// Get current room state
export const wtGetRoomState = async (): Promise<WatchRoom | null> => {
  try {
    return await invoke<WatchRoom | null>("wt_get_room_state");
  } catch (error) {
    console.error("Failed to get room state:", error);
    return null;
  }
};

// Check if Watch Together session is active
export const wtIsActive = async (): Promise<boolean> => {
  try {
    return await invoke<boolean>("wt_is_active");
  } catch (error) {
    console.error("Failed to check watch session:", error);
    return false;
  }
};

// Get local client ID for the current Watch Together session
export const wtGetClientId = async (): Promise<string> => {
  try {
    const clientId = await invoke<string | null>("wt_get_client_id");
    return clientId || "";
  } catch (error) {
    console.error("Failed to get WT client ID:", error);
    return "";
  }
};

// Launch MPV in Watch Together sync mode
// Always uses the client ID from wtGetClientId() — ignores passed sessionId
// to prevent callers from mistakenly passing a room code.
export const wtLaunchMpv = async (
  mediaId: number,
  _sessionId: string,
  startPosition: number = 0,
): Promise<number> => {
  try {
    const effectiveSessionId = await wtGetClientId();
    return await invoke<number>("wt_launch_mpv", {
      mediaId,
      sessionId: effectiveSessionId,
      startPosition,
    });
  } catch (error) {
    console.error("Failed to launch MPV for watch together:", error);
    throw error;
  }
};

// Send a command to MPV in Watch Together mode
export const wtSendMpvCommand = async (
  sessionId: string,
  action: string,
  position: number,
): Promise<void> => {
  try {
    await invoke("wt_send_mpv_command", { sessionId, action, position });
  } catch (error) {
    console.error("Failed to send MPV command:", error);
    throw error;
  }
};

// ==================== SYNC VALIDATOR ====================

// Duplicate detection types
export interface DuplicateGroup {
  items: MediaItem[];
  reason: string;
}

export const findDuplicateMedia = async (): Promise<DuplicateGroup[]> => {
  try {
    return await invoke<DuplicateGroup[]>("find_duplicate_media");
  } catch (error) {
    console.error("Failed to find duplicate media:", error);
    return [];
  }
};

export interface SyncIssue {
  category: string;
  file_name: string;
  file_id: string | null;
  reason: string;
  fixable: boolean;
  fix_action: string;
}

export interface SyncValidationReport {
  ghost_entries: SyncIssue[];
  missing_files: SyncIssue[];
  failed_indexings: SyncIssue[];
  orphaned_zip_entries: SyncIssue[];
  stale_token: SyncIssue[];
  total_issues: number;
}

export const runSyncValidation = async (): Promise<SyncValidationReport> => {
  try {
    return await invoke<SyncValidationReport>("run_sync_validation");
  } catch (error) {
    console.error("Sync validation failed:", error);
    throw error;
  }
};

export const fixSyncIssues = async (
  category: string,
  fileIds: string[],
): Promise<{ category: string; fixed: number; failed: number; total: number }> => {
  try {
    return await invoke("fix_sync_issues", { category, fileIds });
  } catch (error) {
    console.error("Fix sync issues failed:", error);
    throw error;
  }
};

// ==================== OPEN SUBTITLES ====================

export interface SubtitleEntry {
  id: string;
  url: string;
  lang: string;
}

export const fetchSubtitles = async (
  imdbId: string,
  mediaType: string,
): Promise<SubtitleEntry[]> => {
  try {
    return await invoke<SubtitleEntry[]>("fetch_subtitles", { imdbId, mediaType });
  } catch (error) {
    console.error("Failed to fetch subtitles:", error);
    return [];
  }
};

export const downloadSubtitle = async (
  url: string,
  filename: string,
): Promise<string> => {
  try {
    return await invoke<string>("download_subtitle", { url, filename });
  } catch (error) {
    console.error("Failed to download subtitle:", error);
    throw error;
  }
};

// Session-scoped subtitle file path store (per media ID)
const pendingSubtitlePaths = new Map<number, string>()

export const setPendingSubtitlePath = (mediaId: number, path: string): void => {
  pendingSubtitlePaths.set(mediaId, path)
}

export const consumePendingSubtitlePath = (mediaId: number): string | null => {
  const path = pendingSubtitlePaths.get(mediaId) ?? null
  pendingSubtitlePaths.delete(mediaId)
  return path
}
