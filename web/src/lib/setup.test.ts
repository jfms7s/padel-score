import { describe, expect, it } from 'vitest';
import {
	LAST_SETUP_KEY,
	RECENT_KEY,
	addRecent,
	defaultSetup,
	listRecent,
	loadLastSetup,
	afterCreate,
	matchLabel,
	recentSpectator,
	scorerPath,
	DEUCE_RULES,
	SETS_FORMATS,
	DECIDING_SETS,
	saveLastSetup,
	serveChoices,
	setSets,
	toCreateRequest,
	type Setup,
	type StorageLike
} from './setup';
import { safeStorage } from './storage';

function fakeStorage(initial: Record<string, string> = {}): StorageLike & { data: Record<string, string> } {
	const data = { ...initial };
	return {
		data,
		getItem: (k) => (k in data ? data[k] : null),
		setItem: (k, v) => {
			data[k] = v;
		}
	};
}

const throwing: StorageLike = {
	getItem: () => {
		throw new Error('denied');
	},
	setItem: () => {
		throw new Error('denied');
	}
};

const named = (over: Partial<Setup> = {}): Setup => ({
	...defaultSetup(),
	teams: { a: ['Ana', 'Rui'], b: ['João', 'Marta'] },
	...over
});

describe('setup', () => {
	it('default setup is golden point one set', () => {
		expect(defaultSetup()).toEqual({
			deuce: 'golden_point',
			sets: 'one',
			deciding: 'full_set',
			teams: { a: ['', ''], b: ['', ''] },
			firstServer: { team: 'a', index: 0 },
			otherFirstServer: 0
		});
	});

	it('one set forces full deciding set', () => {
		const s = named({ sets: 'one', deciding: 'super_tie_break' });
		expect(toCreateRequest(s).format).toEqual({
			sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' }
		});
		expect(setSets(named({ sets: 'best_of_3', deciding: 'super_tie_break' }), 'one').deciding).toBe(
			'full_set'
		);
		expect(setSets(named({ deciding: 'super_tie_break' }), 'best_of_3').deciding).toBe(
			'super_tie_break'
		);
	});

	it('corrupt saved setup falls back to default', () => {
		for (const bad of ['{nope', 'null', '7', '{"deuce":"bogus"}', '{"deuce":"advantage"}']) {
			expect(loadLastSetup(fakeStorage({ [LAST_SETUP_KEY]: bad }))).toEqual(defaultSetup());
		}
		expect(loadLastSetup(fakeStorage())).toEqual(defaultSetup());
		expect(loadLastSetup(throwing)).toEqual(defaultSetup());
	});

	it('saves and restores the rules and names', () => {
		const st = fakeStorage();
		const s = named({ deuce: 'star_point', sets: 'best_of_3', deciding: 'super_tie_break' });
		saveLastSetup(st, s);
		expect(loadLastSetup(st)).toEqual(s);
		expect(() => saveLastSetup(throwing, s)).not.toThrow();
	});

	it('request carries both serve choices', () => {
		const s = named({
			deuce: 'advantage',
			sets: 'best_of_3',
			deciding: 'super_tie_break',
			firstServer: { team: 'b', index: 1 },
			otherFirstServer: 1
		});
		expect(toCreateRequest(s)).toEqual({
			format: { sets: { deuce: 'advantage', sets: 'best_of_3', deciding: 'super_tie_break' } },
			start: { first_server: { team: 'b', index: 1 }, other_first_server: 1 },
			teams: { a: ['Ana', 'Rui'], b: ['João', 'Marta'] }
		});
	});

	it('blank names become Player placeholders in the request', () => {
		const s = { ...defaultSetup(), teams: { a: [' Ana ', ''] as [string, string], b: ['', ''] as [string, string] } };
		expect(toCreateRequest(s).teams).toEqual({
			a: ['Ana', 'Player 2'],
			b: ['Player 3', 'Player 4']
		});
	});

	it('serve choices fall back to Player 1 to 4 placeholders and follow the first server', () => {
		const blank = defaultSetup();
		expect(serveChoices(blank).first.map((c) => c.label)).toEqual([
			'Player 1',
			'Player 2',
			'Player 3',
			'Player 4'
		]);
		expect(serveChoices(blank).other.map((c) => c.label)).toEqual(['Player 3', 'Player 4']);
		const s = named({ firstServer: { team: 'b', index: 0 } });
		expect(serveChoices(s).other.map((c) => c.label)).toEqual(['Ana', 'Rui']);
	});
});

