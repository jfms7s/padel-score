import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import { defaultSetup } from './setup';
import {
	cancelScheduled,
	createScheduled,
	joinMatch,
	laterStart,
	matchActions,
	scheduledChange,
	removeFromScheduled,
	scheduledMessage,
	startScheduled,
	toNewScheduled,
	type LaterDraft
} from './scheduled';
import { strings } from './strings';
import type { PlaceOut, ScheduledMe, ScheduledState, ScheduledView } from './types';

const START = 1_800_000_000;

function places(filled: number): PlaceOut[] {
	return [0, 1, 2, 3].map((slot) => ({ slot, name: slot < filled ? `P${slot}` : null, user_id: null }));
}

function view(over: Partial<ScheduledView> = {}): ScheduledView {
	return {
		id: 's1',
		join_token: 'tok',
		owner_name: 'Ana',
		starts_at: START,
		place: null,
		listing: 'private',
		format: { sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' } },
		places: places(2),
		line: [],
		state: 'open',
		match_spectator_token: null,
		me: { owner: false, slot: null, in_line: false },
		...over
	};
}

const me = (over: Partial<ScheduledMe> = {}): ScheduledMe => ({ owner: false, slot: null, in_line: false, ...over });

describe('toNewScheduled', () => {
	it('to_new_scheduled_maps_slots_and_format', () => {
		const setup = defaultSetup();
		setup.deuce = 'star_point';
		setup.sets = 'best_of_3';
		setup.deciding = 'super_tie_break';
		setup.players = { a: ['p1', null], b: ['p3', 'p4'] };
		const out = toNewScheduled(setup, { startsAt: START, place: '  Club Norte ', listing: 'public' });
		expect(out).toEqual({
			starts_at: START,
			place: 'Club Norte',
			listing: 'public',
			format: { sets: { deuce: 'star_point', sets: 'best_of_3', deciding: 'super_tie_break' } },
			places: ['p1', null, 'p3', 'p4']
		});
	});

	it('leaves the place out when blank and keeps one set to a full deciding set', () => {
		const setup = defaultSetup();
		setup.sets = 'one';
		setup.deciding = 'super_tie_break';
		const out = toNewScheduled(setup, { startsAt: START, place: '   ', listing: 'private' });
		expect('place' in out).toBe(false);
		expect(out.places).toEqual([null, null, null, null]);
		expect(out.format).toEqual({ sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' } });
	});
});

describe('matchActions', () => {
	const owner = me({ owner: true, slot: 0 });
	const inWindow = START - 1800;

	it('start_only_for_owner_in_the_window_with_four', () => {
		const full = view({ places: places(4), state: 'full', me: owner });
		expect(matchActions(full, inWindow).start).toBe(true);
		expect(matchActions(full, START + 5399).start).toBe(true);
		expect(matchActions(full, inWindow - 1).start).toBe(false);
		expect(matchActions(full, START + 5400).start).toBe(false);
		// Three places filled.
		expect(matchActions(view({ places: places(3), state: 'open', me: owner }), START).start).toBe(false);
		// Not the owner.
		expect(matchActions(view({ places: places(4), state: 'full', me: me({ slot: 1 }) }), START).start).toBe(false);
		// Signed out.
		expect(matchActions(view({ places: places(4), state: 'full', me: null }), START).start).toBe(false);
		// Cancelled or already started.
		for (const state of ['cancelled', 'started'] as ScheduledState[]) {
			expect(matchActions(view({ places: places(4), state, me: owner }), START).start).toBe(false);
		}
	});

	it('join_hidden_when_already_in_or_cancelled', () => {
		expect(matchActions(view(), START - 3600).join).toBe(true);
		expect(matchActions(view({ me: me({ slot: 2 }) }), START - 3600).join).toBe(false);
		expect(matchActions(view({ me: me({ in_line: true }) }), START - 3600).join).toBe(false);
		for (const state of ['cancelled', 'started', 'past'] as ScheduledState[]) {
			expect(matchActions(view({ state }), START - 3600).join).toBe(false);
		}
		// Signed out: Sign in to join is shown instead.
		expect(matchActions(view({ me: null }), START - 3600).join).toBe(false);
		// A full match still takes people on the line.
		expect(matchActions(view({ state: 'full', places: places(4) }), START - 3600).join).toBe(true);
	});

	it('leave shows for a place or the line', () => {
		expect(matchActions(view({ me: me({ slot: 1 }) }), START).leave).toBe(true);
		expect(matchActions(view({ me: me({ in_line: true }) }), START).leave).toBe(true);
		expect(matchActions(view(), START).leave).toBe(false);
		expect(matchActions(view({ me: null }), START).leave).toBe(false);
	});

	it('edit is for the owner while the match is open or full', () => {
		expect(matchActions(view({ me: owner }), START).edit).toBe(true);
		expect(matchActions(view({ me: owner, state: 'full', places: places(4) }), START).edit).toBe(true);
		for (const state of ['cancelled', 'started', 'past'] as ScheduledState[]) {
			expect(matchActions(view({ me: owner, state }), START).edit).toBe(false);
		}
		expect(matchActions(view(), START).edit).toBe(false);
	});
});

describe('laterStart', () => {
	const draft = (over: Partial<LaterDraft> = {}): LaterDraft => ({
		on: true,
		date: '2026-11-04',
		time: '20:00',
		place: '',
		listing: 'private',
		...over
	});
	const zone = 'UTC';
	const at = Date.UTC(2026, 10, 4, 20) / 1000;

	it('reads the inputs in the zone', () => {
		expect(laterStart(draft(), at - 60, zone)).toBe(at);
	});

	it('is null without both fields or when not in the future', () => {
		expect(laterStart(draft({ date: '' }), 0, zone)).toBeNull();
		expect(laterStart(draft({ time: '' }), 0, zone)).toBeNull();
		expect(laterStart(draft(), at, zone)).toBeNull();
	});
});

describe('scheduledChange', () => {
	const v = view({ starts_at: Date.UTC(2026, 10, 4, 20) / 1000, place: 'Club Norte' });
	const zone = 'UTC';

	it('sends only what changed', () => {
		expect(scheduledChange(v, { date: '2026-11-04', time: '20:00', place: 'Club Norte' }, zone)).toBeNull();
		expect(scheduledChange(v, { date: '2026-11-05', time: '20:00', place: ' Club Norte ' }, zone)).toEqual({
			starts_at: Date.UTC(2026, 10, 5, 20) / 1000
		});
		expect(scheduledChange(v, { date: '2026-11-04', time: '20:00', place: 'Padel Sul' }, zone)).toEqual({
			place: 'Padel Sul'
		});
	});

	it('clears the place with null', () => {
		expect(scheduledChange(v, { date: '2026-11-04', time: '20:00', place: '  ' }, zone)).toEqual({ place: null });
		expect(scheduledChange(view({ place: null }), { date: '2026-11-04', time: '20:00', place: '' }, 'UTC')).not.toEqual(
			{ place: null }
		);
	});
});

describe('scheduledMessage', () => {
	const err = (status: number, code: string) => new ApiError(status, code as never, code);

	it('says each code in sentence case', () => {
		for (const code of Object.keys(strings.scheduled.errors)) {
			expect(scheduledMessage(err(409, code))).toBe((strings.scheduled.errors as Record<string, string>)[code]);
		}
		expect(Object.keys(strings.scheduled.errors)).toEqual(
			expect.arrayContaining([
				'already_in',
				'line_full',
				'match_closed',
				'match_short',
				'too_many_scheduled',
				'not_yet',
				'game_over',
				'already_started'
			])
		);
	});

	it('names the field of a 422 and falls back to the sign-up wording', () => {
		const bad = new ApiError(422, 'validation', 'validation', false, { field: 'starts_at' });
		expect(scheduledMessage(bad)).toBe(strings.scheduled.fields.starts_at);
		expect(scheduledMessage(err(409, 'signups_closed'))).toBe(strings.signups.errors.signups_closed);
	});
});

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		return new Response(status === 204 ? null : JSON.stringify(body), {
			status,
			headers: { 'content-type': 'application/json' }
		});
	});
	return calls;
}

describe('the calls', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('create takes the 201 the server answers with', async () => {
		const calls = serve(201, view());
		const req = toNewScheduled(defaultSetup(), { startsAt: START, place: '', listing: 'private' });
		expect((await createScheduled(req)).id).toBe('s1');
		expect(calls[0]?.url).toBe('/api/scheduled');
		expect(calls[0]?.init?.method).toBe('POST');
	});

	it('owner routes go by id and join by the link', async () => {
		const calls = serve(200, view());
		await cancelScheduled('s 1');
		await removeFromScheduled('s1', 2);
		await joinMatch('tok');
		expect(calls.map((c) => c.url)).toEqual([
			'/api/scheduled/s%201/cancel',
			'/api/scheduled/s1/remove',
			'/api/join/tok'
		]);
		expect(JSON.parse(String(calls[1]?.init?.body))).toEqual({ slot: 2 });
		expect(JSON.parse(String(calls[2]?.init?.body))).toEqual({ as: 'solo' });
	});

	it('start gives the players for New match', async () => {
		serve(200, { players: [], format: null, from: { kind: 'scheduled', id: 's1' } });
		expect((await startScheduled('s1')).from).toEqual({ kind: 'scheduled', id: 's1' });
	});
});
