import { describe, expect, it } from 'vitest';
import {
	LAST_SETUP_KEY,
	RECENT_KEY,
	addRecent,
	defaultSetup,
	listRecent,
	loadLastSetup,
	nameOrSlot,
	slotName,
	afterCreate,
	matchLabel,
	recentSpectator,
	scorerPath,
	DEUCE_RULES,
	SETS_FORMATS,
	DECIDING_SETS,
	saveLastSetup,
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
		},
		removeItem: (k) => {
			delete data[k];
		}
	};
}

const throwing: StorageLike = {
	getItem: () => {
		throw new Error('denied');
	},
	setItem: () => {
		throw new Error('denied');
	},
	removeItem: () => {
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
			serve: { team: 'a', first: { a: 1, b: 1 } },
			players: { a: [null, null], b: [null, null] }
		});
	});

	it('one set forces full deciding set', () => {
		const s = named({ sets: 'one', deciding: 'super_tie_break' });
		expect(toCreateRequest(s).format).toEqual({
			sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' }
		});
		expect(setSets(named({ sets: 'best_of_3', deciding: 'super_tie_break' }), 'one').deciding).toBe('full_set');
		expect(setSets(named({ deciding: 'super_tie_break' }), 'best_of_3').deciding).toBe('super_tie_break');
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
			serve: { team: 'b', first: { a: 0, b: 1 } }
		});
		expect(toCreateRequest(s)).toEqual({
			format: { sets: { deuce: 'advantage', sets: 'best_of_3', deciding: 'super_tie_break' } },
			start: { first_server: { team: 'b', index: 1 }, other_first_server: 0 },
			teams: { a: ['Ana', 'Rui'], b: ['João', 'Marta'] }
		});
	});

	it('to_create_request_omits_players_when_none_picked', () => {
		expect(toCreateRequest(named())).not.toHaveProperty('players');
	});

	it('to_create_request_sends_picked_slots', () => {
		const s = named({ players: { a: ['p-ana', null], b: [null, 'p-marta'] } });
		expect(toCreateRequest(s).players).toEqual({ a: ['p-ana', null], b: [null, 'p-marta'] });
		expect(toCreateRequest(s).teams).toEqual({ a: ['Ana', 'Rui'], b: ['João', 'Marta'] });
	});

	it('last_setup_does_not_store_player_ids', () => {
		const st = fakeStorage();
		const s = named({ players: { a: ['p-ana', 'p-rui'], b: [null, 'p-marta'] } });
		saveLastSetup(st, s);
		expect(st.data[LAST_SETUP_KEY]).not.toContain('p-ana');
		expect(JSON.parse(st.data[LAST_SETUP_KEY]!)).not.toHaveProperty('players');
		const loaded = loadLastSetup(st);
		expect(loaded.players).toEqual({ a: [null, null], b: [null, null] });
		expect(loaded.teams).toEqual(s.teams);
		// Ids saved by hand (or by an older build) are ignored too.
		const planted = JSON.stringify({ ...s, players: { a: ['p-x', null], b: [null, null] } });
		expect(loadLastSetup(fakeStorage({ [LAST_SETUP_KEY]: planted })).players).toEqual({
			a: [null, null],
			b: [null, null]
		});
	});

	it('blank names become Player placeholders in the request', () => {
		const s = { ...defaultSetup(), teams: { a: [' Ana ', ''] as [string, string], b: ['', ''] as [string, string] } };
		expect(toCreateRequest(s).teams).toEqual({
			a: ['Ana', 'Player 2'],
			b: ['Player 3', 'Player 4']
		});
	});

	it('names each slot Player 1..4, team A then team B', () => {
		expect([slotName('a', 0), slotName('a', 1), slotName('b', 0), slotName('b', 1)]).toEqual([
			'Player 1',
			'Player 2',
			'Player 3',
			'Player 4'
		]);
		const teams = { a: [' Ana ', ''] as [string, string], b: ['  ', 'Marta'] as [string, string] };
		expect(nameOrSlot(teams, 'a', 0)).toBe('Ana');
		expect(nameOrSlot(teams, 'a', 1)).toBe('Player 2');
		expect(nameOrSlot(teams, 'b', 0)).toBe('Player 3');
		expect(nameOrSlot(teams, 'b', 1)).toBe('Marta');
	});

	it('remembers the serve order in the new shape', () => {
		const st = fakeStorage();
		saveLastSetup(st, named({ serve: { team: 'b', first: { a: 0, b: 1 } } }));
		const stored = JSON.parse(st.data[LAST_SETUP_KEY]!);
		expect(stored.serveTeam).toBe('b');
		expect(stored.serveFirst).toEqual({ a: 0, b: 1 });
		expect(stored).not.toHaveProperty('firstServer');
		expect(loadLastSetup(st).serve).toEqual({ team: 'b', first: { a: 0, b: 1 } });
	});

	it('loads the old firstServer + otherFirstServer shape losslessly', () => {
		const { serve: _s, ...rest } = named();
		const old = (firstServer: object, otherFirstServer: number) =>
			fakeStorage({ [LAST_SETUP_KEY]: JSON.stringify({ ...rest, firstServer, otherFirstServer }) });
		expect(loadLastSetup(old({ team: 'a', index: 0 }, 0)).serve).toEqual({ team: 'a', first: { a: 0, b: 0 } });
		expect(loadLastSetup(old({ team: 'b', index: 0 }, 1)).serve).toEqual({ team: 'b', first: { a: 1, b: 0 } });
		expect(loadLastSetup(old({ team: 'b', index: 1 }, 0)).serve).toEqual({ team: 'b', first: { a: 0, b: 1 } });
	});

	it('a stored setup with no serve fields keeps its rules and defaults the serve', () => {
		const { serve: _s, ...rest } = named({ deuce: 'star_point' });
		const loaded = loadLastSetup(fakeStorage({ [LAST_SETUP_KEY]: JSON.stringify(rest) }));
		expect(loaded.deuce).toBe('star_point');
		expect(loaded.serve).toEqual(defaultSetup().serve);
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
		expect(
			listRecent(fakeStorage({ [RECENT_KEY]: '[{"bad":1},{"scorer_token":"a","label":"l","created_at":1}]' }))
		).toHaveLength(1);
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
						for (const index of [0, 1] as const)
							for (const other of [0, 1] as const) {
								const r = toCreateRequest(
									named({ deuce, sets, deciding, serve: { team, first: { a: index, b: other } } })
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

	it('a saved bad serve choice falls back to the default', () => {
		const { serve: _s, ...rest } = named();
		for (const bad of [
			{ firstServer: { team: 'a', index: 2 }, otherFirstServer: 0 },
			{ firstServer: { team: 'c', index: 0 }, otherFirstServer: 0 },
			{ firstServer: { team: 'a', index: 0 }, otherFirstServer: 5 },
			{ firstServer: { team: 'a', index: 0 } },
			{ serveTeam: 'c', serveFirst: { a: 0, b: 0 } },
			{ serveTeam: 'a', serveFirst: { a: 2, b: 0 } },
			{ serveTeam: 'a' }
		]) {
			const st = fakeStorage({ [LAST_SETUP_KEY]: JSON.stringify({ ...rest, ...bad }) });
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
