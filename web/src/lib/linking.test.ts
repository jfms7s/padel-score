import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	PENDING_LINK_KEY,
	claimBadgeCount,
	codeDeadline,
	linkLabel,
	outcomeText,
	previewText,
	stashLinkCode,
	takeStashedLinkCode,
	STASH_TTL_MS
} from './linking';
import type { ClaimPreview, Player } from './types';

const at = (iso: string) => Math.floor(Date.parse(iso) / 1000);

const preview = (over: Partial<ClaimPreview> = {}): ClaimPreview => ({
	name: 'Rui',
	coordinator_name: 'Ana',
	matches: 5,
	events: 2,
	first_at: at('2026-03-15T12:00:00Z'),
	last_at: at('2026-10-15T12:00:00Z'),
	...over
});

const player = (link: Player['link']): Player => ({
	id: 'p1',
	name: 'Rui',
	archived: false,
	created_at: 1761400000,
	record: { wins: 0, losses: 0, draws: 0 },
	link,
	pending_claims: 0
});

class MemoryStorage {
	private data = new Map<string, string>();
	getItem = (k: string) => this.data.get(k) ?? null;
	setItem = (k: string, v: string) => void this.data.set(k, v);
	removeItem = (k: string) => void this.data.delete(k);
}

afterEach(() => {
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe('previewText', () => {
	it('preview_text_counts_and_range', () => {
		expect(previewText(preview())).toBe('5 matches · 2 events · Mar 2026 – Oct 2026');
	});

	it('preview_text_same_month', () => {
		const p = preview({
			matches: 1,
			events: 0,
			first_at: at('2026-03-02T12:00:00Z'),
			last_at: at('2026-03-20T12:00:00Z')
		});
		expect(previewText(p)).toBe('1 match · Mar 2026');
		expect(
			previewText(
				preview({ matches: 3, events: 1, first_at: at('2026-03-02T12:00:00Z'), last_at: at('2026-03-20T12:00:00Z') })
			)
		).toBe('3 matches · 1 event · Mar 2026');
	});

	it('preview_text_nothing_yet', () => {
		expect(previewText(preview({ matches: 0, events: 0, first_at: null, last_at: null }))).toBe(
			'No finished matches yet'
		);
	});
});

describe('linkLabel', () => {
	it('link_label_me_linked_none', () => {
		expect(linkLabel(player({ display_name: 'Rui Costa', linked_at: 1, is_me: true, automatic: false }))).toBe('You');
		expect(linkLabel(player({ display_name: 'Rui Costa', linked_at: 1, is_me: false, automatic: false }))).toBe(
			'Linked · Rui Costa'
		);
		expect(linkLabel(player(null))).toBeNull();
	});
});

describe('the stashed link code', () => {
	it('stash_round_trip_and_expiry', () => {
		const store = new MemoryStorage();
		vi.stubGlobal('sessionStorage', store);
		vi.useFakeTimers();
		vi.setSystemTime(new Date('2026-10-25T10:00:00Z'));

		stashLinkCode('abc');
		expect(store.getItem(PENDING_LINK_KEY)).not.toBeNull();
		expect(takeStashedLinkCode()).toBe('abc');
		// A read clears it.
		expect(takeStashedLinkCode()).toBeNull();
		expect(store.getItem(PENDING_LINK_KEY)).toBeNull();

		stashLinkCode('old');
		vi.setSystemTime(Date.now() + STASH_TTL_MS + 1000);
		expect(takeStashedLinkCode()).toBeNull();
		expect(store.getItem(PENDING_LINK_KEY)).toBeNull();

		stashLinkCode('fresh');
		vi.setSystemTime(Date.now() + STASH_TTL_MS - 1000);
		expect(takeStashedLinkCode()).toBe('fresh');
	});

	it('stash_survives_storage_errors', () => {
		const boom = () => {
			throw new Error('blocked');
		};
		vi.stubGlobal('sessionStorage', { getItem: boom, setItem: boom, removeItem: boom });
		expect(() => stashLinkCode('abc')).not.toThrow();
		expect(takeStashedLinkCode()).toBeNull();
		// Merely touching the global throws.
		vi.unstubAllGlobals();
		Object.defineProperty(globalThis, 'sessionStorage', { get: boom, configurable: true });
		try {
			expect(takeStashedLinkCode()).toBeNull();
		} finally {
			Reflect.deleteProperty(globalThis, 'sessionStorage');
		}
	});
});

describe('outcomeText', () => {
	it('outcome_text_win_loss_draw', () => {
		expect(outcomeText({ outcome: 'win' })).toBe('Won');
		expect(outcomeText({ outcome: 'loss' })).toBe('Lost');
		expect(outcomeText({ outcome: 'draw' })).toBe('Drawn');
	});
});

describe('claimBadgeCount', () => {
	it('sums pending claims over the list', () => {
		expect(claimBadgeCount([])).toBe(0);
		expect(claimBadgeCount([{ pending_claims: 1 }, { pending_claims: 0 }, { pending_claims: 2 }])).toBe(3);
	});
});

describe('codeDeadline', () => {
	const ttl = 600;
	it('counts the server-reported time from when the response arrived', () => {
		const received = Date.parse('2026-10-25T12:00:00Z');
		expect(codeDeadline(ttl, received)).toBe(received + 600_000);
	});
	it('gives a full code lifetime however wrong the client clock is', () => {
		for (const skew of [15 * 60_000, -15 * 60_000]) {
			const received = Date.now() + skew;
			const deadline = codeDeadline(ttl, received);
			// Fresh at arrival (not already expired), and ten minutes of it left.
			expect(deadline).toBeGreaterThan(received);
			expect(deadline - received).toBe(600_000);
		}
	});
	it('never lies in the past for a bad value', () => {
		expect(codeDeadline(-5, 1000)).toBe(1000);
	});
});
