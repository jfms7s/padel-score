import { describe, expect, it, vi } from 'vitest';
import { keepAwake, type WakeDoc, type WakeNav } from './wakelock';

function fakeDoc(): WakeDoc & { fire(state: 'visible' | 'hidden'): void; listeners: number } {
	const handlers = new Set<() => void>();
	const doc = {
		visibilityState: 'visible' as DocumentVisibilityState,
		addEventListener: (_: 'visibilitychange', h: () => void) => void handlers.add(h),
		removeEventListener: (_: 'visibilitychange', h: () => void) => void handlers.delete(h),
		fire(state: 'visible' | 'hidden') {
			doc.visibilityState = state;
			for (const h of handlers) h();
		},
		get listeners() {
			return handlers.size;
		}
	};
	return doc;
}

function fakeNav() {
	const sentinels: { release: ReturnType<typeof vi.fn>; released: boolean }[] = [];
	const request = vi.fn(async (type: 'screen') => {
		expect(type).toBe('screen');
		const s = { released: false, release: vi.fn(async () => void (s.released = true)) };
		sentinels.push(s);
		return s;
	});
	return { nav: { wakeLock: { request } } as unknown as WakeNav, request, sentinels };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('keepAwake', () => {
	it('wake_lock_reacquired_on_visibility', async () => {
		const { nav, request, sentinels } = fakeNav();
		const doc = fakeDoc();
		const lock = keepAwake(nav, doc);
		expect(lock.supported).toBe(true);
		await flush();
		expect(request).toHaveBeenCalledTimes(1);

		// The browser drops the lock when the page is hidden; coming back asks again.
		doc.fire('hidden');
		await flush();
		expect(request).toHaveBeenCalledTimes(1);
		doc.fire('visible');
		await flush();
		expect(request).toHaveBeenCalledTimes(2);

		lock.release();
		await flush();
		expect(sentinels.every((s) => s.release.mock.calls.length > 0 || s.released)).toBe(true);
		expect(sentinels[1]?.release).toHaveBeenCalled();
		expect(doc.listeners).toBe(0);
		doc.fire('visible');
		await flush();
		expect(request).toHaveBeenCalledTimes(2);
	});

	it('wake lock missing is a no-op', () => {
		const doc = fakeDoc();
		const lock = keepAwake({} as WakeNav, doc);
		expect(lock.supported).toBe(false);
		expect(doc.listeners).toBe(0);
		expect(() => lock.release()).not.toThrow();
	});

	it('swallows a refused request and a lock granted after release', async () => {
		const doc = fakeDoc();
		const refused = vi.fn(async () => {
			throw new DOMException('no', 'NotAllowedError');
		});
		const lock = keepAwake({ wakeLock: { request: refused } } as unknown as WakeNav, doc);
		await flush();
		expect(refused).toHaveBeenCalledTimes(1);
		lock.release();

		// A request still in flight when the page leaves is released as soon as it lands.
		const { nav, sentinels } = fakeNav();
		const late = keepAwake(nav, doc);
		late.release();
		await flush();
		expect(sentinels[0]?.release).toHaveBeenCalled();
	});

	it('retry asks again only while no lock is held', async () => {
		const doc = fakeDoc();
		let refuse = true;
		const sentinels: { release: () => Promise<void>; addEventListener: (t: 'release', h: () => void) => void; fire: () => void }[] = [];
		const request = vi.fn(async () => {
			if (refuse) throw new DOMException('needs a gesture', 'NotAllowedError');
			let onRelease = () => {};
			const s = {
				release: vi.fn(async () => {}),
				addEventListener: (_: 'release', h: () => void) => void (onRelease = h),
				fire: () => onRelease()
			};
			sentinels.push(s);
			return s;
		});
		const lock = keepAwake({ wakeLock: { request } } as unknown as WakeNav, doc);
		await flush();
		expect(request).toHaveBeenCalledTimes(1);
		refuse = false;
		lock.retry();
		await flush();
		expect(request).toHaveBeenCalledTimes(2);
		lock.retry();
		await flush();
		expect(request).toHaveBeenCalledTimes(2);
		// The system drops the lock (battery saver): the next tap asks again.
		sentinels[0]?.fire();
		lock.retry();
		await flush();
		expect(request).toHaveBeenCalledTimes(3);
		lock.release();
		lock.retry();
		await flush();
		expect(request).toHaveBeenCalledTimes(3);
	});

	it('retry is a no-op without support', () => {
		expect(() => keepAwake({} as WakeNav, fakeDoc()).retry()).not.toThrow();
	});
});
