import { invoke } from "@tauri-apps/api/tauri";

// ==================== Types ====================

export interface DriveAccountInfo {
  email: string;
  display_name?: string;
  photo_url?: string;
  storage_used?: number;
  storage_limit?: number;
}

export interface DriveItem {
  id: string;
  name: string;
  mimeType: string;
  size?: string;
  modifiedTime?: string;
  parents?: string[];
  webContentLink?: string;
}

export interface DriveListResponse {
  files: DriveItem[];
  nextPageToken?: string;
}

// ==================== Connection Status ====================

/**
 * Check if user is connected to Google Drive
 */
export const isGDriveConnected = async (): Promise<boolean> => {
  try {
    return await invoke<boolean>("gdrive_is_connected");
  } catch (error) {
    console.error("[GDrive] Failed to check connection:", error);
    return false;
  }
};

export interface GDriveAuthStatus {
  has_refresh_token: boolean;
  has_access_token: boolean;
  last_refresh_error?: string | null;
  last_refresh_failed_at?: string | null;
  consecutive_failures?: number;
}

/**
 * Lightweight snapshot of the backend OAuth state — distinguishes
 * "refresh lost but library readable" from "fully logged out."
 * Returns null if the backend command itself fails (e.g. Tauri runtime
 * unavailable). Callers should treat null as "keep current UI state."
 */
export const getGDriveAuthStatus = async (): Promise<GDriveAuthStatus | null> => {
  try {
    return await invoke<GDriveAuthStatus>("gdrive_get_auth_status");
  } catch (error) {
    console.warn("[GDrive] gdrive_get_auth_status failed", error);
    return null;
  }
};

/**
 * Get current Google Drive access token.
 * Tauri backend refreshes automatically when needed.
 */
export const getGDriveAccessToken = async (): Promise<string> => {
  try {
    return await invoke<string>("gdrive_get_access_token");
  } catch (error) {
    console.error("[GDrive] Failed to get access token:", error);
    throw error;
  }
};

/**
 * Get Google Drive account info (email, name, storage)
 */
export const getGDriveAccountInfo =
  async (): Promise<DriveAccountInfo | null> => {
    try {
      return await invoke<DriveAccountInfo>("gdrive_get_account_info");
    } catch (error) {
      console.error("[GDrive] Failed to get account info:", error);
      return null;
    }
  };

// ==================== Authentication ====================

/**
 * Start Google Drive OAuth flow
 * Opens browser for user to login and authorize
 * Returns the auth URL (browser is opened automatically)
 */
export const startGDriveAuth = async (): Promise<string> => {
  try {
    return await invoke<string>("gdrive_start_auth");
  } catch (error) {
    console.error("[GDrive] Failed to start auth:", error);
    throw error;
  }
};

/**
 * Complete the OAuth flow after user authorizes
 * This waits for the OAuth callback and exchanges the code for tokens
 * Returns account info on success
 */
export const completeGDriveAuth = async (): Promise<DriveAccountInfo> => {
  try {
    return await invoke<DriveAccountInfo>("gdrive_complete_auth");
  } catch (error) {
    console.error("[GDrive] Failed to complete auth:", error);
    throw error;
  }
};

/**
 * Disconnect from Google Drive (revoke access)
 */
export const disconnectGDrive = async (): Promise<void> => {
  try {
    await invoke("gdrive_disconnect");
  } catch (error) {
    console.error("[GDrive] Failed to disconnect:", error);
    throw error;
  }
};

// ==================== File Operations ====================

/**
 * List folders in Google Drive
 * @param parentId Parent folder ID (null for root)
 */
export const listGDriveFolders = async (
  parentId?: string,
): Promise<DriveItem[]> => {
  try {
    return await invoke<DriveItem[]>("gdrive_list_folders", {
      parentId: parentId || null,
    });
  } catch (error) {
    console.error("[GDrive] Failed to list folders:", error);
    throw error;
  }
};

/**
 * List all files in a folder
 * @param folderId Folder ID (null for root)
 */
