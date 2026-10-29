// The Organise page's rows: events, scheduled matches and boards the user runs, each with the
// segment it sits in and the one thing that needs doing.

import { accountRequest } from './api';
import { longDate, timeText } from './dates';
import { strings } from './strings';
import type { Organised, OrganisedBoard, OrganisedEvent, OrganisedMatch } from './types';

export type Segment = 'coming' | 'running' | 'past';

export interface OrganiseRow {
	kind: 'event' | 'match' | 'board';
	/** `event:{id}`, `match:{id}` or `board:{token}`. */
	key: string;
	title: string;
	when: string;
	place: string | null;
	chip: string;
	/** The one thing that needs doing, or null. */
	needs: string | null;
	href: string;
	segment: Segment;
	sortAt: number;
}

const s = strings.organise;
const enc = encodeURIComponent;
/** An open or full match is past this long after its start, even before the server says so. */
const STALE_MATCH_SECS = 3 * 3600;

export function getOrganised(): Promise<Organised> {
	return accountRequest('/api/me/organised', { signedIn: true });
}

const whenText = (t: number, timeZone?: string) => `${longDate(t, timeZone)} · ${timeText(t, timeZone)}`;

function eventRow(e: OrganisedEvent, now: number, timeZone?: string): OrganiseRow {
	const { starts_at, place, signups, deadline_at } = e.schedule;
	const segment: Segment = e.status === 'setup' ? 'coming' : e.status === 'running' ? 'running' : 'past';
	let needs: string | null = null;
	if (segment === 'coming') {
		if (starts_at !== null && now >= starts_at) needs = s.startOverdue;
		else if (e.solo_count > 0) needs = s.solo(e.solo_count);
		else if (signups !== 'off' && deadline_at !== null && deadline_at > now) {
			needs = s.signupsClose(longDate(deadline_at, timeZone));
		} else if (e.line_count > 0) needs = s.inLine(e.line_count);
	}
	return {
		kind: 'event',
		key: `event:${e.id}`,
		title: e.name,
		when: starts_at === null ? s.noDate : whenText(starts_at, timeZone),
		place,
		chip: e.status === 'setup' && signups !== 'off' ? s.signups : strings.events.status[e.status],
		needs,
		href: `/events/${enc(e.id)}`,
		segment,
		sortAt: starts_at ?? e.created_at
	};
}

function matchRow(m: OrganisedMatch, now: number, timeZone?: string): OrganiseRow {
	const stale = (m.state === 'open' || m.state === 'full') && now >= m.starts_at + STALE_MATCH_SECS;
	const past = m.state === 'cancelled' || m.state === 'started' || m.state === 'past' || stale;
	let needs: string | null = null;
	if (!past) {
		const openPlaces = m.places.filter((p) => p === null).length;
		if (openPlaces > 0) needs = s.moreNeeded(openPlaces);
		else if (m.line_count > 0) needs = s.inLine(m.line_count);
	}
	return {
		kind: 'match',
		key: `match:${m.id}`,
		title: m.place === null ? s.match : `${s.match} · ${m.place}`,
		when: whenText(m.starts_at, timeZone),
		place: m.place,
		chip: strings.calendar.matchState[m.state],
		needs,
		href: `/j/${enc(m.join_token)}`,
		segment: past ? 'past' : 'coming',
		sortAt: m.starts_at
	};
}

function boardRow(b: OrganisedBoard, timeZone?: string): OrganiseRow {
	return {
		kind: 'board',
		key: `board:${b.token}`,
		title: b.title,
		when: whenText(b.starts_at, timeZone),
		place: b.place,
		chip: s.board,
		needs: b.over ? null : b.game_count === 0 ? s.noGames : s.gamesFormed(b.game_count),
		href: `/b/${enc(b.token)}`,
		segment: b.over ? 'past' : 'coming',
		sortAt: b.starts_at
	};
}

/** Every row, unsorted. `now` is unix seconds. */
export function organiseRows(o: Organised, now: number, timeZone?: string): OrganiseRow[] {
	return [
		...o.events.map((e) => eventRow(e, now, timeZone)),
		...o.matches.map((m) => matchRow(m, now, timeZone)),
		...o.boards.map((b) => boardRow(b, timeZone))
	];
}

/** Coming up and running by start ascending; past newest first. */
export function bySegment(rows: OrganiseRow[], seg: Segment): OrganiseRow[] {
	const inSeg = rows.filter((r) => r.segment === seg);
	return inSeg.sort((a, b) => (seg === 'past' ? b.sortAt - a.sortAt : a.sortAt - b.sortAt));
}

export function segmentFromQuery(search: URLSearchParams): Segment {
	const seg = search.get('seg');
	return seg === 'running' || seg === 'past' ? seg : 'coming';
}
