import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import { guestsToPick, submitGuestName, submitPick } from './guestjoin';
import * as guests from './guests';
import { strings } from './strings';
import type { BoardView, GuestJoined } from './types';

vi.mock('./guests', () => ({ joinAsGuest: vi.fn(), pickGuest: vi.fn() }));

const board = (members: { id: string; guest: boolean }[]): BoardView => ({
	board: {
		id: 'b',
		token: 't',
		title: 'Thursday padel',
		place: null,
		time_zone: 'UTC',
		starts_at: 0,
		ends_at: 7 * 86400,
		creator_user_id: 'ana',
		creator_name: 'Ana',
		over: false,
		allow_guests: true
	},
	me: null,
	members: members.map((m, i) => ({ user_id: m.id, name: m.id.toUpperCase(), guest: m.guest, joined_at: i })),
	pieces: [],
	games: [],
	requests: []
});

const joined = (view: BoardView): GuestJoined => ({ guest_key: 'k', view });

beforeEach(() => vi.resetAllMocks());

describe('guestsToPick', () => {
	it('lists only the guests, in member order', () => {
		const v = board([
			{ id: 'ana', guest: false },
			{ id: 'ze', guest: true },
			{ id: 'rui', guest: false },
			{ id: 'bia', guest: true }
		]);
		expect(guestsToPick(v).map((m) => m.user_id)).toEqual(['ze', 'bia']);
	});
});

describe('submitGuestName', () => {
	it('trims the name and returns the view', async () => {
		const view = board([{ id: 'ze', guest: true }]);
		vi.mocked(guests.joinAsGuest).mockResolvedValue(joined(view));
		expect(await submitGuestName('t', '  Zé ')).toEqual({ view });
		expect(guests.joinAsGuest).toHaveBeenCalledWith('t', 'Zé');
	});

	it('maps a refusal to its message', async () => {
		vi.mocked(guests.joinAsGuest).mockRejectedValue(new ApiError(409, 'guest_name_taken', 'x'));
		expect(await submitGuestName('t', 'Zé')).toEqual({ error: strings.boards.errors.guest_name_taken });
	});
});

describe('submitPick', () => {
	it('picks the guest and returns the view', async () => {
		const view = board([{ id: 'ze', guest: true }]);
		vi.mocked(guests.pickGuest).mockResolvedValue(joined(view));
		expect(await submitPick('t', 'ze')).toEqual({ view });
		expect(guests.pickGuest).toHaveBeenCalledWith('t', 'ze');
	});

	it('maps a refusal to its message', async () => {
		vi.mocked(guests.pickGuest).mockRejectedValue(new ApiError(409, 'guests_off', 'x'));
		expect(await submitPick('t', 'ze')).toEqual({ error: strings.boards.errors.guests_off });
	});
});
