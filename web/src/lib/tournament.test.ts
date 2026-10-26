import { describe, expect, it } from 'vitest';
import { strings } from './strings';
import labelRows from './fixtures/tournament-labels.json';
import {
	bracketSections,
	canFillCourts,
	finishBlocker,
	heldText,
	slotText,
	sideScore,
	teamCountHint,
	tournamentData,
	whereText,
	type BracketNode,
	type TournamentData
} from './tournament';
import type { CourtOut, EventMatch, EventTeam, FinalScore, Score } from './types';

// ---- fixtures: the JSON the server writes (crates/server .../tournament/store.rs). ---

type P = [number, number]; // [place_from, size]
const pos = (place_from: number, size: number, idx: number) => ({ place_from, size, idx });

/** Every label the server writes (Layout::label), checked against Rust by tournament.rs. */
const LABELS = new Map<string, string>(
	(labelRows as { place_from: number; size: number; idx: number; label: string }[]).map((r) => [
		`${r.place_from},${r.size},${r.idx}`,
		r.label
	])
);

function labelOf([from, size]: P, idx: number): string {
	return LABELS.get(`${from},${size},${idx}`)!;
}

/** The draws of `teams` in Layout::positions order (size descending, place_from, idx). */
function draws(teams: number, full: boolean): P[] {
	const out: P[] = [];
	for (let size = teams; size >= 2; size /= 2) {
		for (let from = 1; from + size - 1 <= teams; from += size) {
			if (from === 1 || full || (from === 3 && size === 2)) out.push([from, size]);
		}
	}
	return out;
}

function tournament(teams: number, full: boolean): TournamentData {
	const nodes = draws(teams, full).flatMap(([from, size]) =>
		Array.from({ length: size / 2 }, (_, idx) => ({
			...pos(from, size, idx),
			label: labelOf([from, size], idx),
			teams: [null, null] as [string | null, string | null],
			from: [null, null] as [null, null],
			match_id: null,
			held: false
		}))
	);
	return { full_placement: full, teams, nodes, places: null, can_redraw: true } as TournamentData;
}

const sizes = (d: TournamentData) =>
	bracketSections(d).map((s) => ({ kind: s.kind, title: s.title, cols: s.columns.map((c) => [c.title, c.nodes.length]) }));

describe('bracketSections', () => {
	it('sections_for_eight_full', () => {
		const s = bracketSections(tournament(8, true));
		expect(s).toHaveLength(2);
		expect(s[0]!.kind).toBe('main');
		expect(s[0]!.title).toBeNull();
		expect(s[0]!.columns.map((c) => [c.title, c.nodes.length])).toEqual([
			['Quarter-finals', 4],
			['Semi-finals', 2],
			['Final and 3rd place', 2]
		]);
		expect(s[0]!.columns[2]!.nodes.map((n) => n.label)).toEqual(['Final', '3rd place']);
		expect(s[1]!.kind).toBe('placement');
		expect(s[1]!.title).toBe('Placement semis, 5th to 8th');
		expect(s[1]!.columns.map((c) => [c.title, c.nodes.length])).toEqual([
			['Placement semis', 2],
			['5th place and 7th place', 2]
		]);
	});

	it('sections_for_eight_off', () => {
		const s = bracketSections(tournament(8, false));
		expect(s).toHaveLength(1);
		expect(s[0]!.columns.map((c) => c.nodes.length)).toEqual([4, 2, 2]);
	});

	it('sections_for_four', () => {
		const s = bracketSections(tournament(4, true));
		expect(s).toHaveLength(1);
		expect(s[0]!.columns.map((c) => c.title)).toEqual(['Semi-finals', 'Final and 3rd place']);
	});

	it('sections_for_sixteen_full', () => {
		const s = sizes(tournament(16, true));
		expect(s).toHaveLength(3);
		expect(s[0]!.cols).toEqual([
			['Round of 16', 8],
			['Quarter-finals', 4],
			['Semi-finals', 2],
			['Final and 3rd place', 2]
		]);
		expect(s[1]).toMatchObject({ kind: 'placement', title: 'Placement semis, 5th to 8th' });
		expect(s[2]).toMatchObject({ kind: 'placement', title: 'Placement quarter-finals, 9th to 16th' });
		expect(s[2]!.cols).toEqual([
			['Placement quarter-finals', 4],
			['Placement semis', 4],
			['9th, 11th, 13th and 15th place', 4]
		]);
	});

	it('sections_for_thirty_two', () => {
		const s = sizes(tournament(32, true));
		expect(s.map((x) => x.title)).toEqual([
			null,
			'Placement semis, 5th to 8th',
			'Placement quarter-finals, 9th to 16th',
			'Placement round of 16, 17th to 32nd'
		]);
		expect(s[0]!.cols[0]).toEqual(['Round of 32', 16]);
		expect(sizes(tournament(32, false))).toHaveLength(1);
	});
});

