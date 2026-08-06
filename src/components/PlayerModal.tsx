import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { MonitorPlay, ExternalLink, Sparkles } from 'lucide-react'

interface PlayerModalProps {
    open: boolean
    onOpenChange: (open: boolean) => void
    onSelectPlayer: (player: 'mpv' | 'vlc' | 'builtin') => void
    title: string
}

export function PlayerModal({ open, onOpenChange, onSelectPlayer, title }: PlayerModalProps) {
    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="sm:max-w-md bg-background/95 backdrop-blur-xl border-border/50">
                <DialogHeader>
                    <DialogTitle className="text-xl font-bold text-white">
                        Choose Player
                    </DialogTitle>
                    <DialogDescription className="text-muted-foreground">
                        Watch <span className="font-medium text-foreground">{title}</span>
                    </DialogDescription>
                </DialogHeader>

                <div className="grid gap-4 py-4">
                    {/* Built-in Player (default) */}
                    <Button
                        variant="outline"
                        className="h-auto p-4 flex flex-col items-start gap-2 hover:bg-white/10 hover:border-blue-400/50 transition-all duration-300 group border-blue-500/20 bg-blue-500/5"
                        onClick={() => {
                            onSelectPlayer('builtin')
                            onOpenChange(false)
                        }}
                    >
                        <div className="flex items-center gap-3 w-full">
                            <div className="p-2 rounded-lg bg-gradient-to-br from-blue-500 to-blue-700 text-white shadow-lg shadow-blue-500/20 group-hover:scale-110 transition-transform">
                                <MonitorPlay className="size-5" />
                            </div>
                            <div className="flex-1 text-left">
                                <div className="font-semibold text-foreground group-hover:text-white transition-colors">
                                    Built-in Player
                                </div>
                                <div className="text-xs text-muted-foreground">
                                    Default • In-app playback • Faster startup
                                </div>
                            </div>
                            <div className="flex items-center gap-1 px-2 py-0.5 rounded-full bg-blue-500/20 text-blue-400 text-[10px] font-semibold">
                                <Sparkles className="size-3" />
                                RECOMMENDED
                            </div>
                        </div>
                        <div className="text-xs text-muted-foreground/70 pl-12">
                            Plays directly inside the app with libmpv
                        </div>
                    </Button>

                    {/* External MPV Option */}
                    <Button
                        variant="outline"
                        className="h-auto p-4 flex flex-col items-start gap-2 hover:bg-white/10 hover:border-white/50 transition-all duration-300 group"
                        onClick={() => {
                            onSelectPlayer('mpv')
                            onOpenChange(false)
                        }}
                    >
                        <div className="flex items-center gap-3 w-full">
                            <div className="p-2 rounded-lg bg-gradient-to-br from-gray-400 to-gray-600 text-white shadow-lg shadow-gray-500/20 group-hover:scale-110 transition-transform">
                                <ExternalLink className="size-5" />
                            </div>
                            <div className="flex-1 text-left">
                                <div className="font-semibold text-foreground group-hover:text-white transition-colors">
                                    External MPV
                                </div>
                                <div className="text-xs text-muted-foreground">
                                    Separate window • Full feature set
                                </div>
                            </div>
                        </div>
                        <div className="text-xs text-muted-foreground/70 pl-12">
                            Opens MPV as an external process
                        </div>
                    </Button>
                </div>

                {/* Quick tip */}
                <div className="text-xs text-center text-muted-foreground/60 border-t border-border/50 pt-4">
                    <span className="text-white">Tip:</span> Change default player in Settings → Player Engine
                </div>
            </DialogContent>
        </Dialog>
    )
}
