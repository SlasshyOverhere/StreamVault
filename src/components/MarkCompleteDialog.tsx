import { motion, AnimatePresence } from 'framer-motion'
import { X, CheckCircle, RotateCcw, HelpCircle } from 'lucide-react'

interface MarkCompleteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  seasonEpisode?: string
  progressPercent: number
  onMarkComplete: () => void
  onKeepProgress: () => void
  isCompletionConfirmation?: boolean // True when MPV detected end chapter
}

export function MarkCompleteDialog({
  open,
  onOpenChange,
  title,
  seasonEpisode,
  progressPercent,
  onMarkComplete,
  onKeepProgress,
  isCompletionConfirmation = false
}: MarkCompleteDialogProps) {
  const handleClose = () => {
    onOpenChange(false)
    onKeepProgress()
  }

  return (
    <AnimatePresence>
      {open && (
        <>
          {/* Backdrop */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={handleClose}
            className="fixed inset-0 bg-black/70 backdrop-blur-sm z-[300]"
          />

          {/* Dialog */}
          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 20 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            className="fixed inset-0 flex items-center justify-center z-[301] p-4"
          >
            <div className="bg-card/95 backdrop-blur-2xl border border-white/10 rounded-2xl shadow-2xl w-full max-w-sm overflow-hidden">
              {/* Header */}
              <div className="flex items-center justify-between p-4 border-b border-white/10">
                <div className="flex items-center gap-3">
                  <div className={`p-2 rounded-xl bg-gradient-to-br ${isCompletionConfirmation ? 'from-blue-500/20 to-blue-500/5' : 'from-green-500/20 to-green-500/5'}`}>
                    {isCompletionConfirmation ? (
                      <HelpCircle className="w-5 h-5 text-blue-400" />
                    ) : (
                      <CheckCircle className="w-5 h-5 text-green-400" />
                    )}
                  </div>
                  <div>
                    <h2 className="text-base font-bold text-white">
                      {isCompletionConfirmation ? 'Did you complete your playback?' : 'Almost Finished!'}
                    </h2>
                    <p className="text-xs text-muted-foreground">
                      {isCompletionConfirmation ? 'End of video detected' : `${Math.round(progressPercent)}% watched`}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClose}
                  className="p-2 rounded-lg hover:bg-white/10 transition-colors"
                >
                  <X className="w-4 h-4 text-muted-foreground" />
                </button>
              </div>

              {/* Content */}
              <div className="p-4">
                <p className="text-sm text-muted-foreground mb-4">
                  {isCompletionConfirmation ? (
                    <>
                      You reached the end of <span className="text-white font-medium">{title}</span>
                      {seasonEpisode && <span className="text-white font-medium"> ({seasonEpisode})</span>}.
                      Mark as complete?
                    </>
                  ) : (
                    <>
                      You watched most of <span className="text-white font-medium">{title}</span>
                      {seasonEpisode && <span className="text-white font-medium"> ({seasonEpisode})</span>}.
                      Would you like to mark it as complete?
                    </>
                  )}
                </p>

                <div className="flex flex-col gap-2">
                  <button
                    onClick={() => {
                      onMarkComplete()
                      onOpenChange(false)
                    }}
                    className="w-full py-2.5 px-4 rounded-xl bg-white text-black font-semibold text-sm hover:bg-white/90 transition-colors flex items-center justify-center gap-2"
                  >
                    <CheckCircle className="w-4 h-4" />
                    {isCompletionConfirmation ? 'Yes, Mark Complete' : 'Mark as Complete'}
                  </button>
                  <button
                    onClick={handleClose}
                    className="w-full py-2.5 px-4 rounded-xl bg-white/10 text-white font-medium text-sm hover:bg-white/20 transition-colors flex items-center justify-center gap-2"
                  >
                    <RotateCcw className="w-4 h-4" />
                    {isCompletionConfirmation ? 'No, Keep Progress' : `Keep Progress (${Math.round(progressPercent)}%)`}
                  </button>
                </div>
              </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
