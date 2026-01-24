import { useState, useEffect } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { X, Sparkles, CheckCircle, Layout, Monitor, Bug, Bell, Palette, Lock, List, MousePointer, AppWindow } from 'lucide-react'

const CURRENT_VERSION = '3.0.4'

interface UpdateNotesModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  isFromSettings?: boolean
}

export function UpdateNotesModal({ open, onOpenChange, isFromSettings = false }: UpdateNotesModalProps) {
  const handleClose = () => {
    if (!isFromSettings) {
      // Mark as shown in localStorage
      markUpdateNotesAsShown()
    }
    onOpenChange(false)
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
            className="fixed inset-0 bg-black/70 backdrop-blur-sm z-[200]"
          />

          {/* Modal */}
          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 20 }}
            transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
            className="fixed inset-0 flex items-center justify-center z-[201] p-4"
          >
            <div className="bg-card/95 backdrop-blur-2xl border border-white/10 rounded-2xl shadow-2xl w-full max-w-lg max-h-[80vh] overflow-hidden flex flex-col">
              {/* Header */}
              <div className="flex items-center justify-between p-5 border-b border-white/10">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-xl bg-gradient-to-br from-white/20 to-white/5">
                    <Sparkles className="w-5 h-5 text-white" />
                  </div>
                  <div>
                    <h2 className="text-lg font-bold text-white">What's New</h2>
                    <p className="text-xs text-muted-foreground">Version {CURRENT_VERSION}</p>
                  </div>
                </div>
                <button
                  onClick={handleClose}
                  className="p-2 rounded-lg hover:bg-white/10 transition-colors"
                >
                  <X className="w-5 h-5 text-muted-foreground" />
                </button>
              </div>

              {/* Content */}
              <div className="flex-1 overflow-y-auto p-5 space-y-4">
                {/* Custom Title Bar */}
                <Section
                  icon={<Monitor className="w-4 h-4" />}
                  title="Custom Title Bar"
                  color="from-blue-500/20 to-blue-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Sleek custom title bar - no more Windows chrome</li>
                    <li>• Drag anywhere on title bar to move window</li>
                    <li>• Custom minimize & close buttons</li>
                  </ul>
                </Section>

                {/* Fixed Window */}
                <Section
                  icon={<Lock className="w-4 h-4" />}
                  title="Locked Window Size"
                  color="from-cyan-500/20 to-cyan-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Fixed resolution at <span className="text-white font-medium">1400x900</span></li>
                    <li>• No resizing or maximizing - optimized layout</li>
                  </ul>
                </Section>

                {/* Redesigned Home */}
                <Section
                  icon={<Layout className="w-4 h-4" />}
                  title="Redesigned Home Tab"
                  color="from-purple-500/20 to-purple-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Large centered hero with bigger search bar</li>
                    <li>• Continue Watching in middle-bottom</li>
                    <li>• Library stats at bottom - no empty space</li>
                    <li>• Hero section stays fixed, never moves</li>
                  </ul>
                </Section>

                {/* List View */}
                <Section
                  icon={<List className="w-4 h-4" />}
                  title="New List View"
                  color="from-green-500/20 to-green-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Grid/List toggle in Google Drive tab</li>
                    <li>• Horizontal cards - see more at a glance</li>
                  </ul>
                </Section>

                {/* Single Instance */}
                <Section
                  icon={<AppWindow className="w-4 h-4" />}
                  title="Single Instance"
                  color="from-indigo-500/20 to-indigo-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Running .exe again brings app from tray</li>
                    <li>• No more duplicate windows</li>
                  </ul>
                </Section>

                {/* Notifications */}
                <Section
                  icon={<Bell className="w-4 h-4" />}
                  title="Better Notifications"
                  color="from-yellow-500/20 to-yellow-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Shows from <span className="text-white font-medium">StreamVault</span> not PowerShell</li>
                  </ul>
                </Section>

                {/* Fresh Look */}
                <Section
                  icon={<Palette className="w-4 h-4" />}
                  title="Fresh Look"
                  color="from-pink-500/20 to-pink-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• New app icon and system tray icon</li>
                    <li>• Right-click menu disabled in production</li>
                    <li>• Patch notes modal with "View Notes" in Settings</li>
                  </ul>
                </Section>

                {/* Bug Fixes */}
                <Section
                  icon={<Bug className="w-4 h-4" />}
                  title="Bug Fixes"
                  color="from-orange-500/20 to-orange-500/5"
                >
                  <ul className="space-y-1 text-sm text-muted-foreground">
                    <li>• Fixed hero section moving with content</li>
                    <li>• Fixed window not draggable</li>
                    <li>• Fixed close button not hiding to tray</li>
                    <li>• Fixed floating controls overlapping</li>
                    <li>• Synced all version numbers</li>
                  </ul>
                </Section>
              </div>

              {/* Footer */}
              <div className="p-5 border-t border-white/10">
                <button
                  onClick={handleClose}
                  className="w-full py-2.5 px-4 rounded-xl bg-white text-black font-semibold text-sm hover:bg-white/90 transition-colors flex items-center justify-center gap-2"
                >
                  <CheckCircle className="w-4 h-4" />
                  Got it!
                </button>
              </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}

// Section component for organizing update notes
function Section({
  icon,
  title,
  color,
  children
}: {
  icon: React.ReactNode
  title: string
  color: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <div className={`p-1.5 rounded-lg bg-gradient-to-br ${color}`}>
          {icon}
        </div>
        <h3 className="text-sm font-semibold text-white">{title}</h3>
      </div>
      <div className="pl-8">
        {children}
      </div>
    </div>
  )
}

// Utility functions for managing update notes state
const UPDATE_NOTES_KEY = 'streamvault_update_notes_shown'

export function getUpdateNotesConfig(): { version: string; shown: boolean } | null {
  try {
    const stored = localStorage.getItem(UPDATE_NOTES_KEY)
    if (stored) {
      return JSON.parse(stored)
    }
  } catch (e) {
    console.error('Failed to read update notes config:', e)
  }
  return null
}

export function shouldShowUpdateNotes(): boolean {
  const config = getUpdateNotesConfig()
  // Show if no config exists, or if version doesn't match current
  if (!config) return true
  if (config.version !== CURRENT_VERSION) return true
  return !config.shown
}

export function markUpdateNotesAsShown(): void {
  try {
    localStorage.setItem(UPDATE_NOTES_KEY, JSON.stringify({
      version: CURRENT_VERSION,
      shown: true
    }))
  } catch (e) {
    console.error('Failed to save update notes config:', e)
  }
}

export function resetUpdateNotesForVersion(version: string): void {
  try {
    localStorage.setItem(UPDATE_NOTES_KEY, JSON.stringify({
      version: version,
      shown: false
    }))
  } catch (e) {
    console.error('Failed to reset update notes config:', e)
  }
}

export const CURRENT_APP_VERSION = CURRENT_VERSION
