// Event dates and sign-ups: the calls of the join page and the organiser's Sign-ups tab, the
// schedule form's pure parts (the draft, what it sends) and the copy helpers they share.
//
// `getJoin` and `getPartnerInvite` work signed out (a link opens for anyone); every write and
// the organiser's calls are signed-in calls, so a 401 sends the browser to sign-in.

import { accountRequest, ApiError } from './api';
import { dayLabel, fromLocalInputs, monthYear, timeText, toLocalInputs } from './dates';
import { eventMessage } from './events';
import { strings } from './strings';
import type {
	EventKind,
	JoinAs,
	JoinView,
	MyEntry,
	PartnerInvite,
	Schedule,
	ScheduleInput,
	SignupList,
	SignupMode,
	TakeReport
} from './types';

const enc = encodeURIComponent;
const signedIn = { signedIn: true } as const;
const join = (token: string) => `/api/join/${enc(token)}`;
const partner = (token: string, rest = '') => `/api/partner/${enc(token)}${rest}`;
const signups = (eventId: string, rest = '') => `/api/events/${enc(eventId)}/signups${rest}`;

// ---- calls -------------------------------------------------------------------------------

/** Anyone with the link; signed out, `me` is null. 404 for an unknown link or sign-ups off. */
export function getJoin(token: string): Promise<JoinView> {
	return accountRequest(join(token));
}

/** Alone, or with a partner: by name (a complete team) or, without one, by a link to send. */
export function joinEvent(token: string, as: JoinAs): Promise<MyEntry> {
	return accountRequest(join(token), { ...signedIn, method: 'POST', body: as });
}

/** Not once the sign-up is a team of the event (409 `signup_taken`). */
export async function leaveJoin(token: string): Promise<void> {
	await accountRequest(join(token), { ...signedIn, method: 'DELETE', ok: 204 });
}

/** Who invites you; anyone with the link can read it. */
export function getPartnerInvite(partnerToken: string): Promise<PartnerInvite> {
	return accountRequest(partner(partnerToken));
}

/** 409 `own_link`, `already_signed_up`, `signups_closed`. */
export function acceptPartner(partnerToken: string): Promise<MyEntry> {
	return accountRequest(partner(partnerToken, '/accept'), { ...signedIn, method: 'POST' });
}

/** The inviter gets a fresh link. */
export async function declinePartner(partnerToken: string): Promise<void> {
	await accountRequest(partner(partnerToken, '/decline'), { ...signedIn, method: 'POST', ok: 204 });
}

export function getSignups(eventId: string): Promise<SignupList> {
	return accountRequest(signups(eventId), signedIn);
}

/** 409 `signup_taken` for an entry that is already a team of the event. */
export async function removeSignup(eventId: string, entryId: string): Promise<void> {
	await accountRequest(signups(eventId, `/${enc(entryId)}`), { ...signedIn, method: 'DELETE', ok: 204 });
}

/** An automatic pair becomes two held solos (409 `not_auto_paired`). */
export function splitSignup(eventId: string, entryId: string): Promise<SignupList> {
	return accountRequest(signups(eventId, `/${enc(entryId)}/split`), { ...signedIn, method: 'POST' });
}

/** Two people waiting for a partner (409 `not_solo`). */
export function pairSignups(eventId: string, a: string, b: string): Promise<SignupList> {
	return accountRequest(signups(eventId, '/pair'), { ...signedIn, method: 'POST', body: { entries: [a, b] } });
}

/** Makes teams of the complete sign-ups and closes sign-ups. */
export function takeTeams(eventId: string): Promise<TakeReport> {
	return accountRequest(`/api/events/${enc(eventId)}/take-teams`, { ...signedIn, method: 'POST' });
}

/** What to tell the user about a failed sign-up call. */
export function signupMessage(e: unknown): string {
	if (e instanceof ApiError) {
		const errors: Record<string, string> = strings.signups.errors;
		const fields: Record<string, string> = strings.signups.fields;
		if (e.code === 'validation' && e.field !== undefined && Object.hasOwn(fields, e.field)) return fields[e.field]!;
		if (Object.hasOwn(errors, e.code)) return errors[e.code]!;
	}
	return eventMessage(e);
}

// ---- links and copy ----------------------------------------------------------------------

/** The link a user sends their partner: the event's join page with the partner token. */
export function partnerLink(origin: string, joinToken: string, partnerToken: string): string {
	return `${origin}/j/${enc(joinToken)}?partner=${enc(partnerToken)}`;
}

/** The event's sign-up link. */
export function joinLink(origin: string, joinToken: string): string {
	return `${origin}/j/${enc(joinToken)}`;
}

