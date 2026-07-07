// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { StremioAddonSetupCard } from './StremioAddonSetupCard'

vi.mock('@/services/api', () => ({
  addStremioAddon: vi.fn(),
}))

import { addStremioAddon } from '@/services/api'
const mockAdd = addStremioAddon as unknown as ReturnType<typeof vi.fn>

describe('StremioAddonSetupCard', () => {
  it('disables the install button until a valid URL is entered', () => {
    render(<StremioAddonSetupCard onInstalled={() => {}} onError={() => {}} />)
    const button = screen.getByRole('button', { name: /Fetch & Install/i })
    expect(button).toBeDisabled()
  })

  it('enables the button when a valid https URL is typed', () => {
    render(<StremioAddonSetupCard onInstalled={() => {}} onError={() => {}} />)
    const input = screen.getByLabelText(/Stremio addon URL/i)
    fireEvent.change(input, { target: { value: 'https://addon.example.com/manifest.json' } })
    const button = screen.getByRole('button', { name: /Fetch & Install/i })
    expect(button).not.toBeDisabled()
  })

  it('rejects non-http URLs', () => {
    render(<StremioAddonSetupCard onInstalled={() => {}} onError={() => {}} />)
    const input = screen.getByLabelText(/Stremio addon URL/i)
    fireEvent.change(input, { target: { value: 'ftp://nope' } })
    const button = screen.getByRole('button', { name: /Fetch & Install/i })
    expect(button).toBeDisabled()
  })

  it('calls addStremioAddon with the URL on click', async () => {
    mockAdd.mockResolvedValue({
      id: 'x',
      url: 'https://addon.example.com/manifest.json',
      name: 'Example',
      version: '1.0.0',
      types: ['movie'],
      resources: ['stream'],
      idPrefixes: [],
      catalogs: [],
      status: 'available',
      installedAt: new Date().toISOString(),
      lastCheckedAt: new Date().toISOString(),
    })
    const onInstalled = vi.fn()
    render(<StremioAddonSetupCard onInstalled={onInstalled} onError={() => {}} />)
    fireEvent.change(screen.getByLabelText(/Stremio addon URL/i), {
      target: { value: 'https://addon.example.com/manifest.json' },
    })
    fireEvent.click(screen.getByRole('button', { name: /Fetch & Install/i }))
    await waitFor(() => expect(mockAdd).toHaveBeenCalledWith('https://addon.example.com/manifest.json'))
    await waitFor(() => expect(onInstalled).toHaveBeenCalled())
  })

  it('translates an unreachable error to a friendly toast', async () => {
    mockAdd.mockRejectedValue(new Error('could not reach the addons manifest: status 500'))
    const onError = vi.fn()
    render(<StremioAddonSetupCard onInstalled={() => {}} onError={onError} />)
    fireEvent.change(screen.getByLabelText(/Stremio addon URL/i), {
      target: { value: 'https://addon.example.com/manifest.json' },
    })
    fireEvent.click(screen.getByRole('button', { name: /Fetch & Install/i }))
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(
        expect.stringContaining("Couldn't reach the addon's manifest")
      )
    )
  })
})
