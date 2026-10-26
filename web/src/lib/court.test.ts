import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import {
	courtMatchText,
	courtScreen,
	finalRefusal,
	goneReason,
	isCourtState,
	JOINED_KEY_PREFIX,
	loadJoined,
	matchResultText,
	saveJoined
} from './court';
import { asView } from './fixtures/load';
import finishedJson from './fixtures/view-finished.json';
import newJson from './fixtures/view-new.json';
import onePointJson from './fixtures/view-one-point.json';
import type { StorageLike } from './setup';
import { strings } from './strings';
import type { CourtMatch, CourtState, EventTeam, EventViewMatch, MatchView } from './types';

const fresh = asView(newJson);
const onePoint = asView(onePointJson);
const finished = asView(finishedJson);

function match(view: MatchView, extra: Partial<CourtMatch> = {}): CourtMatch {
	return { id: view.id, label: 'Match 1', teams: view.teams, spectator_token: 'v1', scorer_token: 's1', ...extra };
}

function state(current: CourtMatch | null, next: CourtMatch | null = null): CourtState {
	return {
		event: { id: 'e1', name: 'Sunday cup', kind: 'tournament', status: 'running' },
		court: { id: 'c1', name: 'Court 1' },
		current,
		next,
		kind_data: null
	};
}

function memory(): StorageLike & { data: Map<string, string> } {
	const data = new Map<string, string>();
	return {
		data,
		getItem: (k) => data.get(k) ?? null,
		setItem: (k, v) => void data.set(k, v),
		removeItem: (k) => void data.delete(k)
	};
}

describe('courtScreen', () => {
	it('screen_join_before_tap', () => {
		expect(courtScreen(state(match(onePoint)), onePoint, false)).toBe('join');
		expect(courtScreen(state(null), null, false)).toBe('join');
	});

	it('screen_waiting_without_current', () => {
		expect(courtScreen(state(null), null, true)).toBe('waiting');
		// A queued match alone isn't one to score yet.
		expect(courtScreen(state(null, match(fresh)), null, true)).toBe('waiting');
	});

	it('screen_serve_on_empty_log', () => {
		expect(fresh.head_seq).toBe(0);
		expect(courtScreen(state(match(fresh)), fresh, true)).toBe('serve');
		// Once this phone chose the server, the scoring screen takes over before any point.
		expect(courtScreen(state(match(fresh)), fresh, true, fresh.id)).toBe('live');
		// A choice made for another match doesn't count.
		expect(courtScreen(state(match(fresh)), fresh, true, 'other')).toBe('serve');
	});

	it('screen_live', () => {
		expect(courtScreen(state(match(onePoint)), onePoint, true)).toBe('live');
	});

	it('screen_finished', () => {
		expect(courtScreen(state(match(finished)), finished, true)).toBe('finished');
		expect(courtScreen(state(match(finished), match(fresh, { id: 'm2' })), finished, true)).toBe('finished');
	});

	it('screen_gone', () => {
		// No court state at all: the link was replaced or the event finished.
		expect(courtScreen(null, null, true)).toBe('gone');
		expect(courtScreen(null, null, false)).toBe('gone');
	});

	it('screen_loading_until_the_current_match_view_arrives', () => {
		expect(courtScreen(state(match(onePoint)), null, true)).toBe('loading');
		// A view of the previous match is not this one's.
		expect(courtScreen(state(match(fresh, { id: 'm2' })), finished, true)).toBe('loading');
	});
});

