// Keep the fence screen on while the page is open. The browser drops a screen wake lock
// whenever the page is hidden, so it is asked for again each time the page is visible.

interface Sentinel {
	release(): Promise<void>;
}

export interface WakeNav {
	wakeLock?: { request(type: 'screen'): Promise<Sentinel> };
}

export interface WakeDoc {
	readonly visibilityState: DocumentVisibilityState;
	addEventListener(type: 'visibilitychange', listener: () => void): void;
	removeEventListener(type: 'visibilitychange', listener: () => void): void;
}

export interface Awake {
	/** False when the browser has no wake lock; the screen then sleeps as usual. */
	supported: boolean;
	release(): void;
}

/** Hold a screen wake lock until `release()`. Never throws: a refused lock is simply not held. */
export function keepAwake(nav: WakeNav, doc: WakeDoc): Awake {
	const api = nav.wakeLock;
	if (!api || typeof api.request !== 'function') {
		return { supported: false, release: () => {} };
	}
	const wakeLock = api;

	let released = false;
	let sentinel: Sentinel | null = null;

	const drop = (s: Sentinel) => void s.release().catch(() => {});

	async function acquire() {
		try {
			const s = await wakeLock.request('screen');
			if (released) drop(s);
			else {
				if (sentinel) drop(sentinel);
				sentinel = s;
			}
		} catch {
			// Refused (battery saver, not visible, no user gesture yet): try again next time.
		}
	}

	const onVisibility = () => {
		if (!released && doc.visibilityState === 'visible') void acquire();
	};

	doc.addEventListener('visibilitychange', onVisibility);
	void acquire();

	return {
		supported: true,
		release() {
			if (released) return;
			released = true;
			doc.removeEventListener('visibilitychange', onVisibility);
			if (sentinel) drop(sentinel);
			sentinel = null;
		}
	};
}
