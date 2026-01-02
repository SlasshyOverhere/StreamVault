import { useState, useEffect } from "react"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Button } from "@/components/ui/button"
import {
  Plus, Trash2, MonitorPlay, ExternalLink, HelpCircle, FolderOpen,
  AlertTriangle, Settings, Film, Key, Zap, Power, X, Save, RefreshCw
} from "lucide-react"
import {
  Config, getConfig, saveConfig, scanLibrary, getPlayerPreference,
  setPlayerPreference, PlayerPreference, clearAllAppData
} from "@/services/api"
import { useToast } from "@/components/ui/use-toast"
import { open as openDialog } from '@tauri-apps/api/dialog'
import { invoke } from '@tauri-apps/api/tauri'
import { Switch } from "@/components/ui/switch"
import { motion, AnimatePresence } from "framer-motion"
import { cn } from "@/lib/utils"

interface SettingsModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

type SettingsSection = 'general' | 'library' | 'player' | 'api' | 'danger'

export function SettingsModal({ open, onOpenChange }: SettingsModalProps) {
  const [config, setConfig] = useState<Config>({
    mpv_path: "",
    ffprobe_path: "",
    media_folders: [],
    tmdb_api_key: ""
  })
  const [playerPref, setPlayerPref] = useState<PlayerPreference>('ask')
  const [loading, setLoading] = useState(false)
  const [autoStart, setAutoStart] = useState(false)
  const [showResetConfirm, setShowResetConfirm] = useState(false)
  const [resetting, setResetting] = useState(false)
  const [activeSection, setActiveSection] = useState<SettingsSection>('general')
  const { toast } = useToast()

  useEffect(() => {
    if (open) {
      loadConfig()
      setPlayerPref(getPlayerPreference())
      checkAutoStart()
      setActiveSection('general')
      setShowResetConfirm(false)
    }
  }, [open])

  const checkAutoStart = async () => {
    try {
      const enabled = await invoke<boolean>('plugin:autostart|is_enabled')
      setAutoStart(enabled)
    } catch (error) {
      console.error("Failed to check autostart", error)
    }
  }

  const toggleAutoStart = async (checked: boolean) => {
    try {
      if (checked) {
        await invoke('plugin:autostart|enable')
        toast({ title: "Auto Startup Enabled", description: "Slasshy will now start automatically." })
      } else {
        await invoke('plugin:autostart|disable')
        toast({ title: "Auto Startup Disabled", description: "Slasshy will not start automatically." })
      }
      setAutoStart(checked)
    } catch (error) {
      console.error("Failed to toggle autostart", error)
      toast({ title: "Error", description: "Failed to update startup settings", variant: "destructive" })
    }
  }

  const loadConfig = async () => {
    try {
      const data = await getConfig()
      setConfig({
        mpv_path: data.mpv_path || "",
        ffprobe_path: data.ffprobe_path || "",
        media_folders: data.media_folders || [],
        tmdb_api_key: data.tmdb_api_key || ""
      })
    } catch (error) {
      console.error("Failed to load config", error)
      toast({ title: "Error", description: "Failed to load configuration", variant: "destructive" })
    }
  }

  const handleSave = async () => {
    setLoading(true)
    try {
      await saveConfig(config)
      setPlayerPreference(playerPref)
      toast({ title: "Success", description: "Settings saved successfully" })
      onOpenChange(false)
    } catch (error) {
      console.error("Failed to save config", error)
      toast({ title: "Error", description: "Failed to save settings", variant: "destructive" })
    } finally {
      setLoading(false)
    }
  }

  const handleScan = async () => {
    try {
      await scanLibrary()
      toast({ title: "Scan Started", description: "Library scan has been initiated in the background." })
    } catch (error) {
      toast({ title: "Error", description: "Failed to start scan", variant: "destructive" })
    }
  }

  const handleResetApp = async () => {
    setResetting(true)
    try {
      await clearAllAppData()
      toast({
        title: "App Reset Complete",
        description: "All data has been cleared. The app is now like new."
      })
      setShowResetConfirm(false)
      onOpenChange(false)
      window.location.reload()
    } catch (error) {
      console.error("Failed to reset app", error)
      toast({
        title: "Error",
        description: "Failed to reset app data",
        variant: "destructive"
      })
    } finally {
      setResetting(false)
    }
  }

  const addFolder = () => {
    setConfig({ ...config, media_folders: [...config.media_folders, ""] })
  }

  const removeFolder = (index: number) => {
    const newFolders = [...config.media_folders]
    newFolders.splice(index, 1)
    setConfig({ ...config, media_folders: newFolders })
  }

  const updateFolder = (index: number, value: string) => {
    const newFolders = [...config.media_folders]
    newFolders[index] = value
    setConfig({ ...config, media_folders: newFolders })
  }

  const browseFolder = async (index: number) => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: 'Select Media Folder'
      })
      if (selected && typeof selected === 'string') {
        updateFolder(index, selected)
      }
    } catch (error) {
      console.error("Failed to open folder dialog", error)
    }
  }

  const browseMpvPath = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: 'Executable', extensions: ['exe'] }],
        title: 'Select MPV Executable'
      })
      if (selected && typeof selected === 'string') {
        setConfig({ ...config, mpv_path: selected })
      }
    } catch (error) {
      console.error("Failed to open file dialog", error)
    }
  }

  const browseFfprobePath = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: 'Executable', extensions: ['exe'] }],
        title: 'Select FFprobe Executable'
      })
      if (selected && typeof selected === 'string') {
        setConfig({ ...config, ffprobe_path: selected })
      }
    } catch (error) {
      console.error("Failed to open file dialog", error)
    }
  }

  const sections: { id: SettingsSection; label: string; icon: React.ReactNode }[] = [
    { id: 'general', label: 'General', icon: <Settings className="w-4 h-4" /> },
    { id: 'library', label: 'Library', icon: <Film className="w-4 h-4" /> },
    { id: 'player', label: 'Player', icon: <MonitorPlay className="w-4 h-4" /> },
    { id: 'api', label: 'API Keys', icon: <Key className="w-4 h-4" /> },
    { id: 'danger', label: 'Advanced', icon: <AlertTriangle className="w-4 h-4" /> },
  ]

  const playerOptions: { value: PlayerPreference; label: string; description: string; icon: React.ReactNode }[] = [
    { value: 'ask', label: 'Always Ask', description: 'Choose player each time', icon: <HelpCircle className="h-5 w-5" /> },
    { value: 'mpv', label: 'MPV', description: 'External player (recommended)', icon: <ExternalLink className="h-5 w-5" /> },
    { value: 'builtin', label: 'Built-in', description: 'In-app player', icon: <MonitorPlay className="h-5 w-5" /> },
  ]

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-4xl h-[80vh] p-0 gap-0 overflow-hidden">
        <div className="flex h-full">
          {/* Sidebar */}
          <div className="w-56 flex-shrink-0 bg-card/50 border-r border-border p-4">
            <div className="flex items-center justify-between mb-6">
              <h2 className="text-lg font-semibold text-foreground">Settings</h2>
              <button
                onClick={() => onOpenChange(false)}
                className="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <nav className="space-y-1">
              {sections.map((section) => (
                <button
                  key={section.id}
                  onClick={() => setActiveSection(section.id)}
                  className={cn(
                    "w-full flex items-center gap-3 px-3 py-2.5 rounded-xl transition-all duration-200 text-left",
                    activeSection === section.id
                      ? "bg-primary/10 text-primary"
                      : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                  )}
                >
                  {section.icon}
                  <span className="text-sm font-medium">{section.label}</span>
                </button>
              ))}
            </nav>
          </div>

          {/* Content */}
          <div className="flex-1 flex flex-col overflow-hidden">
            {/* Content Area */}
            <div className="flex-1 overflow-y-auto p-6">
              <AnimatePresence mode="wait">
                {/* General Section */}
                {activeSection === 'general' && (
                  <motion.div
                    key="general"
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    className="space-y-6"
                  >
                    <div>
                      <h3 className="text-lg font-semibold text-foreground mb-1">General Settings</h3>
                      <p className="text-sm text-muted-foreground">Configure general app behavior</p>
                    </div>

                    {/* Auto Start */}
                    <div className="p-4 rounded-xl bg-card border border-border">
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          <div className="p-2 rounded-lg bg-primary/10">
                            <Power className="w-5 h-5 text-primary" />
                          </div>
                          <div>
                            <Label className="text-base font-medium">Run on Startup</Label>
                            <p className="text-sm text-muted-foreground">
                              Automatically start Slasshy when you log in
                            </p>
                          </div>
                        </div>
                        <Switch checked={autoStart} onCheckedChange={toggleAutoStart} />
                      </div>
                    </div>
                  </motion.div>
                )}

                {/* Library Section */}
                {activeSection === 'library' && (
                  <motion.div
                    key="library"
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    className="space-y-6"
                  >
                    <div>
                      <h3 className="text-lg font-semibold text-foreground mb-1">Media Library</h3>
                      <p className="text-sm text-muted-foreground">Configure folders to scan for media</p>
                    </div>

                    {/* Media Folders */}
                    <div className="space-y-3">
                      <Label className="text-sm font-medium">Media Folders</Label>
                      {config.media_folders.length === 0 && (
                        <div className="p-8 rounded-xl border border-dashed border-border text-center">
                          <FolderOpen className="w-10 h-10 text-muted-foreground mx-auto mb-3" />
                          <p className="text-sm text-muted-foreground mb-3">No media folders configured</p>
                          <Button variant="outline" size="sm" onClick={addFolder}>
                            <Plus className="w-4 h-4 mr-2" /> Add Folder
                          </Button>
                        </div>
                      )}

                      {config.media_folders.map((folder, index) => (
                        <div key={index} className="flex gap-2">
                          <Input
                            value={folder}
                            onChange={(e) => updateFolder(index, e.target.value)}
                            placeholder="Path to media folder"
                            className="flex-1"
                          />
                          <Button variant="outline" size="icon" onClick={() => browseFolder(index)}>
                            <FolderOpen className="h-4 w-4" />
                          </Button>
                          <Button variant="destructive" size="icon" onClick={() => removeFolder(index)}>
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      ))}

                      {config.media_folders.length > 0 && (
                        <Button variant="outline" size="sm" onClick={addFolder} className="mt-2">
                          <Plus className="mr-2 h-4 w-4" /> Add Another Folder
                        </Button>
                      )}
                    </div>

                    {/* Scan Button */}
                    <div className="p-4 rounded-xl bg-primary/5 border border-primary/20">
                      <div className="flex items-center justify-between">
                        <div>
                          <h4 className="font-medium text-foreground">Scan Library</h4>
                          <p className="text-sm text-muted-foreground">Scan folders for new media files</p>
                        </div>
                        <Button onClick={handleScan} className="gap-2">
                          <RefreshCw className="w-4 h-4" />
                          Scan Now
                        </Button>
                      </div>
                    </div>
                  </motion.div>
                )}

                {/* Player Section */}
                {activeSection === 'player' && (
                  <motion.div
                    key="player"
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    className="space-y-6"
                  >
                    <div>
                      <h3 className="text-lg font-semibold text-foreground mb-1">Player Settings</h3>
                      <p className="text-sm text-muted-foreground">Configure video playback preferences</p>
                    </div>

                    {/* Default Player */}
                    <div className="space-y-3">
                      <Label className="text-sm font-medium">Default Player</Label>
                      <div className="grid grid-cols-3 gap-3">
                        {playerOptions.map((option) => (
                          <button
                            key={option.value}
                            onClick={() => setPlayerPref(option.value)}
                            className={cn(
                              "p-4 rounded-xl border-2 transition-all duration-200 flex flex-col items-center gap-3 text-center",
                              playerPref === option.value
                                ? "border-primary bg-primary/10 text-primary"
                                : "border-border hover:border-primary/50 hover:bg-muted/50"
                            )}
                          >
                            <div className={cn(
                              "p-3 rounded-lg",
                              playerPref === option.value ? "bg-primary/20" : "bg-muted"
                            )}>
                              {option.icon}
                            </div>
                            <div>
                              <div className="font-medium text-sm">{option.label}</div>
                              <div className="text-xs text-muted-foreground">{option.description}</div>
                            </div>
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* MPV Path */}
                    <div className="space-y-2">
                      <Label className="text-sm font-medium">MPV Executable Path</Label>
                      <div className="flex gap-2">
                        <Input
                          value={config.mpv_path || ""}
                          onChange={(e) => setConfig({ ...config, mpv_path: e.target.value })}
                          placeholder="C:\path\to\mpv.exe"
                          className="flex-1"
                        />
                        <Button variant="outline" size="icon" onClick={browseMpvPath}>
                          <FolderOpen className="h-4 w-4" />
                        </Button>
                      </div>
                      <p className="text-xs text-muted-foreground">Required for external player functionality</p>
                    </div>

                    {/* FFprobe Path */}
                    <div className="space-y-2">
                      <Label className="text-sm font-medium">FFprobe Path (Optional)</Label>
                      <div className="flex gap-2">
                        <Input
                          value={config.ffprobe_path || ""}
                          onChange={(e) => setConfig({ ...config, ffprobe_path: e.target.value })}
                          placeholder="C:\path\to\ffprobe.exe"
                          className="flex-1"
                        />
                        <Button variant="outline" size="icon" onClick={browseFfprobePath}>
                          <FolderOpen className="h-4 w-4" />
                        </Button>
                      </div>
                      <p className="text-xs text-muted-foreground">Used for generating accurate progress bars</p>
                    </div>
                  </motion.div>
                )}

                {/* API Section */}
                {activeSection === 'api' && (
                  <motion.div
                    key="api"
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    className="space-y-6"
                  >
                    <div>
                      <h3 className="text-lg font-semibold text-foreground mb-1">API Configuration</h3>
                      <p className="text-sm text-muted-foreground">Configure external service API keys</p>
                    </div>

                    {/* TMDB API Key/Token */}
                    <div className="p-4 rounded-xl bg-card border border-border space-y-4">
                      <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-blue-500/10">
                          <Zap className="w-5 h-5 text-blue-500" />
                        </div>
                        <div>
                          <Label className="text-base font-medium">TMDB API Key / Access Token</Label>
                          <p className="text-sm text-muted-foreground">Required for metadata, posters, and streaming search</p>
                        </div>
                      </div>
                      <Input
                        type="password"
                        value={config.tmdb_api_key || ""}
                        onChange={(e) => setConfig({ ...config, tmdb_api_key: e.target.value })}
                        placeholder="Enter your TMDB API key or Access Token"
                      />
                      <p className="text-xs text-muted-foreground">
                        You can use either an <strong>API Key</strong> (v3 auth) or <strong>Access Token</strong> (v4 auth / Bearer token).{" "}
                        Get yours at{" "}
                        <a
                          href="https://www.themoviedb.org/settings/api"
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-primary hover:underline"
                        >
                          themoviedb.org
                        </a>
                      </p>
                    </div>
                  </motion.div>
                )}

                {/* Danger Section */}
                {activeSection === 'danger' && (
                  <motion.div
                    key="danger"
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    className="space-y-6"
                  >
                    <div>
                      <h3 className="text-lg font-semibold text-foreground mb-1">Advanced Settings</h3>
                      <p className="text-sm text-muted-foreground">Danger zone - proceed with caution</p>
                    </div>

                    {/* Reset App */}
                    <div className="p-4 rounded-xl border border-destructive/30 bg-destructive/5 space-y-4">
                      <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-destructive/20">
                          <AlertTriangle className="w-5 h-5 text-destructive" />
                        </div>
                        <div>
                          <Label className="text-base font-medium text-destructive">Reset Application</Label>
                          <p className="text-sm text-muted-foreground">
                            Delete all data and start fresh
                          </p>
                        </div>
                      </div>
                      <p className="text-sm text-muted-foreground">
                        This will permanently delete your library data, watch history, streaming history,
                        cached posters, and all settings. This action cannot be undone.
                      </p>

                      {!showResetConfirm ? (
                        <Button
                          variant="destructive"
                          onClick={() => setShowResetConfirm(true)}
                          className="w-full"
                        >
                          <Trash2 className="mr-2 h-4 w-4" />
                          Reset App to Factory State
                        </Button>
                      ) : (
                        <div className="space-y-3 p-4 rounded-lg bg-destructive/10 border border-destructive/30">
                          <p className="text-sm font-medium text-destructive text-center">
                            Are you absolutely sure? This will delete everything!
                          </p>
                          <div className="flex gap-2">
                            <Button
                              variant="outline"
                              onClick={() => setShowResetConfirm(false)}
                              className="flex-1"
                              disabled={resetting}
                            >
                              Cancel
                            </Button>
                            <Button
                              variant="destructive"
                              onClick={handleResetApp}
                              className="flex-1"
                              disabled={resetting}
                            >
                              {resetting ? "Resetting..." : "Yes, Delete Everything"}
                            </Button>
                          </div>
                        </div>
                      )}
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>
            </div>

            {/* Footer */}
            <div className="flex-shrink-0 p-4 border-t border-border bg-card/50">
              <div className="flex justify-end gap-3">
                <Button variant="outline" onClick={() => onOpenChange(false)}>
                  Cancel
                </Button>
                <Button onClick={handleSave} disabled={loading} className="gap-2">
                  <Save className="w-4 h-4" />
                  {loading ? "Saving..." : "Save Changes"}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
