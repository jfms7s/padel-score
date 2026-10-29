// Server-rendered: the first paint of the join form. Typing and submitting need a browser.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import { strings } from '#lib/strings.ts';
import type { BoardView } from '#lib/types.ts';
import GuestJoin from './GuestJoin.svelte';

const t = strings.boards;

const view: BoardView = {
	board: {
		id: 'b',
		token: 't',
		title: 'Thursday padel',
		place: null,
		time_zone: 'UTC',
		starts_at: 0,
		ends_at: 7 * 86400,
		creator_user_id: 'u1',
		creator_name: 'Ana',
		over: false,
		allow_guests: true
	},
	me: null,
	members: [
		{ user_id: 'u1', name: 'Ana Silva', guest: false, joined_at: 0 },
		{ user_id: 'g1', name: 'Zé Guest', guest: true, joined_at: 1 }
	],
	pieces: [],
	games: [],
	requests: []
};

const body = (): string => render(GuestJoin, { props: { view, token: 't', onjoined: () => {} } }).body;
const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');

describe('GuestJoin', () => {
	it('paints the name field and its join button', () => {
		const html = body();
		expect(html).toContain('<input');
		expect(text(html)).toContain(t.guestName);
		expect(text(html)).toContain(t.joinAsGuest);
		expect(text(html)).toContain(t.alreadyHere);
	});

	it('holds the guests to pick and no user names', () => {
		// The pick list is a <details>, so its content is in the first paint.
		const all = text(body());
		expect(all).toContain('Zé Guest');
		expect(all).not.toContain('Ana Silva');
	});
});