export function myEntryText(e: MyEntry): string {
	const t = strings.signups.entry;
	switch (e.state) {
		case 'solo':
			return t.solo;
		case 'pending':
			return t.pending;
		case 'team':
			return t.team(e.place);
		case 'line':
			return t.line(e.place);
	}
}

/** "Wed 4 Nov 2026 · 20:00 · Club Norte", leaving out what the event does not say. */
export function scheduleText(s: Schedule, timeZone?: string): string {
	const parts: string[] = [];
	if (s.starts_at !== null) {
		parts.push(`${dayLabel(s.starts_at, timeZone)} ${monthYear(s.starts_at, timeZone)}`);
		parts.push(timeText(s.starts_at, timeZone));
	}
	if (s.place !== null) parts.push(s.place);
	return parts.join(' · ');
}

// ---- the schedule form -------------------------------------------------------------------

/** The sign-up deadline when none is chosen: a day before, or the start itself when nearer. */
export function deadlineDefault(startsAt: number, now: number): number {
	return startsAt - 86_400 >= now ? startsAt - 86_400 : startsAt;
}

/** The team maximums an event can take (the server refuses others with 422 `max_teams`). */
export function maxChoices(kind: EventKind, legs: number): number[] {
	if (kind === 'tournament') return [4, 8, 16, 32];
	const most = legs >= 3 ? 12 : legs === 2 ? 16 : 22;
	return Array.from({ length: most / 2 }, (_, i) => (i + 1) * 2);
}

/** The minimums that fit under `max`: a bracket needs 4 teams at least, a non-stop 2. */
export function minChoices(kind: EventKind, max: number): number[] {
	const floor = kind === 'tournament' ? 4 : 2;
	return Array.from({ length: Math.max(0, max - floor + 1) }, (_, i) => floor + i);
}

/** The schedule form's fields, as the date and time inputs hold them. */
export interface ScheduleDraft {
	date: string;
	time: string;
	place: string;
	signups: SignupMode;
	min: number;
	max: number | null;
	deadlineDate: string;
	deadlineTime: string;
	/** The organiser chose the deadline: it is sent as it is. Otherwise the server's default. */
	deadlineTouched: boolean;
}

export function draftFromSchedule(s: Schedule | null, timeZone?: string): ScheduleDraft {
	const start = s?.starts_at == null ? null : toLocalInputs(s.starts_at, timeZone);
	const deadline = s?.deadline_at == null ? null : toLocalInputs(s.deadline_at, timeZone);
	return {
		date: start?.date ?? '',
		time: start?.time ?? '',
		place: s?.place ?? '',
		signups: s?.signups ?? 'off',
		min: s?.min_teams ?? 4,
		max: s?.max_teams ?? null,
		deadlineDate: deadline?.date ?? '',
		deadlineTime: deadline?.time ?? '',
		deadlineTouched: false
	};
}

/** The deadline fields' values for the draft's start; empty without a start. */
export function defaultDeadlineInputs(
	d: Pick<ScheduleDraft, 'date' | 'time'>,
	timeZone: string | undefined,
	now: number
): { date: string; time: string } {
	if (d.date === '' || d.time === '') return { date: '', time: '' };
	return toLocalInputs(deadlineDefault(fromLocalInputs(d.date, d.time, timeZone), now), timeZone);
}

export type DraftError = 'bothDateAndTime' | 'needsDate' | 'max_teams';

/**
 * What the draft asks the server to change, or why it can't be asked yet. `current` is the
 * saved schedule (null for a new event). A new event with nothing filled in sends no schedule
 * at all (`ok: undefined`).
 */
export function scheduleInput(
	d: ScheduleDraft,
	current: Schedule | null,
	timeZone: string | undefined
): { ok: ScheduleInput | undefined } | { error: DraftError } {
	if ((d.date === '') !== (d.time === '')) return { error: 'bothDateAndTime' };
	const startsAt = d.date === '' ? null : fromLocalInputs(d.date, d.time, timeZone);
	const place = d.place.trim() === '' ? null : d.place.trim();
	if (current === null && startsAt === null && place === null && d.signups === 'off') return { ok: undefined };
	if (d.signups !== 'off') {
		if (startsAt === null) return { error: 'needsDate' };
		if (d.max === null) return { error: 'max_teams' };
	}
	const input: ScheduleInput = {
		starts_at: startsAt,
		place,
		signups: d.signups,
		min_teams: d.min,
		max_teams: d.max
	};
	if (d.signups !== 'off' && startsAt !== null) {
		if (d.deadlineTouched) input.deadline_at = fromLocalInputs(d.deadlineDate, d.deadlineTime, timeZone);
		else if (current === null || current.starts_at !== startsAt) input.deadline_at = null;
	}
	return { ok: input };
}
