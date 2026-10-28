import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import {
	acceptPartner,
	declinePartner,
	deadlineDefault,
	defaultDeadlineInputs,
	draftFromSchedule,
	getJoin,
	getPartnerInvite,
	getSignups,
	joinEvent,
	leaveJoin,
	maxChoices,
	minChoices,
	myEntryText,
	pairSignups,
	partnerLink,
	removeSignup,
	scheduleInput,
	scheduleText,
	signupMessage,
	splitSignup,
	takeTeams
} from './signups';
import { strings } from './strings';
import type { MyEntry } from './types';

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

afterEach(() => vi.unstubAllGlobals());

const err = (code: string, field?: string) =>
	new ApiError(code === 'validation' ? 422 : 409, code as never, 'x', false, field === undefined ? {} : { field });

describe('partnerLink', () => {
	it('partner_link_shape', () => {
		expect(partnerLink('https://padel.test', 'jt', 'pt')).toBe('https://padel.test/j/jt?partner=pt');
	});

	it('escapes the tokens', () => {
		expect(partnerLink('https://padel.test', 'a/b', 'c d')).toBe('https://padel.test/j/a%2Fb?partner=c%20d');
	});
});

describe('deadlineDefault', () => {
	const NOW = 1_800_000_000;
	const DAY = 86_400;

	it('deadline_default_mirrors_server: a day before when that is not in the past', () => {
		expect(deadlineDefault(NOW + 3 * DAY, NOW)).toBe(NOW + 2 * DAY);
		expect(deadlineDefault(NOW + DAY, NOW)).toBe(NOW);
	});

	it('is the start itself when it is less than a day away', () => {
		expect(deadlineDefault(NOW + DAY - 1, NOW)).toBe(NOW + DAY - 1);
		expect(deadlineDefault(NOW + 3600, NOW)).toBe(NOW + 3600);
	});
});

describe('maxChoices', () => {
	it('max_choices_tournament_and_non_stop', () => {
		expect(maxChoices('tournament', 1)).toEqual([4, 8, 16, 32]);
		expect(maxChoices('tournament', 3)).toEqual([4, 8, 16, 32]);
		const one = maxChoices('non_stop', 1);
		expect(one[0]).toBe(2);
		expect(one[one.length - 1]).toBe(22);
		expect(one.every((n) => n % 2 === 0)).toBe(true);
		expect(one).toHaveLength(11);
		expect(maxChoices('non_stop', 2).at(-1)).toBe(16);
		expect(maxChoices('non_stop', 3).at(-1)).toBe(12);
	});
});

describe('myEntryText', () => {
	it('my_entry_text_for_each_state', () => {
		const text = (e: MyEntry) => myEntryText(e);
		expect(text({ state: 'solo' })).toBe('Waiting for a partner');
		expect(text({ state: 'pending', partner_token: 'p' })).toBe('Waiting for your partner to accept');
		expect(text({ state: 'team', place: 3 })).toBe('In the event · place 3');
		expect(text({ state: 'line', place: 2 })).toBe('On the waiting list · 2');
	});
});

describe('signupMessage', () => {
	it('maps the sign-up codes to sentence-case copy', () => {
		const e = strings.signups.errors;
		for (const code of [
			'signups_closed',
			'signups_full',
			'signups_off',
			'signups_exist',
			'own_link',
			'already_signed_up',
			'not_solo',
			'signup_taken',
			'not_public',
			'not_auto_paired'
		] as const) {
			expect(signupMessage(err(code))).toBe(e[code]);
			expect(e[code]).not.toBe('');
		}
	});

	it('maps the schedule fields and falls back to the event copy', () => {
		for (const f of ['starts_at', 'place', 'signups', 'min_teams', 'max_teams', 'deadline_at', 'partner_name']) {
			expect(signupMessage(err('validation', f))).toBe(strings.signups.fields[f as 'place']);
		}
		expect(signupMessage(err('event_started'))).toBe(strings.events.errors.event_started);
		expect(signupMessage(err('event_cancelled'))).toBe(strings.events.errors.event_cancelled);
		expect(signupMessage(err('validation', 'name'))).toBe(strings.events.fields.name);
	});
});

