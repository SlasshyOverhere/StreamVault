import { describe, it, expect, vi } from 'vitest'
import { analyzeZip, indexZipEpisodes, getZipStreamInfo } from './zip'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const { invoke } = await import('@tauri-apps/api/core')
const mockInvoke = vi.mocked(invoke)

describe('analyzeZip', () => {
  it('calls invoke with correct args', async () => {
    const result = { zipFileId: 'z1', filename: 'test.zip', fileSize: 100, compressionType: 'deflate', totalEntries: 5, videoEntries: 2, episodes: [] }
    mockInvoke.mockResolvedValueOnce(result)
    expect(await analyzeZip('z1')).toEqual(result)
    expect(mockInvoke).toHaveBeenCalledWith('zip_analyze', { zipFileId: 'z1' })
  })
})

describe('indexZipEpisodes', () => {
  it('calls invoke with correct args', async () => {
    mockInvoke.mockResolvedValueOnce({ indexedCount: 3, skippedCount: 1, message: 'done' })
    const result = await indexZipEpisodes('z1', 'f1')
    expect(result.indexedCount).toBe(3)
    expect(mockInvoke).toHaveBeenCalledWith('zip_index_episodes', { zipFileId: 'z1', folderId: 'f1' })
  })
})

describe('getZipStreamInfo', () => {
  it('calls invoke with correct args', async () => {
    mockInvoke.mockResolvedValueOnce({ zipFileId: 'z1', byteStart: 0, byteEnd: 100, contentType: 'video/mp4' })
    const result = await getZipStreamInfo(42)
    expect(result.zipFileId).toBe('z1')
    expect(mockInvoke).toHaveBeenCalledWith('zip_get_stream_info', { mediaId: 42 })
  })
})
