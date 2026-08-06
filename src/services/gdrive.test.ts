import { describe, it, expect, vi } from 'vitest';
import { parseVideoFilename, getGDriveAuthStatus } from './gdrive';

// Mock Tauri invoke to avoid issues during import/execution
vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn(),
}));

describe('parseVideoFilename', () => {
    describe('TV Shows', () => {
        it('should parse standard format (SxxExx)', () => {
            expect(parseVideoFilename('The Office S01E01.mp4')).toEqual({
                title: 'The Office',
                season: 1,
                episode: 1,
            });
            expect(parseVideoFilename('Breaking Bad S5E14.mkv')).toEqual({
                title: 'Breaking Bad',
                season: 5,
                episode: 14,
            });
        });

        it('should handle lowercase format (sxxexx)', () => {
            expect(parseVideoFilename('house of cards s02e01.avi')).toEqual({
                title: 'house of cards',
                season: 2,
                episode: 1,
            });
        });

        it('should handle dot separators', () => {
            expect(parseVideoFilename('Game.of.Thrones.S01E01.mp4')).toEqual({
                title: 'Game of Thrones',
                season: 1,
                episode: 1,
            });
        });

        it('should handle underscore separators', () => {
            expect(parseVideoFilename('Stranger_Things_S01E01.mp4')).toEqual({
                title: 'Stranger Things',
                season: 1,
                episode: 1,
            });
        });

        it('should handle 1x01 format', () => {
            // Note: Current implementation might not support this despite comments saying so.
            // This test verifies if the feature works as documented.
            expect(parseVideoFilename('Heroes 1x01.mp4')).toEqual({
                title: 'Heroes',
                season: 1,
                episode: 1
            });
        });
    });

    describe('Movies', () => {
        it('should parse standard movie format (Name (Year))', () => {
            expect(parseVideoFilename('Inception (2010).mp4')).toEqual({
                title: 'Inception',
                year: 2010,
            });
        });

        it('should parse movie format with dots', () => {
            expect(parseVideoFilename('The.Matrix.1999.mkv')).toEqual({
                title: 'The Matrix',
                year: 1999,
            });
        });

        it('should parse movie format with brackets', () => {
            expect(parseVideoFilename('Avatar [2009].mp4')).toEqual({
                title: 'Avatar',
                year: 2009,
            });
        });

        it('should parse movie format without brackets', () => {
             // Regex: /^(.+?)[.\s_-]+[(\[]?(\d{4})[)\]]?/
             // 'Interstellar 2014' -> Matches 'Interstellar' then space then '2014'
            expect(parseVideoFilename('Interstellar 2014.mp4')).toEqual({
                title: 'Interstellar',
                year: 2014,
            });
        });
    });

    describe('Fallback / Edge Cases', () => {
        it('should handle files with no clear pattern', () => {
            expect(parseVideoFilename('Just Some Video.mp4')).toEqual({
                title: 'Just Some Video',
            });
        });

        it('should clean up dots and underscores in title', () => {
            expect(parseVideoFilename('My.Home_Video.mp4')).toEqual({
                title: 'My Home Video',
            });
        });
    });
});

describe('getGDriveAuthStatus', () => {
    it('exposes the wrapper function', () => {
        expect(typeof getGDriveAuthStatus).toBe('function');
    });
});
