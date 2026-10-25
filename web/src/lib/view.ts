// The fence screen's view model: everything the scoreboard shows, worked out from a MatchView.

import { strings } from './strings';
import type { DeuceRule, Format, MatchView, PointCall, ServeSide, Team } from './types';

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

export interface TeamModel {
	/** "Ana / Rui". */
	name: string;
	/** The game call ("40", "AD") or, in a tie-break, the tie-break points. Empty once the match is over. */
	points: string;
	/** Games in the current set; for a timed match, the whole match, any overtime game included. */
	games: number;
	/** This team's games in each set, the one in progress last. Empty for a timed match. */
	sets: number[];
	setsWon: number;
	/** Serves the next point. Never true once the match is over. */
	serving: boolean;
}

export interface FenceModel {
	a: TeamModel;
	b: TeamModel;
	/** Null once the match is over: nobody serves. */
	serveSide: ServeSide | null;
	/** Null once the match is over. */
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
			setsWon: score.sets_won[i],
			serving: !finished && score.server.team === t
		};
	};

	return {
		a: team('a'),
		b: team('b'),
		serveSide: finished ? null : score.side,
		servingPlayerName: finished ? null : (teams[score.server.team][score.server.index] ?? null),
		callout: game?.deciding_point ? decidingCallout(deuce) : null,
		changeEnds: score.change_ends,
		inTieBreak: tieBreak !== null,
		superTieBreak: tieBreak?.super_tiebreak ?? false,
		finished: finished ? { winner: finished.winner } : null,
		rulesLine: rulesLine(view, tieBreak !== null, tieBreak?.super_tiebreak ?? false, !!finished)
	};
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
		return `${view.score.overtime ? strings.overtime : strings.nonStop}, ${deuce}`;
	}
	const set = strings.setNumber(Math.max(view.score.sets.length, 1));
	return `${set}, ${inTieBreak ? strings.tieBreak : deuce}`;
}
