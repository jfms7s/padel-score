// The logic behind the guest join form, kept apart from the component so it can be tested
// without a DOM.

import { boardMessage } from './boards';
import { joinAsGuest, pickGuest } from './guests';
import type { BoardView, MemberOut } from './types';

export type JoinResult = { view: BoardView } | { error: string };

/** The guests of the board, in member order: the names "I'm already on this board" offers. */
export function guestsToPick(view: BoardView): MemberOut[] {
	return view.members.filter((m) => m.guest);
}

async function settle(call: () => Promise<{ view: BoardView }>): Promise<JoinResult> {
	try {
		return { view: (await call()).view };
	} catch (e) {
		return { error: boardMessage(e) };
	}
}

/** Join the board as a new guest under the typed name. */
export function submitGuestName(token: string, name: string): Promise<JoinResult> {
	return settle(() => joinAsGuest(token, name.trim()));
}

/** Take the name of a guest already on the board. */
export function submitPick(token: string, guestId: string): Promise<JoinResult> {
	return settle(() => pickGuest(token, guestId));
}