describe('calls', () => {
	it('reads the join and partner links without forcing a sign-in', async () => {
		const calls = serve(200, { kind: 'event' });
		await getJoin('t/1');
		await getPartnerInvite('p 1');
		expect(calls.map((c) => c.url)).toEqual(['/api/join/t%2F1', '/api/partner/p%201']);
		expect(calls[0]?.init?.method ?? 'GET').toBe('GET');
	});

	it('joins alone or with a partner', async () => {
		const calls = serve(200, { state: 'solo' });
		await joinEvent('t', { as: 'solo' });
		await joinEvent('t', { as: 'partner', partner_name: 'Rui' });
		expect(calls.map((c) => [c.url, c.init?.method, c.init?.body])).toEqual([
			['/api/join/t', 'POST', JSON.stringify({ as: 'solo' })],
			['/api/join/t', 'POST', JSON.stringify({ as: 'partner', partner_name: 'Rui' })]
		]);
	});

	it('leaves, accepts and declines', async () => {
		const calls = serve(204, null);
		await leaveJoin('t');
		await declinePartner('p');
		expect(calls.map((c) => [c.url, c.init?.method])).toEqual([
			['/api/join/t', 'DELETE'],
			['/api/partner/p/decline', 'POST']
		]);
		const accepted = serve(200, { state: 'team', place: 1 });
		await acceptPartner('p');
		expect(accepted[0]?.url).toBe('/api/partner/p/accept');
	});

	it('runs the organiser calls', async () => {
		const calls = serve(200, {});
		await getSignups('e');
		await splitSignup('e', 's');
		await pairSignups('e', 'a', 'b');
		await takeTeams('e');
		const gone = serve(204, null);
		await removeSignup('e', 's');
		expect(calls.map((c) => [c.url, c.init?.method ?? 'GET'])).toEqual([
			['/api/events/e/signups', 'GET'],
			['/api/events/e/signups/s/split', 'POST'],
			['/api/events/e/signups/pair', 'POST'],
			['/api/events/e/take-teams', 'POST']
		]);
		expect(calls[2]?.init?.body).toBe(JSON.stringify({ entries: ['a', 'b'] }));
		expect([gone[0]?.url, gone[0]?.init?.method]).toEqual(['/api/events/e/signups/s', 'DELETE']);
	});
});

describe('scheduleText', () => {
	const WED_2000 = Date.UTC(2026, 10, 4, 20, 0) / 1000;
	const base = { place: null, signups: 'off', min_teams: 4, max_teams: null, deadline_at: null } as const;

	it('says when and where, leaving out what is missing', () => {
		expect(scheduleText({ ...base, starts_at: WED_2000, place: 'Club Norte' }, 'UTC')).toBe(
			'Wed 4 Nov 2026 · 20:00 · Club Norte'
		);
		expect(scheduleText({ ...base, starts_at: WED_2000 }, 'UTC')).toBe('Wed 4 Nov 2026 · 20:00');
		expect(scheduleText({ ...base, starts_at: null, place: 'Club Norte' }, 'UTC')).toBe('Club Norte');
		expect(scheduleText({ ...base, starts_at: null }, 'UTC')).toBe('');
	});
});

