// Share a link: the phone's share sheet when there is one, else the clipboard. For the board,
// join and partner links, which are a single link each (a match's two links have ShareSheet).

export type ShareResult = 'shared' | 'copied' | 'failed';

export async function shareLink(url: string, title: string): Promise<ShareResult> {
	if (typeof navigator !== 'undefined' && typeof navigator.share === 'function') {
		try {
			await navigator.share({ url, title });
			return 'shared';
		} catch (e) {
			// Closing the sheet is not a failure and needs no message.
			if (e instanceof DOMException && e.name === 'AbortError') return 'shared';
		}
	}
	try {
		await navigator.clipboard.writeText(url);
		return 'copied';
	} catch {
		return 'failed';
	}
}
