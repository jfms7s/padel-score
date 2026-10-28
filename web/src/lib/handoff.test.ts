import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultSetup } from './setup';
import {
	parseFrom,
	putStartHandoff,
	startFailure,
	startFromParam,
	takeStartHandoff,
	withStartPlayers
} from './handoff';
import { ApiError } from './api';
import { strings } from './strings';
import type { StartPlayers } from './types';

const started: StartPlayers = {
	players: [
		{ player_id: 'p1', name: 'Ana' },
		{ player_id: 'p2', name: 'Rui' },
		{ player_id: 'p3', name: 'Marta' },
		{ player_id: 'p4', name: 'Joana' }
	],
	format: null,
	from: { kind: 'game', id: 'g1' }
};

function fakeSession() {
	const data = new Map<string, string>();
	vi.stubGlobal('sessionStorage', {
		getItem: (k: string) => data.get(k) ?? null,
		setItem: (k: string, v: string) => void data.set(k, v),
		removeItem: (k: string) => void data.delete(k)
	});
	return data;
}

afterEach(() => vi.unstubAllGlobals());

describe('start hand-off', () => {
	it('a take returns the value once', () => {
		const data = fakeSession();
		putStartHandoff(started);
		expect(data.has('padel-score:start:game:g1')).toBe(true);
		expect(takeStartHandoff('game:g1')).toEqual(started);
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('is keyed by the source', () => {
		fakeSession();
		putStartHandoff(started);
		expect(takeStartHandoff('game:g2')).toBeNull();
		expect(takeStartHandoff('scheduled:g1')).toBeNull();
	});

	it('ignores a value that is not a hand-off', () => {
		const data = fakeSession();
		data.set('padel-score:start:game:g1', '{"players":[]}');
		expect(takeStartHandoff('game:g1')).toBeNull();
		data.set('padel-score:start:game:g1', 'nope');
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('survives storage that throws', () => {
		vi.stubGlobal('sessionStorage', {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('full');
			},
			removeItem: () => {}
		});
		expect(() => putStartHandoff(started)).not.toThrow();
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('fills seats 0-1 into team A and 2-3 into team B', () => {
		const s = withStartPlayers(defaultSetup(), started);
		expect(s.teams).toEqual({ a: ['Ana', 'Rui'], b: ['Marta', 'Joana'] });
		expect(s.players).toEqual({ a: ['p1', 'p2'], b: ['p3', 'p4'] });
		expect(s.deuce).toBe(defaultSetup().deuce);
	});

	it('leaves the seats a short game lacks empty', () => {
		const short = { ...started, players: started.players.slice(0, 3) };
		const s = withStartPlayers(defaultSetup(), short);
		expect(s.teams).toEqual({ a: ['Ana', 'Rui'], b: ['Marta', ''] });
		expect(s.players).toEqual({ a: ['p1', 'p2'], b: ['p3', null] });
		fakeSession();
		putStartHandoff(short);
		expect(takeStartHandoff('game:g1')).toEqual(short);
	});

	it('reads the from query value', () => {
		expect(parseFrom('game:g1')).toEqual({ kind: 'game', id: 'g1' });
		expect(parseFrom('nope')).toBeNull();
		expect(parseFrom(null)).toBeNull();
	});
});

describe('startFromParam', () => {
	const last = defaultSetup();

	it('is a plain setup with no from', () => {
		fakeSession();
		expect(startFromParam(null, last)).toEqual({ setup: last, from: null, expired: false });
	});

	it('fills the four players from the hand-off and names the source', () => {
		fakeSession();
		putStartHandoff(started);
		const out = startFromParam('game:g1', last);
		expect(out.setup).toEqual(withStartPlayers(last, started));
		expect(out.from).toEqual({ kind: 'game', id: 'g1' });
		expect(out.expired).toBe(false);
	});

	it('says expired, and keeps the last setup, when the hand-off is gone', () => {
		fakeSession();
		expect(startFromParam('game:g1', last)).toEqual({ setup: last, from: null, expired: true });
	});

	it('says expired for a from this build does not know', () => {
		fakeSession();
		expect(startFromParam('scheduled:s1', last)).toEqual({ setup: last, from: null, expired: true });
	});

	it('is taken once: reading it again, as a re-run would, finds it expired', () => {
		fakeSession();
		putStartHandoff(started);
		expect(startFromParam('game:g1', last).expired).toBe(false);
		expect(startFromParam('game:g1', last).expired).toBe(true);
	});
});

describe('startFailure', () => {
	const err = (status: number, code: string) => new ApiError(status, code as never, 'The server answered ' + status);

	it('says why a create from a game was refused, in the board copy', () => {
		const out = startFailure(err(409, 'already_started'), true);
		expect(out.message).toBe(strings.boards.errors.already_started);
		expect(out.dropFrom).toBe(true);
	});

	it('drops the game when it is over or no longer the starter s', () => {
		expect(startFailure(err(409, 'game_over'), true)).toMatchObject({ dropFrom: true });
		const gone = startFailure(err(404, 'not_found'), true);
		expect(gone.dropFrom).toBe(true);
		expect(gone.message).toBe(strings.setup.gameGone);
	});

	it('keeps the game for a refusal a retry can fix', () => {
		const out = startFailure(err(409, 'not_yet'), true);
		expect(out).toEqual({ message: strings.boards.errors.not_yet, dropFrom: false });
	});

	it('leaves a plain create to its own message', () => {
		expect(startFailure(err(409, 'already_started'), false)).toEqual({ message: null, dropFrom: false });
		expect(startFailure(new Error('boom'), true)).toEqual({ message: null, dropFrom: false });
	});
});
