// The fence screen's view model: everything the scoreboard shows, worked out from a MatchView.

import { slotName } from './setup';
import { strings } from './strings';
import type { DeuceRule, Format, MatchView, PointCall, ServeSide, SetScore, Team } from './types';

const CALLS: Record<PointCall, string> = {
	love: '0',
	fifteen: '15',
	thirty: '30',
	forty: '40',
	advantage: strings.advantageShort
};

export function pointLabel(call: PointCall): string {
	return CALLS[call];
}

/** One team's side of one set. */
export interface SetDetail {
	games: number;
	/**
	 * Tie-break points to show: on a finished set decided by a tie-break, the loser's points
	 * (the small "7-6⁵" figure, on the loser only); on a finished super tie-break, both teams'
	 * points, which are that set's score. Null otherwise, and always for the set in progress.
	 */
	tiebreak: number | null;
	/** The deciding set of a best of 3 played as a super tie-break (its games are 1–0). */
	superTieBreak: boolean;
	/** Whether this team won the set; null while it is being played. */
	won: boolean | null;
}

export interface TeamModel {
	/** "Ana / Rui". */
	name: string;
	/** The game call ("40", "AD") or, in a tie-break, the tie-break points. Empty once the match is over. */
	points: string;
	/** Games in the current set; for a timed match, the whole match, any overtime game included. */
	games: number;
	/** This team's games in each set, the one in progress last. Empty for a timed match. */
	sets: number[];
	/** The same sets in detail, for the scoreboard. */
	setDetails: SetDetail[];
	setsWon: number;
	/** Serves the next point. Never true once the match is over. */
	serving: boolean;
}

export interface FenceModel {
	a: TeamModel;
	b: TeamModel;
	/** Null once the match is over: nobody serves. */
	serveSide: ServeSide | null;
	/**
	 * The player serving the next point, by name. Null once the match is over, and for a quick
	 * match started without names (the name is still its slot's "Player N").
	 */
	servingPlayerName: string | null;
	/** Shown on the net when the next point decides the game. */
	callout: typeof strings.goldenPoint | typeof strings.starPoint | null;
	changeEnds: boolean;
	inTieBreak: boolean;
	superTieBreak: boolean;
	finished: { winner: Team | null } | null;
	/** "Set 2, golden point". */
	rulesLine: string;
}

export function deuceRule(format: Format): DeuceRule {
	return 'sets' in format ? format.sets.deuce : format.timed.deuce;
}

export function fenceModel(view: MatchView): FenceModel {
	const { score, teams, format } = view;
	const current = score.current;
	const game = typeof current === 'object' && 'game' in current ? current.game : null;
	const tieBreak = typeof current === 'object' && 'tie_break' in current ? current.tie_break : null;
	const finished = typeof score.status === 'object' ? score.status.finished : null;
	const deuce = deuceRule(format);

	const team = (t: Team): TeamModel => {
		const i = t === 'a' ? 0 : 1;
		return {
			name: strings.teamName(...teams[t]),
			points: game ? pointLabel(game[t]) : tieBreak ? String(tieBreak[t]) : '',
			games: score.games[i],
			sets: score.sets.map((s) => s[t]),
			setDetails: score.sets.map((s, n) => setDetail(s, t, n, n < score.sets.length - 1 || !!finished, format)),
			setsWon: score.sets_won[i],
			serving: !finished && score.server.team === t
		};
	};

	return {
		a: team('a'),
		b: team('b'),
		serveSide: finished ? null : score.side,
		servingPlayerName: finished ? null : enteredName(view),
		callout: game?.deciding_point ? decidingCallout(deuce) : null,
		changeEnds: score.change_ends,
		inTieBreak: tieBreak !== null,
		superTieBreak: tieBreak?.super_tiebreak ?? false,
		finished: finished ? { winner: finished.winner } : null,
		rulesLine: rulesLine(view, tieBreak !== null, tieBreak?.super_tiebreak ?? false, !!finished)
	};
}

function enteredName({ teams, score }: MatchView): string | null {
	const { team, index } = score.server;
	const name = teams[team][index];
	// The server fills a blank with "Player 1" / "Player 2" by position in the team, so that is no entry either.
	const placeholder = name === slotName(team, index) || name === strings.setup.player(index + 1);
	return name === undefined || placeholder ? null : name;
}

/** The index of the set played as a super tie-break, if the format has one: the deciding set. */
function superTieBreakSet(format: Format): number | null {
	if (!('sets' in format) || format.sets.deciding !== 'super_tie_break') return null;
	const maxSets = format.sets.sets === 'best_of_3' ? 3 : 1;
	return maxSets > 1 ? maxSets - 1 : null;
}

function setDetail(set: SetScore, t: Team, n: number, over: boolean, format: Format): SetDetail {
	const other: Team = t === 'a' ? 'b' : 'a';
	const superTieBreak = n === superTieBreakSet(format);
	if (!over) return { games: set[t], tiebreak: null, superTieBreak, won: null };
	const i = t === 'a' ? 0 : 1;
	const tb = set.tiebreak;
	const won = superTieBreak && tb ? tb[i] > tb[1 - i] : set[t] > set[other];
	const tiebreak = !tb ? null : superTieBreak ? tb[i] : won ? null : tb[i];
	return { games: set[t], tiebreak: tiebreak ?? null, superTieBreak, won };
}

function decidingCallout(deuce: DeuceRule): FenceModel['callout'] {
	switch (deuce) {
		case 'golden_point':
			return strings.goldenPoint;
		case 'star_point':
			return strings.starPoint;
		case 'advantage':
			return null;
	}
}

function rulesLine(view: MatchView, inTieBreak: boolean, superTieBreak: boolean, finished: boolean): string {
	if (finished) return strings.matchOver;
	if (superTieBreak) return strings.superTieBreak;
	const deuce = strings.deuceInLine[deuceRule(view.format)];
	if ('timed' in view.format) {
		return `${view.score.overtime ? strings.overtime : strings.nonStopName}, ${deuce}`;
	}
	const set = strings.setNumber(Math.max(view.score.sets.length, 1));
	return `${set}, ${inTieBreak ? strings.tieBreak : deuce}`;
}

export interface PageMode {
	/** Whether the page may score: tap buttons, undo, share, the offline queue. Spectators never can. */
	controls: boolean;
}

/** What a fence page may do, from its route. Only the scorer route (/s/…) has controls. */
export function pageMode(route: string): PageMode {
	return { controls: /^\/s\/[^/]+\/?$/.test(route) };
}
