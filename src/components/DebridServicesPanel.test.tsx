// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { DebridServicesPanel } from './DebridServicesPanel'

vi.mock('@/services/api', () => ({
  addDebridService: vi.fn(),
  listDebridServices: vi.fn(),
  removeDebridService: vi.fn(),
  setDefaultDebridService: vi.fn(),
}))

import {
  addDebridService,
  listDebridServices,
} from '@/services/api'

const mockAdd = addDebridService as unknown as ReturnType<typeof vi.fn>
const mockList = listDebridServices as unknown as ReturnType<typeof vi.fn>

describe('DebridServicesPanel', () => {
  it('shows configured services after load', async () => {
    mockList.mockResolvedValue([
      {
        kind: 'real_debrid',
        username: 'alice',
        apiKey: '***',
        isDefault: true,
      },
    ])
    render(<DebridServicesPanel />)
    await waitFor(() =>
      expect(screen.getByText(/Signed in as/i)).toBeInTheDocument()
    )
    expect(screen.getByText(/alice/)).toBeInTheDocument()
  })

  it('disables the Add button for kinds that are already configured', async () => {
    mockList.mockResolvedValue([
      { kind: 'real_debrid', username: 'alice', apiKey: '***', isDefault: true },
    ])
    render(<DebridServicesPanel />)
    await waitFor(() => screen.getByText(/alice/))
    // Find Add button (with text "Add" and the icon)
    const addButtons = screen.getAllByRole('button', { name: /Add/i })
    // The Add button at the bottom of the form is the submit; the real_debrid
    // kind is already configured so it should be disabled.
    const submit = addButtons[addButtons.length - 1]
    expect(submit).toBeDisabled()
  })

  it('shows an error when an invalid key is added', async () => {
    mockList.mockResolvedValue([])
    mockAdd.mockRejectedValue(new Error('status 401'))
    render(<DebridServicesPanel />)
    await waitFor(() => screen.getAllByRole('button'))
    const input = screen.getByLabelText(/Real-Debrid API key/i)
    fireEvent.change(input, { target: { value: 'bad-key' } })
    const addButton = screen.getByRole('button', { name: /Add/i })
    fireEvent.click(addButton)
    await waitFor(() =>
      expect(
        screen.getByText(/Couldn't validate that API key/i)
      ).toBeInTheDocument()
    )
  })
})