const team = (id: string, a: string, b: string): EventTeam => ({
	id,
	name: `${a} / ${b}`,
	custom_name: null,
	players: [
		{ id: `${id}a`, name: a },
		{ id: `${id}b`, name: b }
	]
});
const teams = [team('t1', 'Leo', 'Sara'), team('t2', 'Duarte', 'Rita'), team('t3', 'Ana', 'Rui')];

function eight(): TournamentData {
	const d = tournament(8, true);
	const by = (label: string) => d.nodes.find((n) => n.label === label)!;
	by('Quarter-final 1').teams = ['t1', 't2'];
	by('Quarter-final 2').teams = ['t3', null];
	by('Semi-final 1').from = [
		{ winner_of: pos(1, 8, 0) },
		{ winner_of: pos(1, 8, 1) }
	];
	by('Semi-final 2').from = [{ winner_of: pos(1, 8, 2) }, { winner_of: pos(1, 8, 3) }];
	by('Placement semi 1 (5th–8th)').from = [{ loser_of: pos(1, 8, 0) }, { loser_of: pos(1, 8, 1) }];
	return d;
}

describe('slotText', () => {
	it('slot_text_known_or_either_or_source', () => {
		const d = eight();
		const by = (label: string): BracketNode => d.nodes.find((n) => n.label === label)!;
		expect(slotText(d, by('Quarter-final 1'), 0, teams)).toEqual({ text: 'Leo / Sara', known: true });
		// Both source teams known: either of them.
		expect(slotText(d, by('Semi-final 1'), 0, teams)).toEqual({ text: 'Leo / Sara or Duarte / Rita', known: false });
		// A source with an unknown team: its label.
		expect(slotText(d, by('Semi-final 1'), 1, teams)).toEqual({ text: 'Winner of quarter-final 2', known: false });
		expect(slotText(d, by('Semi-final 2'), 0, teams)).toEqual({ text: 'Winner of quarter-final 3', known: false });
		expect(slotText(d, by('Placement semi 1 (5th–8th)'), 1, teams)).toEqual({
			text: 'Loser of quarter-final 2',
			known: false
		});
		// Loser of a match whose teams are both known.
		expect(slotText(d, by('Placement semi 1 (5th–8th)'), 0, teams)).toEqual({ text: 'Leo / Sara or Duarte / Rita', known: false });
		// No team and no source: not drawn yet.
		expect(slotText(d, by('Quarter-final 4'), 0, teams)).toEqual({ text: strings.tournament.toBeDrawn, known: false });
	});
});

const score = (sets: [number, number][], games: [number, number] = [0, 0]): Score =>
	({ sets: sets.map(([a, b]) => ({ a, b, tiebreak: null })), games }) as unknown as Score;
const match = (over: Partial<EventMatch>): EventMatch =>
	({ id: 'm1', court_id: null, state: 'waiting', result: null, live: null, ...over }) as EventMatch;
const court = (id: string, name: string, current: string | null, queue: string[] = []): CourtOut =>
	({ id, name, position: 1, current, queue }) as CourtOut;

describe('whereText', () => {
	const node = (match_id: string | null): BracketNode => ({ ...eight().nodes[0]!, match_id });
	const courts = [court('c1', 'Court 1', 'm1', ['m2']), court('c2', 'Court 2', null, ['m3'])];

	it('where_text_variants', () => {
		expect(whereText(node('m1'), [match({ id: 'm1', court_id: 'c1', state: 'live' })], courts)).toEqual({
			text: 'Live on Court 1',
			live: true
		});
		// Current on the court but nobody started: next.
		expect(whereText(node('m1'), [match({ id: 'm1', court_id: 'c1', state: 'waiting' })], courts)).toEqual({
			text: 'Next on Court 1',
			live: false
		});
		// Queued behind the current one.
		expect(whereText(node('m2'), [match({ id: 'm2', court_id: 'c1' })], courts)).toEqual({
			text: 'Next on Court 1',
			live: false
		});
		expect(whereText(node('m4'), [match({ id: 'm4' })], courts)).toEqual({ text: 'Next free court', live: false });
		// Finished, no match yet, or an unknown match: nothing.
		expect(whereText(node('m1'), [match({ id: 'm1', court_id: 'c1', state: 'finished' })], courts)).toBeNull();
		expect(whereText(node(null), [], courts)).toBeNull();
		expect(whereText(node('zz'), [match({ id: 'm1' })], courts)).toBeNull();
	});
});