describe('recent matches', () => {
	it('adds newest first, caps at 10, dedupes by token', () => {
		const st = fakeStorage();
		for (let i = 0; i < 12; i++) addRecent(st, { scorer_token: `t${i}`, label: `m${i}`, created_at: i });
		const list = listRecent(st);
		expect(list).toHaveLength(10);
		expect(list[0].scorer_token).toBe('t11');
		addRecent(st, { scorer_token: 't5', label: 'again', created_at: 99 });
		expect(listRecent(st)[0].label).toBe('again');
		expect(listRecent(st).filter((r) => r.scorer_token === 't5')).toHaveLength(1);
	});

	it('survives corrupt or unavailable storage', () => {
		expect(listRecent(fakeStorage({ [RECENT_KEY]: '{x' }))).toEqual([]);
		expect(listRecent(fakeStorage({ [RECENT_KEY]: '[{"bad":1},{"scorer_token":"a","label":"l","created_at":1}]' }))).toHaveLength(1);
		expect(listRecent(throwing)).toEqual([]);
		expect(() => addRecent(throwing, { scorer_token: 'a', label: 'l', created_at: 1 })).not.toThrow();
	});

	it('keeps the spectator token, and entries saved before it existed', () => {
		const raw = JSON.stringify([
			{ scorer_token: 'a', label: 'l', created_at: 1, spectator_token: 'v' },
			{ scorer_token: 'b', label: 'l', created_at: 1 },
			{ scorer_token: 'c', label: 'l', created_at: 1, spectator_token: 7 }
		]);
		const st = fakeStorage({ [RECENT_KEY]: raw });
		expect(listRecent(st).map((r) => r.scorer_token)).toEqual(['a', 'b']);
		expect(recentSpectator(st, 'a')).toBe('v');
		expect(recentSpectator(st, 'b')).toBeNull();
		expect(recentSpectator(throwing, 'a')).toBeNull();
	});

	it('scorer links carry the spectator token in the fragment', () => {
		expect(scorerPath('a/b')).toBe('/s/a%2Fb');
		expect(scorerPath('a', 'v w')).toBe('/s/a#v=v%20w');
	});

	it('labels a match from the team names', () => {
		expect(matchLabel({ a: ['Ana', 'Rui'], b: ['João', 'Marta'] })).toBe('Ana / Rui v João / Marta');
	});
});

describe('option sweep', () => {
	it('every combination builds a valid request', () => {
		let n = 0;
		for (const deuce of DEUCE_RULES)
			for (const sets of SETS_FORMATS)
				for (const deciding of DECIDING_SETS)
					for (const team of ['a', 'b'] as const)
						for (const index of [0, 1])
							for (const other of [0, 1] as const) {
								const r = toCreateRequest(
									named({ deuce, sets, deciding, firstServer: { team, index }, otherFirstServer: other })
								);
								n++;
								const f = (r.format as { sets: { sets: string; deciding: string } }).sets;
								expect(f.sets === 'one' && f.deciding === 'super_tie_break').toBe(false);
								expect([0, 1]).toContain(r.start.first_server.index);
								expect(['a', 'b']).toContain(r.start.first_server.team);
								expect([0, 1]).toContain(r.start.other_first_server);
							}
		expect(n).toBe(3 * 2 * 2 * 2 * 2 * 2);
	});

	it('a saved one-set super tie-break loads as full set', () => {
		const st = fakeStorage();
		st.setItem(LAST_SETUP_KEY, JSON.stringify({ ...named(), sets: 'one', deciding: 'super_tie_break' }));
		expect(loadLastSetup(st).deciding).toBe('full_set');
	});

	it('a saved bad index or team falls back to the default', () => {
		for (const bad of [
			{ firstServer: { team: 'a', index: 2 } },
			{ firstServer: { team: 'c', index: 0 } },
			{ otherFirstServer: 5 }
		]) {
			const st = fakeStorage({ [LAST_SETUP_KEY]: JSON.stringify({ ...named(), ...bad }) });
			expect(loadLastSetup(st)).toEqual(defaultSetup());
		}
	});
});

describe('recent entries', () => {
	it('drops entries with a non-finite or out-of-range created_at', () => {
		const e = (created_at: unknown) => ({ scorer_token: 't', label: 'l', created_at });
		const raw = JSON.stringify([e(1), e(1e300), e(-1e300), e('x'), e(null)]);
		expect(listRecent(fakeStorage({ [RECENT_KEY]: raw })).map((r) => r.created_at)).toEqual([1]);
	});
});

describe('safeStorage', () => {
	it('safeStorage falls back when access throws', () => {
		const orig = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
		Object.defineProperty(globalThis, 'localStorage', {
			configurable: true,
			get() {
				throw new Error('blocked');
			}
		});
		try {
			const s = safeStorage();
			expect(s.getItem('x')).toBeNull();
			expect(() => s.setItem('x', 'y')).not.toThrow();
		} finally {
			if (orig) Object.defineProperty(globalThis, 'localStorage', orig);
			else delete (globalThis as { localStorage?: unknown }).localStorage;
		}
	});
});

describe('afterCreate', () => {
	const args = (storage: StorageLike, go: (p: string) => Promise<unknown>) => ({
		storage,
		setup: named(),
		scorerToken: 'tok/en',
		spectatorToken: 'spec',
		teams: named().teams,
		now: 5,
		go
	});

	it('saves, then navigates', async () => {
		const st = fakeStorage();
		const seen: string[] = [];
		const r = await afterCreate(args(st, async (p) => void seen.push(p)));
		expect(r).toEqual({ navigated: true, path: '/s/tok%2Fen#v=spec' });
		expect(seen).toEqual(['/s/tok%2Fen#v=spec']);
		expect(listRecent(st)).toEqual([
			{ scorer_token: 'tok/en', spectator_token: 'spec', label: 'Ana / Rui v João / Marta', created_at: 5 }
		]);
		expect(recentSpectator(st, 'tok/en')).toBe('spec');
		expect(recentSpectator(st, 'other')).toBeNull();
	});

	it('navigates even when saving throws', async () => {
		const seen: string[] = [];
		const r = await afterCreate(args(throwing, async (p) => void seen.push(p)));
		expect(r.navigated).toBe(true);
		expect(seen).toHaveLength(1);
	});

	it('reports the path when navigation rejects', async () => {
		const r = await afterCreate(
			args(fakeStorage(), async () => {
				throw new Error('nav');
			})
		);
		expect(r).toEqual({ navigated: false, path: '/s/tok%2Fen#v=spec' });
	});
});
