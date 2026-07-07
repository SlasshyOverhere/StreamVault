import { useState } from "react"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useToast } from "@/components/ui/use-toast"
import { shareGDriveFile } from "@/services/gdrive"
import { useAuthGuard, AuthCancelledError } from "@/hooks/useAuthGuard"
import { Mail, Send, CheckCircle2, Loader2, Shield } from "lucide-react"
import { cn } from "@/lib/utils"

interface ShareDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  fileId: string
  fileName: string
}

export const ShareDialog = ({ open, onOpenChange, fileId, fileName }: ShareDialogProps) => {
  const [email, setEmail] = useState("")
  const [isSharing, setIsSharing] = useState(false)
  const [shared, setShared] = useState(false)
  const { toast } = useToast()
  // Soft_lost guard: re-auth prompt before letting the share write hit Drive.
  const { guard: guardDriveWrite } = useAuthGuard()

  const handleShare = async () => {
    const trimmedEmail = email.trim()
    if (!trimmedEmail || !trimmedEmail.includes("@")) {
      toast({
        title: "Invalid email",
        description: "Please enter a valid email address.",
        variant: "destructive",
      })
      return
    }

    setIsSharing(true)
    try {
      await guardDriveWrite(() => shareGDriveFile(fileId, trimmedEmail, "reader"))
      setShared(true)
      toast({
        title: "Shared successfully",
        description: `"${fileName}" has been shared with ${trimmedEmail}.`,
      })
    } catch (error) {
      // AuthCancelledError: user dismissed the re-auth modal — no toast,
      // just leave the dialog open so they can retry sharing.
      if (error instanceof AuthCancelledError) {
        return
      }
      console.error('[ShareDialog] Failed to share:', error)
      toast({
        title: "Failed to share",
        description: error instanceof Error ? error.message : "Something went wrong. Please try again.",
        variant: "destructive",
      })
    } finally {
      setIsSharing(false)
    }
  }

  const handleClose = () => {
    setEmail("")
    setShared(false)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) handleClose() }}>
      <DialogContent className="sm:max-w-md bg-[#090909]/95 backdrop-blur-2xl p-0 overflow-hidden border border-white/10 shadow-[0_0_120px_rgba(0,0,0,0.85)]">
        <div className="px-6 pt-6 pb-5">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2.5 text-white text-base">
              <div className="size-8 rounded-xl bg-white/10 flex items-center justify-center">
                <Shield className="size-4.5 text-white" style={{ width: 18, height: 18 }} />
              </div>
              <span>Share via Google Drive</span>
            </DialogTitle>
            <DialogDescription className="text-white/45 text-xs leading-relaxed mt-2">
              Grant access to &ldquo;{fileName}&rdquo; by entering their Google account email.
              They&apos;ll see it in their &ldquo;Shared with me&rdquo; on Google Drive.
            </DialogDescription>
          </DialogHeader>
        </div>

        <div className="px-6 pb-6">
          {shared ? (
            <div className="flex flex-col items-center gap-5 py-8">
              <div className="size-20 rounded-full bg-white/10 flex items-center justify-center ring-1 ring-white/10">
                <CheckCircle2 className="size-10 text-white" />
              </div>
              <p className="text-white/85 text-base font-medium">File shared successfully!</p>
              <Button
                onClick={handleClose}
                className="h-11 px-10 rounded-2xl bg-white/10 hover:bg-white/18 text-white/85 hover:text-white text-sm font-semibold border border-white/8 transition-all"
              >
                Close
              </Button>
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              <div className="relative">
                <Mail className="absolute left-3.5 top-1/2 -translate-y-1/2 size-4 text-white/25" />
                <Input
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="person@example.com"
                  type="email"
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !isSharing) handleShare()
                  }}
                  className={cn(
                    "pl-10 h-12 bg-white/[0.04] border border-white/10 text-white placeholder:text-white/22 text-sm rounded-2xl",
                    "focus-visible:ring-white/25 focus-visible:border-white/35 transition-all",
                    "hover:border-white/18"
                  )}
                />
              </div>
              <p className="text-[11px] text-white/28 leading-relaxed px-0.5">
                The recipient needs a Google account associated with this email to access the file.
              </p>
              <Button
                onClick={handleShare}
                disabled={isSharing || !email.trim()}
                className={cn(
                  "w-full h-12 rounded-2xl gap-2.5 text-sm font-semibold transition-all",
                  "bg-white hover:bg-white/90 text-black",
                  "disabled:opacity-30 disabled:cursor-not-allowed"
                )}
              >
                {isSharing ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Send className="size-4" />
                )}
                {isSharing ? "Sharing..." : "Share"}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