describe('sideScore', () => {
	it('side_score_result_and_live', () => {
		const result = (sets: FinalScore['sets'], games: [number, number]): FinalScore =>
			({ sets, games, regulation_games: games, overtime: false, timed: false }) as FinalScore;
		const set = (a: number, b: number, tiebreak: [number, number] | null = null) => ({ a, b, tiebreak });
		const one = match({ result: result([set(6, 4)], [6, 4]) });
		expect(sideScore(one, 'a')).toBe('6');
		expect(sideScore(one, 'b')).toBe('4');
		const three = match({ result: result([set(7, 6, [7, 3]), set(3, 6), set(1, 0, [10, 7])], [1, 0]) });
		expect(sideScore(three, 'a')).toBe('7 3 10');
		expect(sideScore(three, 'b')).toBe('6 6 7');
		// A timed result has no sets: the games.
		expect(sideScore(match({ result: result([], [5, 3]) }), 'b')).toBe('3');
		// Live: the sets so far, else the games.
		expect(sideScore(match({ state: 'live', live: score([[6, 3], [2, 1]]) }), 'a')).toBe('6 2');
		expect(sideScore(match({ state: 'live', live: score([], [4, 2]) }), 'b')).toBe('2');
		expect(sideScore(match({}), 'a')).toBe('');
	});
});

describe('teamCountHint', () => {
	it('team_count_hint', () => {
		for (const n of [4, 8, 16, 32]) expect(teamCountHint(n)).toBeNull();
		expect(teamCountHint(6)).toBe('6 teams don’t make a bracket: add 2 teams to make 8, or remove 2 teams to make 4.');
		expect(teamCountHint(3)).toBe('3 teams don’t make a bracket: add 1 team to make 4.');
		expect(teamCountHint(40)).toBe('40 teams don’t make a bracket: remove 8 teams to make 32.');
		expect(teamCountHint(7)).toBe('7 teams don’t make a bracket: add 1 team to make 8, or remove 3 teams to make 4.');
	});
});

describe('tournamentData', () => {
	it('tournament_data_rejects_junk', () => {
		expect(tournamentData(null)).toBeNull();
		expect(tournamentData(undefined)).toBeNull();
		expect(tournamentData({})).toBeNull();
		expect(tournamentData({ nodes: 3 })).toBeNull();
		expect(tournamentData('x')).toBeNull();
		const d = tournament(4, false);
		expect(tournamentData(d)).toBe(d);
	});
});

describe('strings', () => {
	it('every_tournament_error_has_copy', () => {
		const errors: Record<string, string> = strings.events.errors;
		for (const code of ['team_count', 'play_started', 'no_decision', 'bracket_unfinished', 'bracket_needs_decision']) {
			expect(errors[code], code).toEqual(expect.any(String));
			expect(errors[code]).not.toBe('');
		}
	});
});

describe('heldText', () => {
	const held = (discards: string[] | undefined): BracketNode => {
		const d = tournament(8, true);
		const n = d.nodes.find((x) => x.label === 'Quarter-final 1')!;
		n.held = true;
		if (discards) n.replay_discards = discards;
		return n;
	};

	it('held_warning_text', () => {
		const d = tournament(8, true);
		const n = held(['Semi-final 1', 'Final']);
		d.nodes.find((x) => x.label === 'Semi-final 1')!.match_id = 'm-sf1';
		const h = heldText(d, n, false, [{ id: 'm-sf1', state: 'live' }]);
		expect(h.warning).toBe('Quarter-final 1’s result changed after Semi-final 1 started.');
		expect(h.replay).toBe('Replay 2 matches: Semi-final 1, Final. Started matches lose their scores.');
		expect(h.keep).toBe('The later matches stand. The new score counts only for Quarter-final 1.');
		expect(h.finishedNote).toBeNull();
	});

	it('held_warning_names_the_started_match', () => {
		const d = tournament(8, true);
		const n = d.nodes.find((x) => x.label === 'Quarter-final 1')!;
		n.held = true;
		n.replay_discards = ['Placement semi 1 (5th–8th)', 'Semi-final 1', 'Final'];
		d.nodes.find((x) => x.label === 'Semi-final 1')!.match_id = 'm2';
		d.nodes.find((x) => x.label === 'Placement semi 1 (5th–8th)')!.match_id = 'm1';
		const matches = [
			{ id: 'm1', state: 'waiting' as const },
			{ id: 'm2', state: 'live' as const }
		];
		expect(heldText(d, n, false, matches).warning).toBe('Quarter-final 1’s result changed after Semi-final 1 started.');
		expect(heldText(d, n, false, [{ id: 'm1', state: 'waiting' }]).warning).toBe('Quarter-final 1’s result changed after a later match started.');
	});

	it('held_text_for_one_match_and_none', () => {
		const d = tournament(8, true);
		expect(heldText(d, held(['Final']), false).replay).toBe('Replay 1 match: Final. Started matches lose their scores.');
		const none = heldText(d, held(undefined), false);
		expect(none.replay).toBeNull();
		expect(none.warning).toBe('Quarter-final 1’s result changed after a later match started.');
	});

	it('held_after_finish_offers_only_keep', () => {
		const h = heldText(tournament(8, true), held(['Final']), true);
		expect(h.replay).toBeNull();
		expect(h.finishedNote).toBe('The event is finished, so later matches can’t be replayed.');
	});
});

