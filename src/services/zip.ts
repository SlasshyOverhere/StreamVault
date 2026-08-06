import { invoke } from "@tauri-apps/api/core";

export interface ZipEpisodeInfo {
  filename: string;
  title: string;
  season: number;
  episode: number;
  size: number;
}

export interface ZipAnalysisResult {
  zipFileId: string;
  filename: string;
  fileSize: number;
  compressionType: "store" | "deflate" | "mixed" | "other";
  totalEntries: number;
  videoEntries: number;
  episodes: ZipEpisodeInfo[];
}

export interface ZipIndexResult {
  indexedCount: number;
  skippedCount: number;
  message: string;
}

export interface ZipStreamInfo {
  zipFileId: string;
  byteStart: number;
  byteEnd: number;
  contentType: string;
}

export const analyzeZip = async (
  zipFileId: string,
): Promise<ZipAnalysisResult> => {
  return invoke<ZipAnalysisResult>("zip_analyze", { zipFileId });
};

export const indexZipEpisodes = async (
  zipFileId: string,
  folderId: string,
): Promise<ZipIndexResult> => {
  return invoke<ZipIndexResult>("zip_index_episodes", { zipFileId, folderId });
};

export const getZipStreamInfo = async (
  mediaId: number,
): Promise<ZipStreamInfo> => {
  return invoke<ZipStreamInfo>("zip_get_stream_info", { mediaId });
};
