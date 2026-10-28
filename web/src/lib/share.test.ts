import { afterEach, describe, expect, it, vi } from 'vitest';
import { shareLink } from './share';

afterEach(() => vi.unstubAllGlobals());

describe('shareLink', () => {
	it('uses the share sheet when there is one', async () => {
		const share = vi.fn().mockResolvedValue(undefined);
		vi.stubGlobal('navigator', { share });
		expect(await shareLink('https://x.test/b/t', 'Board')).toBe('shared');
		expect(share).toHaveBeenCalledWith({ url: 'https://x.test/b/t', title: 'Board' });
	});

	it('counts a closed share sheet as done, not as a failure', async () => {
		const share = vi.fn().mockRejectedValue(new DOMException('closed', 'AbortError'));
		vi.stubGlobal('navigator', { share });
		expect(await shareLink('u', 't')).toBe('shared');
	});

	it('copies the link when there is no share sheet', async () => {
		const writeText = vi.fn().mockResolvedValue(undefined);
		vi.stubGlobal('navigator', { clipboard: { writeText } });
		expect(await shareLink('u', 't')).toBe('copied');
		expect(writeText).toHaveBeenCalledWith('u');
	});

	it('falls back to the clipboard when sharing fails, and says so when that fails too', async () => {
		vi.stubGlobal('navigator', {
			share: vi.fn().mockRejectedValue(new Error('no')),
			clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) }
		});
		expect(await shareLink('u', 't')).toBe('failed');
	});
});
