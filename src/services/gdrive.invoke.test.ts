import { describe, it, expect, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const { invoke } = await import('@tauri-apps/api/core')
const mockInvoke = vi.mocked(invoke)

beforeEach(() => vi.clearAllMocks())

// Import after mock
const mod = await import('./gdrive')

describe('gdrive invoke wrappers: throw on error', () => {
  const cases: [string, () => Promise<unknown>][] = [
    ['startGDriveAuth', () => mod.startGDriveAuth()],
    ['completeGDriveAuth', () => mod.completeGDriveAuth()],
    ['disconnectGDrive', () => mod.disconnectGDrive()],
    ['listGDriveFolders', () => mod.listGDriveFolders()],
    ['listGDriveFiles', () => mod.listGDriveFiles()],
    ['listGDriveVideoFiles', () => mod.listGDriveVideoFiles('fid')],
    ['getGDriveStreamUrl', () => mod.getGDriveStreamUrl('fid')],
    ['shareGDriveFile', () => mod.shareGDriveFile('fid', 'a@b.com')],
    ['getGDriveFileMetadata', () => mod.getGDriveFileMetadata('fid')],
    ['scanCloudFolder', () => mod.scanCloudFolder('fid', 'name')],
    ['deleteCloudFolderMedia', () => mod.deleteCloudFolderMedia('fid')],
    ['addCloudFolder', () => mod.addCloudFolder('fid', 'name')],
    ['removeCloudFolder', () => mod.removeCloudFolder('fid')],
    ['scanAllCloudFolders', () => mod.scanAllCloudFolders()],
    ['checkCloudChanges', () => mod.checkCloudChanges()],
  ]

  for (const [name, fn] of cases) {
    it(`${name} throws on error`, async () => {
      mockInvoke.mockRejectedValueOnce(new Error('boom'))
      await expect(fn()).rejects.toThrow('boom')
    })
  }
})

describe('gdrive invoke wrappers: return default on error', () => {
  it('isGDriveConnected returns false on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.isGDriveConnected()).toBe(false)
  })

  it('getGDriveAccountInfo returns null on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getGDriveAccountInfo()).toBeNull()
  })

  it('getCloudFolders returns [] on error', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('fail'))
    expect(await mod.getCloudFolders()).toEqual([])
  })
})

describe('gdrive invoke wrappers: success paths', () => {
  it('isGDriveConnected returns true', async () => {
    mockInvoke.mockResolvedValueOnce(true)
    expect(await mod.isGDriveConnected()).toBe(true)
    expect(mockInvoke).toHaveBeenCalledWith('gdrive_is_connected')
  })

  it('getGDriveAccountInfo returns info', async () => {
    mockInvoke.mockResolvedValueOnce({ email: 'a@b.com' })
    const info = await mod.getGDriveAccountInfo()
    expect(info?.email).toBe('a@b.com')
  })

  it('getGDriveAccessToken returns token', async () => {
    mockInvoke.mockResolvedValueOnce('tok123')
    expect(await mod.getGDriveAccessToken()).toBe('tok123')
  })

  it('listGDriveFolders passes parentId', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.listGDriveFolders('pid')
    expect(mockInvoke).toHaveBeenCalledWith('gdrive_list_folders', { parentId: 'pid' })
  })

  it('listGDriveFolders passes null when no parentId', async () => {
    mockInvoke.mockResolvedValueOnce([])
    await mod.listGDriveFolders()
    expect(mockInvoke).toHaveBeenCalledWith('gdrive_list_folders', { parentId: null })
  })

  it('shareGDriveFile passes role', async () => {
    mockInvoke.mockResolvedValueOnce({ success: true, message: 'ok' })
    await mod.shareGDriveFile('fid', 'a@b.com', 'reader')
    expect(mockInvoke).toHaveBeenCalledWith('gdrive_share_file', { fileId: 'fid', email: 'a@b.com', role: 'reader' })
  })

  it('shareGDriveFile defaults role to null', async () => {
    mockInvoke.mockResolvedValueOnce({ success: true, message: 'ok' })
    await mod.shareGDriveFile('fid', 'a@b.com')
    expect(mockInvoke).toHaveBeenCalledWith('gdrive_share_file', { fileId: 'fid', email: 'a@b.com', role: null })
  })

  it('getCloudFolders returns folders', async () => {
    mockInvoke.mockResolvedValueOnce([{ id: '1', name: 'Movies', auto_scan: true }])
    const folders = await mod.getCloudFolders()
    expect(folders).toHaveLength(1)
  })
})
