import { describe, it, expect, vi } from 'vitest'
import {
  formatStorageSize,
  isFolder,
  isVideoFile,
  isZipFile,
  isCloudPlayableItem,
  parseVideoFilename,
} from './gdrive'
import type { DriveItem } from './gdrive'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

describe('formatStorageSize', () => {
  it('returns "Unknown" for undefined/0', () => {
    expect(formatStorageSize(undefined)).toBe('Unknown')
    expect(formatStorageSize(0)).toBe('Unknown')
  })

  it('formats bytes', () => {
    expect(formatStorageSize(500)).toBe('500.0 B')
  })

  it('formats KB', () => {
    expect(formatStorageSize(1024)).toBe('1.0 KB')
  })

  it('formats MB', () => {
    expect(formatStorageSize(1048576)).toBe('1.0 MB')
  })

  it('formats GB', () => {
    expect(formatStorageSize(1073741824)).toBe('1.0 GB')
  })

  it('formats TB', () => {
    expect(formatStorageSize(1099511627776)).toBe('1.0 TB')
  })
})

const makeItem = (overrides: Partial<DriveItem> = {}): DriveItem => ({
  id: '1',
  name: 'test',
  mimeType: 'video/mp4',
  ...overrides,
})

describe('isFolder', () => {
  it('returns true for folder mime type', () => {
    expect(isFolder(makeItem({ mimeType: 'application/vnd.google-apps.folder' }))).toBe(true)
  })

  it('returns false for non-folder', () => {
    expect(isFolder(makeItem({ mimeType: 'video/mp4' }))).toBe(false)
  })
})

describe('isVideoFile', () => {
  it('returns true for video mime types', () => {
    expect(isVideoFile(makeItem({ mimeType: 'video/mp4' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/x-matroska' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/webm' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/quicktime' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/avi' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/x-m4v' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/x-ms-wmv' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/x-flv' }))).toBe(true)
    expect(isVideoFile(makeItem({ mimeType: 'video/mp2t' }))).toBe(true)
  })

  it('returns false for non-video mime types', () => {
    expect(isVideoFile(makeItem({ mimeType: 'application/pdf' }))).toBe(false)
    expect(isVideoFile(makeItem({ mimeType: 'image/png' }))).toBe(false)
  })
})

describe('isZipFile', () => {
  it('returns true for zip mime types', () => {
    expect(isZipFile(makeItem({ mimeType: 'application/zip' }))).toBe(true)
    expect(isZipFile(makeItem({ mimeType: 'application/x-zip-compressed' }))).toBe(true)
  })

  it('returns true for .zip extension regardless of mime', () => {
    expect(isZipFile(makeItem({ name: 'archive.zip', mimeType: 'application/octet-stream' }))).toBe(true)
    expect(isZipFile(makeItem({ name: 'Archive.ZIP', mimeType: 'application/octet-stream' }))).toBe(true)
  })

  it('returns false for non-zip', () => {
    expect(isZipFile(makeItem({ name: 'video.mp4', mimeType: 'video/mp4' }))).toBe(false)
  })
})

describe('isCloudPlayableItem', () => {
  it('returns true for video files', () => {
    expect(isCloudPlayableItem(makeItem({ mimeType: 'video/mp4' }))).toBe(true)
  })

  it('returns true for zip files', () => {
    expect(isCloudPlayableItem(makeItem({ mimeType: 'application/zip' }))).toBe(true)
  })

  it('returns false for other types', () => {
    expect(isCloudPlayableItem(makeItem({ mimeType: 'application/pdf' }))).toBe(false)
  })
})