export const listGDriveFiles = async (
  folderId?: string,
): Promise<DriveListResponse> => {
  try {
    return await invoke<DriveListResponse>("gdrive_list_files", {
      folderId: folderId || null,
    });
  } catch (error) {
    console.error("[GDrive] Failed to list files:", error);
    throw error;
  }
};

/**
 * List video files in a folder
 * @param folderId Folder ID to scan
 * @param recursive Whether to scan subfolders
 */
export const listGDriveVideoFiles = async (
  folderId: string,
  recursive: boolean = true,
): Promise<DriveItem[]> => {
  try {
    return await invoke<DriveItem[]>("gdrive_list_video_files", {
      folderId,
      recursive,
    });
  } catch (error) {
    console.error("[GDrive] Failed to list video files:", error);
    throw error;
  }
};

/**
 * Get streaming URL and auth token for a file
 * Returns [url, accessToken] tuple
 */
export const getGDriveStreamUrl = async (
  fileId: string,
): Promise<[string, string]> => {
  try {
    return await invoke<[string, string]>("gdrive_get_stream_url", { fileId });
  } catch (error) {
    console.error("[GDrive] Failed to get stream URL:", error);
    throw error;
  }
};

/**
 * Share a file with a user by email (Google Drive native share)
 */
export interface ShareResult {
  success: boolean;
  message: string;
}

export const shareGDriveFile = async (
  fileId: string,
  email: string,
  role?: string,
): Promise<ShareResult> => {
  try {
    return await invoke<ShareResult>("gdrive_share_file", { fileId, email, role: role ?? null });
  } catch (error) {
    console.error("[GDrive] Failed to share file:", error);
    throw error;
  }
};

/**
 * Get file metadata
 */
export const getGDriveFileMetadata = async (
  fileId: string,
): Promise<DriveItem> => {
  try {
    return await invoke<DriveItem>("gdrive_get_file_metadata", { fileId });
  } catch (error) {
    console.error("[GDrive] Failed to get file metadata:", error);
    throw error;
  }
};

// ==================== Helpers ====================

/**
 * Format bytes to human readable string
 */
export const formatStorageSize = (bytes?: number): string => {
  if (!bytes) return "Unknown";

  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = bytes;
  let unitIndex = 0;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex++;
  }

  return `${size.toFixed(1)} ${units[unitIndex]}`;
};

/**
 * Check if a Drive item is a folder
 */
export const isFolder = (item: DriveItem): boolean => {
  return item.mimeType === "application/vnd.google-apps.folder";
};

/**
 * Check if a Drive item is a video file
 */
export const isVideoFile = (item: DriveItem): boolean => {
  const videoMimeTypes = [
    "video/mp4",
    "video/x-matroska",
    "video/avi",
    "video/quicktime",
    "video/webm",
    "video/x-m4v",
    "video/x-ms-wmv",
    "video/x-flv",
    "video/mp2t",
  ];
  return videoMimeTypes.includes(item.mimeType);
};

export const isZipFile = (item: DriveItem): boolean => {
  const zipMimeTypes = ["application/zip", "application/x-zip-compressed"];
  return (
    zipMimeTypes.includes(item.mimeType) ||
    item.name.toLowerCase().endsWith(".zip")
  );
};

export const isCloudPlayableItem = (item: DriveItem): boolean => {
  return isVideoFile(item) || isZipFile(item);
};

/**
 * Parse video filename to extract title, year, season, episode
 */
