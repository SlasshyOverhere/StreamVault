import { useEffect } from 'react'
import { Shield } from 'lucide-react'
import * as Dialog from '@radix-ui/react-dialog'

interface ReauthModalProps {
  open: boolean
  onReauth: () => Promise<void> | void
  onClose: () => void
}

export function ReauthModal({ open, onReauth, onClose }: ReauthModalProps) {
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onClose])

  return (
    <Dialog.Root open={open} onOpenChange={(o) => { if (!o) onClose() }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[500] bg-black/60 backdrop-blur-sm" />
        <Dialog.Content
          className="fixed left-1/2 top-1/2 z-[501] w-[min(420px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-white/10 bg-[#111] p-6 text-white shadow-2xl"
        >
          <div className="mb-4 flex items-center gap-3">
            <Shield className="size-6 text-amber-400" />
            <Dialog.Title className="text-lg font-semibold">
              Re-authenticate Google Drive
            </Dialog.Title>
          </div>
          <Dialog.Description className="mb-6 text-sm text-neutral-400">
            Your Drive session has expired and you need to sign in again to
            continue syncing. Your library and current selection are preserved.
          </Dialog.Description>
          <div className="flex justify-end">
            <button
              type="button"
              onClick={() => void onReauth()}
              className="rounded-md bg-white px-4 py-2 text-sm font-medium text-black hover:bg-neutral-200"
            >
              Re-authenticate with Google
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