describe('court helpers', () => {
	it('court_match_text_names_both_pairs', () => {
		expect(courtMatchText(match(fresh))).toBe('Ana / Rui v João / Marta');
	});

	it('gone_reason_from_court_errors', () => {
		expect(goneReason(new ApiError(404, 'not_found', ''))).toBe('replaced');
		expect(goneReason(new ApiError(410, 'event_finished', ''))).toBe('finished');
		expect(goneReason(new ApiError(0, 'network', '', true))).toBeNull();
		expect(goneReason(new ApiError(500, 'internal', ''))).toBeNull();
		expect(goneReason(new Error('x'))).toBeNull();
	});

	it('final_refusal_turns_outcomes_into_errors_for_the_sheet', () => {
		expect(finalRefusal({ status: 'applied', view: finished })).toBeNull();
		const conflict = finalRefusal({ status: 'conflict', view: onePoint });
		expect(conflict).toBeInstanceOf(ApiError);
		expect(conflict?.code).toBe('conflict');
		expect(conflict?.status).toBe(409);
		expect(finalRefusal({ status: 'locked', view: finished })?.code).toBe('locked');
		const invalid = finalRefusal({ status: 'invalid_score', reason: 'extra_sets' });
		expect(invalid?.code).toBe('invalid_score');
		expect(invalid?.status).toBe(422);
		expect(invalid?.reason).toBe('extra_sets');
	});

	it('joined_is_remembered_per_token', () => {
		const s = memory();
		expect(loadJoined(s, 'tok')).toBe(false);
		saveJoined(s, 'tok');
		expect(loadJoined(s, 'tok')).toBe(true);
		expect(loadJoined(s, 'other')).toBe(false);
		expect([...s.data.keys()]).toEqual([`${JOINED_KEY_PREFIX}tok`]);
	});

	it('joined_storage_failures_are_harmless', () => {
		const broken: StorageLike = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('blocked');
			},
			removeItem: () => {}
		};
		expect(loadJoined(broken, 'tok')).toBe(false);
		expect(() => saveJoined(broken, 'tok')).not.toThrow();
	});

	it('is_court_state_checks_the_frame', () => {
		expect(isCourtState(state(match(fresh)))).toBe(true);
		expect(isCourtState(state(null))).toBe(true);
		expect(isCourtState({ event: {}, court: { id: 'c' } })).toBe(false);
		expect(isCourtState(null)).toBe(false);
	});
});

describe('matchResultText', () => {
	const teams: EventTeam[] = [
		{
			id: 't1',
			name: 'Ana / Marta',
			custom_name: null,
			players: [
				{ id: 'p1', name: 'Ana' },
				{ id: 'p2', name: 'Marta' }
			]
		},
		{
			id: 't2',
			name: 'Rui / João',
			custom_name: null,
			players: [
				{ id: 'p3', name: 'Rui' },
				{ id: 'p4', name: 'João' }
			]
		}
	];
	const base: EventViewMatch = {
		id: 'm1',
		label: 'Match 1',
		ordinal: 1,
		team_a: 't1',
		team_b: 't2',
		court_id: 'c1',
		state: 'waiting',
		locked: false,
		winner: null,
		result: null,
		live: null,
		spectator_token: 'v1'
	};

	it('event_match_result_text', () => {
		expect(matchResultText(base, teams)).toBeNull();
		const result = {
			sets: [{ a: 6, b: 4, tiebreak: null }],
			games: [6, 4] as [number, number],
			regulation_games: [6, 4] as [number, number],
			overtime: false,
			timed: false
		};
		expect(matchResultText({ ...base, state: 'finished', winner: 'a', result }, teams)).toBe(
			'6-4 · Winner: Ana / Marta'
		);
		expect(matchResultText({ ...base, state: 'finished', winner: 'b', result }, teams)).toBe(
			'6-4 · Winner: Rui / João'
		);
		const live = onePoint.score;
		expect(matchResultText({ ...base, state: 'live', live }, teams)).toMatch(/^0-0, 15-0$/);
	});
});

describe('abandoned taps note', () => {
	it('abandoned_note_counts_taps', () => {
		expect(strings.court.abandoned(2)).toBe("2 taps for the last match weren't sent: the match left this court.");
		expect(strings.court.abandoned(1)).toBe("1 tap for the last match wasn't sent: the match left this court.");
	});
});
