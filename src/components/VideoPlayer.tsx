import { useState, useRef, useEffect, useCallback } from 'react'
import { X, Play, Pause, Volume2, VolumeX, Maximize2, Minimize2, SkipBack, SkipForward, Settings, Loader2 } from 'lucide-react'
import { Slider } from '@/components/ui/slider'
import { invoke } from '@tauri-apps/api/tauri'

interface VideoPlayerProps {
    src: string
    title: string
    poster?: string
    onClose: () => void
    onProgress?: (currentTime: number, duration: number) => void
    initialTime?: number
}

export function VideoPlayer({ src, title, poster, onClose, onProgress, initialTime = 0 }: VideoPlayerProps) {
    const videoRef = useRef<HTMLVideoElement>(null)
    const containerRef = useRef<HTMLDivElement>(null)
    const progressReportRef = useRef<number>(0)

    const [isPlaying, setIsPlaying] = useState(false)
    const [currentTime, setCurrentTime] = useState(0)
    const [duration, setDuration] = useState(0)
    const [volume, setVolume] = useState(1)
    const [isMuted, setIsMuted] = useState(false)
    const [isFullscreen, setIsFullscreen] = useState(false)
    const [showControls, setShowControls] = useState(true)
    const [isLoading, setIsLoading] = useState(true)
    const [error, setError] = useState<string | null>(null)
    const [videoSrc, setVideoSrc] = useState<string | null>(null)
    const blobUrlRef = useRef<string | null>(null)

    const hideControlsTimeout = useRef<NodeJS.Timeout | null>(null)

    // Load video file as blob URL using Tauri commands
    useEffect(() => {
        let cancelled = false;

        async function loadVideo() {
            console.log('[VideoPlayer] Loading video from:', src);

            // If it's already a URL (http/https/blob), use it directly
            if (src.startsWith('http://') || src.startsWith('https://') || src.startsWith('blob:')) {
                setVideoSrc(src);
                return;
            }

            try {
                // Get file size first
                const fileSize = await invoke<number>('get_video_file_size', { filePath: src });
                console.log('[VideoPlayer] File size:', fileSize);

                // Read file in chunks and create blob
                const chunkSize = 10 * 1024 * 1024; // 10MB chunks
                const chunks: Uint8Array[] = [];
                let offset = 0;

                while (offset < fileSize) {
                    if (cancelled) return;

                    const chunk = await invoke<number[]>('read_video_chunk', {
                        filePath: src,
                        offset: offset,
                        chunkSize: Math.min(chunkSize, fileSize - offset)
                    });

                    chunks.push(new Uint8Array(chunk));
                    offset += chunk.length;

                    console.log(`[VideoPlayer] Loaded ${Math.round(offset / fileSize * 100)}%`);
                }

                if (cancelled) return;

                // Determine MIME type based on file extension
                const ext = src.split('.').pop()?.toLowerCase();
                let mimeType = 'video/mp4';
                if (ext === 'mkv') mimeType = 'video/x-matroska';
                else if (ext === 'webm') mimeType = 'video/webm';
                else if (ext === 'avi') mimeType = 'video/x-msvideo';
                else if (ext === 'mov') mimeType = 'video/quicktime';

                // Create blob and URL
                const blob = new Blob(chunks as BlobPart[], { type: mimeType });
                const url = URL.createObjectURL(blob);

                // Clean up old blob URL if exists
                if (blobUrlRef.current) {
                    URL.revokeObjectURL(blobUrlRef.current);
                }
                blobUrlRef.current = url;

                console.log('[VideoPlayer] Created blob URL:', url);
                setVideoSrc(url);
            } catch (e) {
                console.error('[VideoPlayer] Failed to load video:', e);
                if (!cancelled) {
                    setError(`Failed to load video: ${e}`);
                    setIsLoading(false);
                }
            }
        }

        loadVideo();

        return () => {
            cancelled = true;
            // Cleanup blob URL on unmount
            if (blobUrlRef.current) {
                URL.revokeObjectURL(blobUrlRef.current);
                blobUrlRef.current = null;
            }
        };
    }, [src]);

    // Debug logging
    console.log('[VideoPlayer] Current videoSrc:', videoSrc);

    // Format time helper
    const formatTime = (seconds: number): string => {
        const hrs = Math.floor(seconds / 3600)
        const mins = Math.floor((seconds % 3600) / 60)
        const secs = Math.floor(seconds % 60)
        if (hrs > 0) {
            return `${hrs}:${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`
        }
        return `${mins}:${secs.toString().padStart(2, '0')}`
    }

    // Start hiding controls timer
    const startHideControlsTimer = useCallback(() => {
        if (hideControlsTimeout.current) {
            clearTimeout(hideControlsTimeout.current)
        }
        hideControlsTimeout.current = setTimeout(() => {
            if (isPlaying) {
                setShowControls(false)
            }
        }, 3000)
    }, [isPlaying])

    // Show controls on mouse move
    const handleMouseMove = useCallback(() => {
        setShowControls(true)
        startHideControlsTimer()
    }, [startHideControlsTimer])

    // Toggle play/pause
    const togglePlay = useCallback(() => {
        if (videoRef.current) {
            if (isPlaying) {
                videoRef.current.pause()
            } else {
                videoRef.current.play()
            }
        }
    }, [isPlaying])

    // Toggle fullscreen
    const toggleFullscreen = useCallback(() => {
        if (!containerRef.current) return

        if (!document.fullscreenElement) {
            containerRef.current.requestFullscreen()
            setIsFullscreen(true)
        } else {
            document.exitFullscreen()
            setIsFullscreen(false)
        }
    }, [])

    // Toggle mute
    const toggleMute = useCallback(() => {
        if (videoRef.current) {
            videoRef.current.muted = !isMuted
            setIsMuted(!isMuted)
        }
    }, [isMuted])

    // Seek video
    const handleSeek = useCallback((value: number[]) => {
        if (videoRef.current && duration) {
            const newTime = (value[0] / 100) * duration
            videoRef.current.currentTime = newTime
            setCurrentTime(newTime)
        }
    }, [duration])

    // Change volume
    const handleVolumeChange = useCallback((value: number[]) => {
        if (videoRef.current) {
            const newVolume = value[0] / 100
            videoRef.current.volume = newVolume
            setVolume(newVolume)
            setIsMuted(newVolume === 0)
        }
    }, [])

    // Skip forward/backward
    const skip = useCallback((seconds: number) => {
        if (videoRef.current) {
            videoRef.current.currentTime = Math.max(0, Math.min(duration, videoRef.current.currentTime + seconds))
        }
    }, [duration])

    // Keyboard controls
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            switch (e.key) {
                case ' ':
                case 'k':
                    e.preventDefault()
                    togglePlay()
                    break
                case 'f':
                    e.preventDefault()
                    toggleFullscreen()
                    break
                case 'm':
                    e.preventDefault()
                    toggleMute()
                    break
                case 'ArrowLeft':
                    e.preventDefault()
                    skip(-10)
                    break
                case 'ArrowRight':
                    e.preventDefault()
                    skip(10)
                    break
                case 'Escape':
                    if (isFullscreen) {
                        document.exitFullscreen()
                    } else {
                        onClose()
                    }
                    break
            }
        }

        window.addEventListener('keydown', handleKeyDown)
        return () => window.removeEventListener('keydown', handleKeyDown)
    }, [togglePlay, toggleFullscreen, toggleMute, skip, isFullscreen, onClose])

    // Report progress periodically
    useEffect(() => {
        if (onProgress && currentTime > 0) {
            const now = Date.now()
            if (now - progressReportRef.current > 5000) { // Report every 5 seconds
                progressReportRef.current = now
                onProgress(currentTime, duration)
            }
        }
    }, [currentTime, duration, onProgress])

    // Fullscreen change listener
    useEffect(() => {
        const handleFullscreenChange = () => {
            setIsFullscreen(!!document.fullscreenElement)
        }
        document.addEventListener('fullscreenchange', handleFullscreenChange)
        return () => document.removeEventListener('fullscreenchange', handleFullscreenChange)
    }, [])

    return (
        <div
            ref={containerRef}
            className="fixed inset-0 z-50 bg-black flex items-center justify-center font-sans"
            onMouseMove={handleMouseMove}
            onClick={(e) => {
                if (e.target === containerRef.current) {
                    togglePlay()
                }
            }}
        >
            {/* Video element */}
            <video
                ref={videoRef}
                src={videoSrc || undefined}
                poster={poster}
                className="w-full h-full object-contain"
                onLoadedMetadata={() => {
                    if (videoRef.current) {
                        setDuration(videoRef.current.duration)
                        if (initialTime > 0) {
                            videoRef.current.currentTime = initialTime
                        }
                    }
                }}
                onCanPlay={() => setIsLoading(false)}
                onWaiting={() => setIsLoading(true)}
                onPlaying={() => {
                    setIsLoading(false)
                    setIsPlaying(true)
                }}
                onPause={() => setIsPlaying(false)}
                onTimeUpdate={() => {
                    if (videoRef.current) {
                        setCurrentTime(videoRef.current.currentTime)
                    }
                }}
                onError={(e) => {
                    console.error('Video load error:', e)
                    console.error('Attempted video source:', videoSrc)
                    const video = videoRef.current
                    if (video?.error) {
                        console.error('Video error code:', video.error.code, 'message:', video.error.message)
                    }
                    setError(`Failed to load video: ${video?.error?.message || 'Unknown error'}`)
                }}
                onEnded={onClose}
                autoPlay
            />

            {/* Loading spinner */}
            {isLoading && (
                <div className="absolute inset-0 flex items-center justify-center bg-black/50 backdrop-blur-sm z-10 transition-opactiy duration-300">
                    <div className="relative">
                        <Loader2 className="h-16 w-16 animate-spin text-primary" />
                        <div className="absolute inset-0 bg-primary/20 blur-xl rounded-full" />
                    </div>
                </div>
            )}

            {/* Error display */}
            {error && (
                <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/90 backdrop-blur-md text-white z-20 p-8 text-center">
                    <div className="bg-destructive/10 p-4 rounded-full mb-4">
                        <X className="w-12 h-12 text-destructive" />
                    </div>
                    <p className="text-xl font-semibold mb-2">Video Error</p>
                    <p className="text-muted-foreground mb-6 max-w-md">{error}</p>
                    <button
                        onClick={onClose}
                        className="px-6 py-2.5 bg-white text-black font-semibold rounded-full hover:bg-white/90 transition-all transform hover:scale-105 shadow-lg"
                    >
                        Close Player
                    </button>
                </div>
            )}

            {/* Controls overlay */}
            <div
                className={`absolute inset-0 flex flex-col justify-between transition-all duration-500 ease-in-out ${showControls ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4 pointer-events-none'
                    }`}
            >
                {/* Top bar */}
                <div className="bg-gradient-to-b from-black/60 to-transparent p-6 flex items-start justify-between backdrop-blur-[2px]">
                    <div className="flex flex-col">
                        <h1 className="text-white text-xl font-bold tracking-tight drop-shadow-md">{title}</h1>
                    </div>
                    <button
                        onClick={onClose}
                        className="p-2.5 hover:bg-white/10 rounded-full transition-colors backdrop-blur-md bg-black/20 border border-white/5"
                    >
                        <X className="h-6 w-6 text-white" />
                    </button>
                </div>

                {/* Center play button */}
                <div className="flex-1 flex items-center justify-center">
                    <button
                        onClick={togglePlay}
                        className="group relative p-8 rounded-full transition-all duration-300 transform hover:scale-110 active:scale-95"
                    >
                        <div className="absolute inset-0 bg-black/40 backdrop-blur-sm rounded-full border border-white/10 transition-colors group-hover:bg-black/60" />
                        {isPlaying ? (
                            <Pause className="relative h-16 w-16 text-white drop-shadow-lg" />
                        ) : (
                            <Play className="relative h-16 w-16 text-white ml-2 drop-shadow-lg" />
                        )}
                    </button>
                </div>

                {/* Bottom controls */}
                <div className="bg-gradient-to-t from-black/90 via-black/50 to-transparent pt-20 pb-8 px-8 space-y-4">
                    {/* Progress bar */}
                    <div className="flex items-center gap-4">
                        <span className="text-white/90 text-sm font-medium font-mono min-w-[50px]">
                            {formatTime(currentTime)}
                        </span>
                        <Slider
                            value={[duration ? (currentTime / duration) * 100 : 0]}
                            onValueChange={handleSeek}
                            max={100}
                            step={0.1}
                            className="flex-1 cursor-pointer"
                        />
                        <span className="text-white/60 text-sm font-medium font-mono min-w-[50px] text-right">
                            {formatTime(duration)}
                        </span>
                    </div>

                    {/* Control buttons */}
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-4">
                            {/* Play/Pause */}
                            <button
                                onClick={togglePlay}
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white hover:text-primary"
                            >
                                {isPlaying ? (
                                    <Pause className="h-6 w-6 fill-current" />
                                ) : (
                                    <Play className="h-6 w-6 fill-current" />
                                )}
                            </button>

                            {/* Skip backward */}
                            <button
                                onClick={() => skip(-10)}
                                className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white group"
                                title="Skip back 10 seconds"
                            >
                                <SkipBack className="h-5 w-5 group-hover:-translate-x-0.5 transition-transform" />
                            </button>

                            {/* Skip forward */}
                            <button
                                onClick={() => skip(10)}
                                className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white group"
                                title="Skip forward 10 seconds"
                            >
                                <SkipForward className="h-5 w-5 group-hover:translate-x-0.5 transition-transform" />
                            </button>

                            {/* Volume */}
                            <div className="flex items-center gap-2 group/vol">
                                <button
                                    onClick={toggleMute}
                                    className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                >
                                    {isMuted || volume === 0 ? (
                                        <VolumeX className="h-5 w-5" />
                                    ) : (
                                        <Volume2 className="h-5 w-5" />
                                    )}
                                </button>
                                <div className="w-0 group-hover/vol:w-28 overflow-hidden transition-all duration-300 ease-out pl-2">
                                    <Slider
                                        value={[isMuted ? 0 : volume * 100]}
                                        onValueChange={handleVolumeChange}
                                        max={100}
                                        step={1}
                                        className="cursor-pointer py-2"
                                    />
                                </div>
                            </div>
                        </div>

                        <div className="flex items-center gap-3">
                            {/* Settings */}
                            <button
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                title="Settings"
                            >
                                <Settings className="h-5 w-5" />
                            </button>

                            {/* Fullscreen */}
                            <button
                                onClick={toggleFullscreen}
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                title={isFullscreen ? 'Exit fullscreen' : 'Fullscreen'}
                            >
                                {isFullscreen ? (
                                    <Minimize2 className="h-5 w-5" />
                                ) : (
                                    <Maximize2 className="h-5 w-5" />
                                )}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    )
}
