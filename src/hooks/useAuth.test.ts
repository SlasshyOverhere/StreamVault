// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'

// Mock the gdrive service before importing useAuth.
vi.mock('@/services/gdrive', () => ({
  isGDriveConnected: vi.fn(),
  getGDriveAuthStatus: vi.fn(),
  startGDriveAuth: vi.fn(),
  completeGDriveAuth: vi.fn(),
  disconnectGDrive: vi.fn(),
}))

import { useAuth } from './useAuth'
import {
  isGDriveConnected,
  getGDriveAuthStatus,
  startGDriveAuth,
  completeGDriveAuth,
} from '@/services/gdrive'

vi.mock('@/services/api', () => ({
  getConfig: vi.fn().mockResolvedValue(null),
  saveConfig: vi.fn().mockResolvedValue(undefined),
  autoDetectMpv: vi.fn().mockResolvedValue(null),
  getBundledMpvInfo: vi.fn().mockResolvedValue({ exists: false }),
}))

vi.mock('@/components/ui/use-toast', () => ({
  useToast: () => ({ toast: vi.fn() }),
}))

beforeEach(() => {
  vi.resetAllMocks()
})

describe('useAuth', () => {
  it('starts as unauthenticated and loading', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(false)
    vi.mocked(getGDriveAuthStatus).mockResolvedValue(null)
    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.isAuthLoading).toBe(false))
    expect(result.current.state).toBe('unauthenticated')
    expect(result.current.isAuthenticated).toBe(false)
    expect(result.current.needsReauth).toBe(false)
  })

  it('reaches "valid" when refresh token is present', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(true)
    vi.mocked(getGDriveAuthStatus).mockResolvedValue({
      has_refresh_token: true,
      has_access_token: true,
    })
    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.state).toBe('valid'))
    expect(result.current.isAuthenticated).toBe(true)
    expect(result.current.needsReauth).toBe(false)
  })

  it('reaches "soft_lost" when refresh token is gone but tokens exist', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(true)
    vi.mocked(getGDriveAuthStatus).mockResolvedValue({
      has_refresh_token: false,
      has_access_token: false,
      last_refresh_error: 'invalid_grant',
    })
    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.state).toBe('soft_lost'))
    expect(result.current.isAuthenticated).toBe(true) // backward-compat boolean
    expect(result.current.needsReauth).toBe(true)
  })

  it('keeps previous state when getGDriveAuthStatus returns null', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(false)
    vi.mocked(getGDriveAuthStatus).mockResolvedValue(null)
    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.isAuthLoading).toBe(false))
    expect(result.current.state).toBe('unauthenticated')
  })

  it('reauth() flips soft_lost -> valid when status reports fresh refresh token', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(true)
    vi.mocked(getGDriveAuthStatus)
      .mockResolvedValueOnce({ has_refresh_token: false, has_access_token: false })
      .mockResolvedValueOnce({ has_refresh_token: true, has_access_token: true })
    vi.mocked(startGDriveAuth).mockResolvedValue('https://oauth.example')
    vi.mocked(completeGDriveAuth).mockResolvedValue({
      email: 'u@example.com',
      display_name: 'u',
      photo_url: null,
      storage_used: null,
      storage_limit: null,
    })

    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.state).toBe('soft_lost'))

    let ok: boolean | undefined
    await act(async () => {
      ok = await result.current.reauth()
    })
    expect(ok).toBe(true)
    await waitFor(() => expect(result.current.state).toBe('valid'))
    expect(result.current.needsReauth).toBe(false)
  })

  it('reauth() returns false when status still shows no refresh token', async () => {
    vi.mocked(isGDriveConnected).mockResolvedValue(true)
    vi.mocked(getGDriveAuthStatus).mockResolvedValue({
      has_refresh_token: false,
      has_access_token: false,
    })
    vi.mocked(startGDriveAuth).mockResolvedValue('https://oauth.example')
    vi.mocked(completeGDriveAuth).mockResolvedValue(null)

    const { result } = renderHook(() => useAuth())
    await waitFor(() => expect(result.current.state).toBe('soft_lost'))

    let ok: boolean | undefined
    await act(async () => {
      ok = await result.current.reauth()
    })
    expect(ok).toBe(false)
    expect(result.current.state).toBe('soft_lost')
  })
})
