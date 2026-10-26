import { describe, expect, it } from 'vitest';
import { RELOAD_GAP_MS, markReload, mayReload, reloadNow, updateScreen, type UpdateMoment } from './update';
import type { StorageLike } from './setup';

function memory(): StorageLike & { data: Map<string, string> } {
	const data = new Map<string, string>();
	return {
		data,
		getItem: (k) => data.get(k) ?? null,
		setItem: (k, v) => void data.set(k, v),
		removeItem: (k) => void data.delete(k)
	};
}

const broken: StorageLike = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	},
	removeItem: () => {
		throw new Error('blocked');
	}
};

describe('updateScreen', () => {
	it('knows the scoring screens', () => {
		expect(updateScreen('/s/abc')).toBe('scoring');
		expect(updateScreen('/c/abc')).toBe('scoring');
	});

	it('knows the viewing screens', () => {
		expect(updateScreen('/v/abc')).toBe('viewing');
		expect(updateScreen('/cv/abc')).toBe('viewing');
		expect(updateScreen('/e/abc')).toBe('viewing');
	});

	it('treats everything else as the app', () => {
		for (const p of ['/', '/new', '/events', '/events/abc', '/setup', '/s', '/serve/x', '/cvx/a']) {
			expect(updateScreen(p)).toBe('app');
		}
	});
});

describe('reloadNow', () => {
	const at = (m: Partial<UpdateMoment>): UpdateMoment => ({
		screen: 'scoring',
		held: false,
		hidden: false,
		justShown: false,
		...m
	});

	it('never reloads while held, on any screen or moment', () => {
		for (const screen of ['scoring', 'viewing', 'app'] as const) {
			for (const hidden of [false, true]) {
				for (const justShown of [false, true]) {
					expect(reloadNow({ screen, held: true, hidden, justShown })).toBe(false);
				}
			}
		}
	});

	it('reloads a scoring screen only out of view or on return', () => {
		expect(reloadNow(at({}))).toBe(false);
		expect(reloadNow(at({ hidden: true }))).toBe(true);
		expect(reloadNow(at({ justShown: true }))).toBe(true);
	});

	it('reloads a viewing screen at once', () => {
		expect(reloadNow(at({ screen: 'viewing' }))).toBe(true);
	});

	it('leaves an app page in view alone until it navigates or returns', () => {
		expect(reloadNow(at({ screen: 'app' }))).toBe(false);
		expect(reloadNow(at({ screen: 'app', hidden: true }))).toBe(true);
		expect(reloadNow(at({ screen: 'app', justShown: true }))).toBe(true);
	});
});

describe('mayReload', () => {
	it('allows the first reload', () => {
		expect(mayReload(memory(), 1_000_000)).toBe(true);
	});

	it('refuses another within the gap, then allows it', () => {
		const s = memory();
		markReload(s, 1_000_000);
		expect(mayReload(s, 1_000_000 + RELOAD_GAP_MS - 1)).toBe(false);
		expect(mayReload(s, 1_000_000 + RELOAD_GAP_MS)).toBe(true);
	});

	it('ignores a mark from the future (clock set back) and junk', () => {
		const s = memory();
		markReload(s, 5_000_000);
		expect(mayReload(s, 1_000_000)).toBe(true);
		s.data.set('update-reloaded-at', 'nonsense');
		expect(mayReload(s, 1_000_000)).toBe(true);
	});

	it('survives a blocked store', () => {
		expect(mayReload(broken, 1)).toBe(true);
		expect(() => markReload(broken, 1)).not.toThrow();
	});
});