export const parseVideoFilename = (
  filename: string,
): {
  title: string;
  year?: number;
  season?: number;
  episode?: number;
} => {
  // Remove file extension
  const nameWithoutExt = filename.replace(/\.[^/.]+$/, "");

  // Try to match TV show pattern: ShowName S01E01 or ShowName 1x01
  const tvMatch = nameWithoutExt.match(
    /^(.+?)[.\s_-]+(?:[Ss](\d{1,2})[Ee](\d{1,2})|(\d{1,2})[xX](\d{1,2}))/,
  );
  if (tvMatch) {
    const season = tvMatch[2] ? parseInt(tvMatch[2]) : parseInt(tvMatch[4]);
    const episode = tvMatch[3] ? parseInt(tvMatch[3]) : parseInt(tvMatch[5]);
    return {
      title: tvMatch[1].replace(/[._]/g, " ").trim(),
      season,
      episode,
    };
  }

  // Try to match movie pattern: MovieName (2020) or MovieName.2020
  const movieMatch = nameWithoutExt.match(/^(.+?)[.\s_-]+[([]?(\d{4})[)\]]?/);
  if (movieMatch) {
    return {
      title: movieMatch[1].replace(/[._]/g, " ").trim(),
      year: parseInt(movieMatch[2]),
    };
  }

  // Fallback: just clean up the filename
  return {
    title: nameWithoutExt.replace(/[._]/g, " ").trim(),
  };
};

// ==================== Cloud Indexing ====================

export interface CloudScanResult {
  success: boolean;
  indexed_count: number;
  skipped_count: number;
  movies_count: number;
  tv_count: number;
  message: string;
  skipped_reasons?: string[];
  // Aliases for convenience
  indexed?: number;
  skipped?: number;
  movies?: number;
  tv?: number;
}

/**
 * Scan a cloud folder and index its media content
 * Auto-detects movies vs TV shows based on filename patterns
 * @param folderId Google Drive folder ID
 * @param folderName Display name of the folder
 */
export const scanCloudFolder = async (
  folderId: string,
  folderName: string,
): Promise<CloudScanResult> => {
  try {
    return await invoke<CloudScanResult>("gdrive_scan_folder", {
      folderId,
      folderName,
    });
  } catch (error) {
    console.error("[GDrive] Failed to scan folder:", error);
    throw error;
  }
};

/**
 * Delete all indexed media from a cloud folder
 * @param folderId Google Drive folder ID
 */
export const deleteCloudFolderMedia = async (
  folderId: string,
): Promise<{ message: string }> => {
  try {
    return await invoke<{ message: string }>("gdrive_delete_folder_media", {
      folderId,
    });
  } catch (error) {
    console.error("[GDrive] Failed to delete folder media:", error);
    throw error;
  }
};

// ==================== Cloud Folder Management ====================

export interface CloudFolder {
  id: string;
  name: string;
  auto_scan: boolean;
}

/**
 * Add a cloud folder to track (stored in database, auto-scanned)
 */
export const addCloudFolder = async (
  folderId: string,
  folderName: string,
): Promise<{ message: string }> => {
  try {
    return await invoke<{ message: string }>("add_cloud_folder", {
      folderId,
      folderName,
    });
  } catch (error) {
    console.error("[GDrive] Failed to add cloud folder:", error);
    throw error;
  }
};

/**
 * Remove a cloud folder from tracking (also deletes indexed media)
 */
export const removeCloudFolder = async (
  folderId: string,
): Promise<{ message: string }> => {
  try {
    return await invoke<{ message: string }>("remove_cloud_folder", {
      folderId,
    });
  } catch (error) {
    console.error("[GDrive] Failed to remove cloud folder:", error);
    throw error;
  }
};

/**
 * Get all tracked cloud folders
 */
export const getCloudFolders = async (): Promise<CloudFolder[]> => {
  try {
    return await invoke<CloudFolder[]>("get_cloud_folders");
  } catch (error) {
    console.error("[GDrive] Failed to get cloud folders:", error);
    return [];
  }
};

/**
 * Scan all cloud folders for new files
 */
export const scanAllCloudFolders = async (): Promise<CloudScanResult> => {
  try {
    return await invoke<CloudScanResult>("scan_all_cloud_folders");
  } catch (error) {
    console.error("[GDrive] Failed to scan all cloud folders:", error);
    throw error;
  }
};

/**
 * Check for new cloud files using the efficient Changes API
 * This is much lighter than full scanning - only returns delta changes
 */
export const checkCloudChanges = async (): Promise<CloudScanResult> => {
  try {
    return await invoke<CloudScanResult>("check_cloud_changes");
  } catch (error) {
    console.error("[GDrive] Failed to check cloud changes:", error);
    throw error;
  }
};
