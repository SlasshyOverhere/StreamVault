// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'
import { useAuthGuard } from './useAuthGuard'

vi.mock('./useAuth', () => ({
  useAuth: vi.fn(),
}))
import { useAuth } from './useAuth'

beforeEach(() => {
  vi.resetAllMocks()
})

describe('useAuthGuard', () => {
  it('runs immediately when state is "valid"', async () => {
    vi.mocked(useAuth).mockReturnValue({
      state: 'valid',
      needsReauth: false,
      reauth: vi.fn(),
    } as any)

    const { result } = renderHook(() => useAuthGuard())
    const fn = vi.fn().mockResolvedValue('ok')
    const out = await result.current.guard(fn)
    expect(fn).toHaveBeenCalledOnce()
    expect(out).toBe('ok')
  })

  it('rejects when state is "unauthenticated"', async () => {
    vi.mocked(useAuth).mockReturnValue({
      state: 'unauthenticated',
      needsReauth: false,
      reauth: vi.fn(),
    } as any)

    const { result } = renderHook(() => useAuthGuard())
    const fn = vi.fn().mockResolvedValue('ok')
    await expect(result.current.guard(fn)).rejects.toThrow('not_authenticated')
    expect(fn).not.toHaveBeenCalled()
  })

  it('awaits reauth and then runs the function when state is "soft_lost"', async () => {
    const reauth = vi.fn().mockResolvedValue(true)
    vi.mocked(useAuth)
      .mockReturnValueOnce({ state: 'soft_lost', needsReauth: true, reauth } as any)
      .mockReturnValue({ state: 'valid', needsReauth: false, reauth } as any)

    const { result } = renderHook(() => useAuthGuard())
    const fn = vi.fn().mockResolvedValue('ok')

    const promise = result.current.guard(fn)
    await waitFor(() => expect(reauth).toHaveBeenCalled())
    await act(async () => {
      const out = await promise
      expect(out).toBe('ok')
    })
    expect(fn).toHaveBeenCalledOnce()
  })

  it('rejects when reauth returns false (user closed modal)', async () => {
    const reauth = vi.fn().mockResolvedValue(false)
    vi.mocked(useAuth).mockReturnValue({
      state: 'soft_lost',
      needsReauth: true,
      reauth,
    } as any)

    const { result } = renderHook(() => useAuthGuard())
    const fn = vi.fn()
    await expect(result.current.guard(fn)).rejects.toThrow('reauth_cancelled')
    expect(fn).not.toHaveBeenCalled()
  })
})