describe('the schedule draft', () => {
	const NOW = Date.UTC(2026, 10, 1, 12) / 1000;
	const START = Date.UTC(2026, 10, 10, 18, 30) / 1000;
	const empty = {
		starts_at: null,
		place: null,
		signups: 'off',
		min_teams: 4,
		max_teams: null,
		deadline_at: null
	} as const;

	it('starts empty for a new event', () => {
		const d = draftFromSchedule(null, 'UTC');
		expect(d).toMatchObject({ date: '', time: '', place: '', signups: 'off', max: null, deadlineTouched: false });
		expect(scheduleInput(d, null, 'UTC')).toEqual({ ok: undefined });
	});

	it('reads a saved schedule into the fields', () => {
		const d = draftFromSchedule(
			{ ...empty, starts_at: START, place: 'Club', signups: 'public', max_teams: 8, deadline_at: START - 3600 },
			'UTC'
		);
		expect(d).toMatchObject({
			date: '2026-11-10',
			time: '18:30',
			place: 'Club',
			signups: 'public',
			min: 4,
			max: 8,
			deadlineDate: '2026-11-10',
			deadlineTime: '17:30'
		});
	});

	it('sends a new date, a place and the sign-up settings', () => {
		const d = {
			...draftFromSchedule(null, 'UTC'),
			date: '2026-11-10',
			time: '18:30',
			place: ' Club ',
			signups: 'private' as const,
			max: 8
		};
		expect(scheduleInput(d, null, 'UTC')).toEqual({
			ok: { starts_at: START, place: 'Club', signups: 'private', min_teams: 4, max_teams: 8, deadline_at: null }
		});
	});

	it('sends a deadline only the organiser chose, and the default again when the start moved', () => {
		const current = {
			...empty,
			starts_at: START,
			signups: 'private',
			max_teams: 8,
			deadline_at: START - 86_400
		} as const;
		const d = draftFromSchedule(current, 'UTC');
		// Nothing touched: keep the deadline.
		expect(scheduleInput(d, current, 'UTC')).toEqual({
			ok: { starts_at: START, place: null, signups: 'private', min_teams: 4, max_teams: 8 }
		});
		// The start moved and the deadline was not touched: ask for the default.
		const moved = scheduleInput({ ...d, date: '2026-11-12' }, current, 'UTC');
		expect(moved).toMatchObject({ ok: { starts_at: START + 2 * 86_400, deadline_at: null } });
		// The organiser typed one.
		const chosen = scheduleInput(
			{ ...d, deadlineTouched: true, deadlineDate: '2026-11-09', deadlineTime: '10:00' },
			current,
			'UTC'
		);
		expect(chosen).toMatchObject({ ok: { deadline_at: Date.UTC(2026, 10, 9, 10) / 1000 } });
	});

	it('clears the date and the place when the fields are emptied', () => {
		const current = { ...empty, starts_at: START, place: 'Club' } as const;
		const d = { ...draftFromSchedule(current, 'UTC'), date: '', time: '', place: '' };
		expect(scheduleInput(d, current, 'UTC')).toEqual({
			ok: { starts_at: null, place: null, signups: 'off', min_teams: 4, max_teams: null }
		});
	});

	it('refuses half a date, and sign-ups without a date or a maximum', () => {
		const d = draftFromSchedule(null, 'UTC');
		expect(scheduleInput({ ...d, date: '2026-11-10' }, null, 'UTC')).toEqual({ error: 'bothDateAndTime' });
		expect(scheduleInput({ ...d, signups: 'public' }, null, 'UTC')).toEqual({ error: 'needsDate' });
		expect(scheduleInput({ ...d, date: '2026-11-10', time: '18:30', signups: 'public' }, null, 'UTC')).toEqual({
			error: 'max_teams'
		});
	});

	it('suggests the default deadline for a start', () => {
		const d = { ...draftFromSchedule(null, 'UTC'), date: '2026-11-10', time: '18:30' };
		expect(defaultDeadlineInputs(d, 'UTC', NOW)).toEqual({ date: '2026-11-09', time: '18:30' });
		expect(defaultDeadlineInputs({ ...d, date: '2026-11-01', time: '18:30' }, 'UTC', NOW)).toEqual({
			date: '2026-11-01',
			time: '18:30'
		});
		expect(defaultDeadlineInputs({ ...d, date: '' }, 'UTC', NOW)).toEqual({ date: '', time: '' });
	});
});

describe('minChoices', () => {
	it('runs from what the kind needs up to the maximum', () => {
		expect(minChoices('tournament', 8)).toEqual([4, 5, 6, 7, 8]);
		expect(minChoices('non_stop', 6)).toEqual([2, 3, 4, 5, 6]);
		expect(minChoices('tournament', 2)).toEqual([]);
	});
});