describe('finishBlocker', () => {
	it('finish_reason', () => {
		const d = tournament(4, false);
		expect(finishBlocker(d)).toBe('unfinished');
		d.places = [{ team_id: 't1', place: 1, place_to: 1 }];
		expect(finishBlocker(d)).toBeNull();
		d.nodes[0]!.held = true;
		expect(finishBlocker(d)).toBe('decision');
		d.places = null;
		expect(finishBlocker(d)).toBe('decision');
	});
});

describe('final held back', () => {
	// Four teams with full placement: two semi-finals, the final and the 3rd-place match.
	const d = tournament(4, true);
	const fin = d.nodes.find((n) => n.place_from === 1 && n.size === 2)!;
	const others = d.nodes.filter((n) => n !== fin);
	const courtsOf = [{ id: 'c1', name: 'Court 1', current: null, queue: [] as string[] }];

	it('final_waits_for_the_other_matches', () => {
		fin.match_id = 'mf';
		others[0]!.match_id = 'm1';
		const ms = [
			{ id: 'mf', court_id: null, state: 'waiting' as const },
			{ id: 'm1', court_id: null, state: 'finished' as const }
		];
		// Another node has no match yet.
		expect(whereText(fin, ms, courtsOf, d)).toEqual({ text: 'After the other matches', live: false });
		expect(canFillCourts(d, ms, courtsOf)).toBe(false);
		// Every other node has a match, one unfinished on no court.
		others.slice(1).forEach((n, i) => (n.match_id = `mo${i}`));
		// A match the list doesn't have yet blocks, as the server's LEFT JOIN does.
		expect(whereText(fin, ms, courtsOf, d)?.text).toBe('After the other matches');
		const all = [
			...ms,
			...others.slice(1).map((n, i) => ({ id: n.match_id!, court_id: null, state: i === 0 ? ('waiting' as const) : ('finished' as const) }))
		];
		expect(whereText(fin, all, courtsOf, d)?.text).toBe('After the other matches');
		expect(canFillCourts(d, all, courtsOf)).toBe(true); // the waiting other match can go
		// All others finished or on a court: the final takes its turn.
		const done = all.map((m) => (m.id === 'mf' ? m : { ...m, state: 'finished' as const }));
		expect(whereText(fin, done, courtsOf, d)?.text).toBe('Next free court');
		expect(canFillCourts(d, done, courtsOf)).toBe(true);
		fin.match_id = null;
		others.forEach((n) => (n.match_id = null));
	});
});

describe('canFillCourts', () => {
	const d = tournament(4, false);
	d.nodes[0]!.match_id = 'm1';
	const m = (state: 'waiting' | 'live' | 'finished', court_id: string | null) => [{ id: 'm1', court_id, state }];
	const court = (current: string | null, queue: string[] = []) => ({ id: 'c1', name: 'Court 1', current, queue });

	it('fill_courts_offered', () => {
		expect(canFillCourts(d, m('waiting', null), [court(null)])).toBe(true);
		expect(canFillCourts(d, m('waiting', null), [court('x', [])].map((c) => ({ ...c, current: 'm1' })))).toBe(false);
		expect(canFillCourts(d, m('waiting', 'c1'), [court(null, ['m1'])])).toBe(false);
		expect(canFillCourts(d, m('waiting', null), [])).toBe(false);
	});
});
