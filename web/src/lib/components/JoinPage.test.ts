// Server-rendered: while the session is still loading, neither "Sign in to …" nor the
// signed-in action may paint; the match page's three "Places" labels are told apart.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import { strings } from '#lib/strings.ts';
import type { EventJoinView, PartnerInvite, ScheduledView } from '#lib/types.ts';
import JoinEvent from './events/JoinEvent.svelte';
import JoinMatch from './JoinMatch.svelte';

const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');
const noop = () => {};

const match: ScheduledView = {
	id: 'm',
	join_token: 'tok',
	owner_name: 'Ana',
	starts_at: 4_000_000_000,
	place: null,
	listing: 'private',
	format: { sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' } },
	places: [0, 1, 2, 3].map((slot) => ({ slot, name: slot === 0 ? 'Ana' : null, user_id: null })),
	line: [],
	state: 'open',
	match_spectator_token: null,
	me: null
};

const matchProps = {
	view: match,
	origin: 'http://x',
	signedIn: false,
	sessionLoading: false,
	working: false,
	error: null,
	note: null,
	onjoin: noop,
	onleave: noop,
	onsignin: noop,
	onshare: noop,
	onstart: noop,
	oncancel: noop,
	onremove: noop,
	onedit: async () => true
};

const event: EventJoinView = {
	kind: 'event',
	event: {
		name: 'Cup',
		kind: 'tournament',
		format: matchProps.view.format,
		schedule: { starts_at: null, place: null, signups: 'public', min_teams: 2, max_teams: null, deadline_at: null },
		status: 'setup',
		organiser_name: 'Ana',
		closed: false
	},
	list: { hand_added: 0, teams: [], line: [], solos: [], pending: [] },
	me: null
};

const invite: PartnerInvite = { event_name: 'Cup', inviter_name: 'Bia', join_token: 'tok' };

const eventProps = {
	view: event,
	token: 'tok',
	origin: 'http://x',
	signedIn: false,
	sessionLoading: false,
	working: false,
	error: null,
	note: null,
	invite: null,
	inviteGone: false,
	inviteFailed: false,
	onretry: noop,
	onjoin: noop,
	onleave: noop,
	onsignin: noop,
	onshare: noop,
	onaccept: noop,
	ondecline: noop
};

describe('join page while the session loads', () => {
	it('match: shows the sign-in button once signed out, nothing while loading', () => {
		expect(text(render(JoinMatch, { props: matchProps }).body)).toContain(strings.scheduled.signInToJoin);
		const loading = text(render(JoinMatch, { props: { ...matchProps, sessionLoading: true } }).body);
		expect(loading).not.toContain(strings.scheduled.signInToJoin);
		expect(loading).not.toContain(strings.scheduled.join);
	});

	it('match: a signed-in viewer who could join sees no action while loading', () => {
		const props = {
			...matchProps,
			view: { ...match, me: { owner: false, slot: null, in_line: false } },
			signedIn: true
		};
		const ready = text(render(JoinMatch, { props }).body);
		expect(ready).toContain(strings.scheduled.join);
		const loading = text(render(JoinMatch, { props: { ...props, sessionLoading: true } }).body);
		expect(loading).not.toContain(strings.scheduled.join);
		expect(loading).not.toContain(strings.scheduled.signInToJoin);
	});

	it('event: shows the sign-in button once signed out, nothing while loading', () => {
		expect(text(render(JoinEvent, { props: eventProps }).body)).toContain(strings.signups.signInToJoin);
		const loading = text(render(JoinEvent, { props: { ...eventProps, sessionLoading: true } }).body);
		expect(loading).not.toContain(strings.signups.signInToJoin);
		expect(loading).not.toContain(strings.signups.joinAlone);
		expect(loading).not.toContain(`aria-label="${strings.signups.signupsLabel}"`);
	});

	it('invitation: no sign-in and no accept while loading', () => {
		const props = { ...eventProps, invite };
		expect(text(render(JoinEvent, { props }).body)).toContain(strings.signups.signInToAnswer);
		const loading = render(JoinEvent, { props: { ...props, sessionLoading: true } }).body;
		expect(text(loading)).not.toContain(strings.signups.signInToAnswer);
		expect(loading).not.toContain('<button');
	});
});

describe('match page labels', () => {
	it('gives the section, the list and the join card their own labels', () => {
		const html = render(JoinMatch, {
			props: { ...matchProps, view: { ...match, me: { owner: false, slot: null, in_line: false } }, signedIn: true }
		}).body;
		const labels = [...html.matchAll(/aria-label="([^"]*)"/g)].map((m) => m[1]);
		const section = text(html.match(/id="h-places"[^>]*>(.*?)<\/h2>/)![1]);
		const all = [section, ...labels];
		expect(new Set(all).size).toBe(all.length);
		expect(labels).toContain(strings.scheduled.placesListLabel);
		expect(labels).toContain(strings.scheduled.joinCardLabel);
	});
});
