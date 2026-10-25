// Every user-visible string in the app. English, sentence case, worded as in the mockup.

export const strings = {
	appName: 'Padel score',
	appDescription: 'Score padel matches from the fence, a phone or a watch.',

	// The deciding-point callout on the net (fence screen).
	goldenPoint: 'Golden point',
	starPoint: 'Star point',

	// The deuce rule as it reads inside the rules line ("Set 2, golden point").
	deuceInLine: {
		advantage: 'advantage',
		golden_point: 'golden point',
		star_point: 'star point'
	},
	setNumber: (n: number) => `Set ${n}`,
	tieBreak: 'tie-break',
	superTieBreak: 'Super tie-break',
	nonStop: 'Non-stop',
	overtime: 'Overtime',
	matchOver: 'Match over',

	teamName: (first: string, second: string) => `${first} / ${second}`,

	// The advantage call on the scoreboard.
	advantageShort: 'AD',

	// Home.
	newMatch: 'New match',
	recentMatches: 'Recent matches on this device',
	noRecent: 'Matches you start here will show up in this list.',
	homeIntro: 'Score a padel match from the fence. No account needed.',

	// New match.
	setup: {
		title: 'New match',
		back: 'Back',
		deuce: 'At 40–40',
		deuceOptions: { advantage: 'Advantage', golden_point: 'Golden point', star_point: 'Star point' },
		sets: 'Sets',
		setsOptions: { one: '1 set', best_of_3: 'Best of 3' },
		deciding: 'Deciding set',
		decidingOptions: { full_set: 'Full set', super_tie_break: 'Super tie-break' },
		decidingHint: 'Only for best of 3',
		teams: 'Teams',
		teamPlayer: (team: 'A' | 'B', n: number) => `Team ${team} player ${n}`,
		player: (n: number) => `Player ${n}`,
		firstServer: 'First to serve',
		firstServerHint: 'Not sure? Spin the racket and tap the winner.',
		otherFirstServer: 'First to serve for the other pair',
		start: 'Start match',
		starting: 'Starting…',
		failed: "Couldn't start the match.",
		created: 'Your match is ready.',
		openMatch: 'Open it',
		tryAgain: 'Try again'
	},

	// The change of ends cue.
	changeEnds: {
		short: 'Change ends',
		leadInSet: (name: string, hi: number, lo: number, set: number) =>
			`Change ends. ${name} lead ${hi}–${lo} in set ${set}.`,
		levelInSet: (a: number, b: number, set: number) => `Change ends. ${a}–${b} in set ${set}.`,
		tookSet: (name: string, set: number, hi: number, lo: number) =>
			`Change ends. ${name} took set ${set} ${hi}–${lo}.`,
		tieBreakLead: (name: string, hi: number, lo: number) =>
			`Change ends. ${name} lead ${hi}–${lo} in the tie-break.`,
		tieBreakLevel: (a: number, b: number) => `Change ends. ${a}–${b} in the tie-break.`,
		lead: (name: string, hi: number, lo: number) => `Change ends. ${name} lead ${hi}–${lo}.`,
		level: (a: number, b: number) => `Change ends. ${a}–${b}.`
	},

	// The fence screen.
	fence: {
		pointTo: (team: string) => `Point to ${team}`,
		pointShort: (player: string) => `Point ${player}`,
		undo: 'Undo',
		undoLabel: 'Undo last point',
		undoHint: 'Press and hold to undo',
		synced: 'Synced',
		live: 'Live',
		offlineView: 'Offline',
		offline: 'Offline, will sync',
		connecting: 'Connecting',
		sending: 'Sending',
		games: 'Games',
		sets: 'Sets',
		set: (n: number) => `Set ${n}`,
		superTieBreakShort: 'STB',
		serving: 'Serving',
		servingFrom: {
			right: 'serving from the right',
			left: 'serving from the left',
			receiver_chooses: 'receiver picks the side'
		},
		ballTitle: (player: string, side: 'right' | 'left' | 'centre') =>
			side === 'centre'
				? `${player} serves, the receiver picks the side`
				: `${player} serves from the ${side}, behind the baseline`,
		wins: (team: string) => `${team} win`,
		draw: 'Draw',
		newMatch: 'New match',
		share: 'Share',
		layout: { auto: 'Layout: auto', court: 'Layout: court', board: 'Layout: board' },
		layoutHint: 'Switch layout',
		loading: 'Loading the match',
		notFound: "This match doesn't exist, or it has expired.",
		loadFailed: "Couldn't load the match.",
		gone: 'This match no longer exists.',
		home: 'Home',
		rejected: {
			match_over: 'The match is over.',
			not_timed: 'This match has no clock.',
			already_time_up: 'Time is already up.',
			invalid_rules: 'The match rules are invalid.'
		},
		undoConflict: 'The score changed, so nothing was undone.',
		nothingToUndo: 'Nothing to undo.',
		failed: (message: string) => `Couldn't send that. ${message}`,
		lockedDropped: (n: number) => `The coordinator entered the result. ${n} queued point(s) dropped.`
	},

	// Taps made offline, and the choice when the server moved meanwhile.
	queue: {
		conflictTitle: 'Two different scores',
		conflictHint: 'This phone scored while offline, and the score on the server changed too. Pick the one to keep.',
		server: 'Score on the server',
		mine: 'Score on this phone',
		keepServer: "Keep the server's score",
		keepMine: "Keep this phone's score",
		saving: 'Saving…',
		mineUnknown: "This phone's taps can't be scored here.",
		discard: 'Discard these taps',
		lost: "Taps saved on this phone couldn't be read, so they were dropped.",
		full: "This phone can't hold more taps offline. Get back online to send them.",
		tooMany: 'Too many taps to send at once.',
		failed: (message: string) => `The server didn't take the taps made offline. ${message}`
	},

	// The share sheet.
	share: {
		title: 'Share this match',
		scoring: 'Scoring',
		viewing: 'Viewing',
		scoringHint: 'Anyone with this code can score.',
		viewingHint: 'Read only, for the bench and the bar.',
		copy: 'Copy link',
		copied: 'Copied',
		copyFailed: 'Copy the link above by hand.',
		close: 'Close',
		noViewing: 'The viewing link is on the phone that started the match.',
		qr: (what: string) => `QR code for the ${what.toLowerCase()} link`
	},

	// Accounts: sign-in, setup, invites, account, admin, watch approval, reset.
	auth: {
		signIn: 'Sign in',
		signOut: 'Sign out',
		signingIn: 'Signing in…',
		account: 'Account',
		header: 'Account menu',
		email: 'Email',
		password: 'Password',
		name: 'Your name',
		nameHint: 'Shown to the people you invite.',
		passwordHint: '8 to 128 characters.',
		back: 'Home',
		save: 'Save',
		saving: 'Saving…',
		loading: 'Loading…',
		signInFirst: 'Sign in to see this page.',

		login: {
			title: 'Sign in',
			setupFirst: 'This server has no accounts yet.',
			setupLink: 'Set it up'
		},

		setup: {
			title: 'Set up this server',
			intro: 'Create the first account. It will be the admin, and can invite everyone else.',
			code: 'Setup code',
			codeHint: 'Find it in the server log',
			submit: 'Create the admin account',
			done: 'This server is already set up.'
		},

		invite: {
			title: 'Join Padel score',
			invitedBy: (name: string) => `${name} invited you.`,
			expires: (date: string) => `The invite is good until ${date}.`,
			submit: 'Create my account',
			noCode: 'This invite link is incomplete. Ask for a new one.',
			signedInNote: 'Creating an account here signs you out of the one you use now.'
		},

		reset: {
			title: 'Choose a new password',
			newPassword: 'New password',
			submit: 'Set the password',
			done: 'Your password is set. Sign in with it now.',
			noCode: 'This reset link is incomplete. Ask an admin for a new one.'
		},

		accountPage: {
			title: 'Account',
			signedInAs: (name: string, email: string) => `Signed in as ${name} (${email}).`,
			admin: 'Manage users',
			link: 'Approve a watch',

			password: 'Change password',
			currentPassword: 'Current password',
			newPassword: 'New password',
			passwordChanged: 'Password changed. Your other devices were signed out.',

			invites: 'Invites',
			invitesHint: 'Each link makes one account and expires after 7 days.',
			createInvite: 'Create an invite link',
			newInvite: 'New invite link',
			newInviteHint: 'Send it to one person. It works once.',
			noInvites: "You haven't made any invites.",
			inviteStatus: {
				active: 'Waiting',
				used: 'Used',
				expired: 'Expired',
				revoked: 'Revoked'
			},
			usedBy: (name: string) => `Used by ${name}`,
			madeOn: (date: string) => `Made ${date}`,
			revoke: 'Revoke',
			revokeInvite: (date: string) => `Revoke the invite made ${date}`,

			devices: 'Watches',
			devicesHint: 'Watches signed in to your account. Revoke one you no longer use.',
			noDevices: 'No watches signed in.',
			lastUsed: (date: string) => `Last used ${date}`,
			neverUsed: 'Not used yet',
			revokeDevice: (name: string) => `Revoke ${name}`
		},

		adminPage: {
			title: 'Users',
			notAdmin: 'Only admins can see this page.',
			you: 'You',
			adminBadge: 'Admin',
			disabledBadge: 'Disabled',
			invitedBy: (name: string) => `Invited by ${name}`,
			firstUser: 'Set up the server',
			disable: 'Disable',
			enable: 'Enable',
			disableUser: (name: string) => `Disable ${name}`,
			enableUser: (name: string) => `Enable ${name}`,
			resetLink: 'Create reset link',
			resetLinkFor: (name: string) => `Create a reset link for ${name}`,
			resetTitle: (name: string) => `Reset link for ${name}`,
			resetHint: 'Shown only once. It works once, for 24 hours.'
		},

		link: {
			title: 'Approve a watch',
			intro: 'Type the code your watch shows.',
			code: 'Code on the watch',
			next: 'Continue',
			checking: 'Checking…',
			device: (name: string) => `${name} wants to sign in to your account.`,
			made: (ago: string) => `The code was made ${ago}.`,
			warning: 'Only approve a code shown on your own watch.',
			approve: 'Approve',
			deny: 'Deny',
			approved: 'Approved. Your watch is signing in.',
			denied: 'Denied. The watch was not signed in.',
			another: 'Approve another watch',
			notFound: 'No watch is waiting with this code. Check the code on the watch.'
		},

		// "made 2 minutes ago".
		ago: (secs: number) => {
			if (secs < 60) return 'just now';
			const mins = Math.floor(secs / 60);
			return mins === 1 ? '1 minute ago' : `${mins} minutes ago`;
		},

		share: {
			copy: 'Copy link',
			copied: 'Copied',
			copyFailed: 'Copy the link above by hand.',
			qr: 'QR code for the link'
		},

		errors: {
			invalid_credentials: 'Wrong email or password.',
			unauthorized: 'Your session ended. Sign in again.',
			forbidden: "You can't do that.",
			setup_code: "That setup code isn't right.",
			setup_done: 'This server is already set up.',
			email_taken: 'An account with this email already exists.',
			too_many_invites: 'You have too many open invites. Revoke one first.',
			invite_used: 'That invite was already used.',
			invite_invalid: 'This invite has expired, was revoked or was already used.',
			reset_invalid: 'This reset link has expired or was already used.',
			cannot_disable_self: "You can't disable your own account.",
			last_admin: "You can't disable the last admin.",
			device_code_used: 'This code was already used.',
			not_found: 'Not found.',
			tooMany: (mins: number) =>
				`Too many attempts. Try again in ${mins === 1 ? '1 minute' : `${mins} minutes`}.`,
			field: {
				email: 'Enter a valid email address.',
				display_name: 'Enter a name of 1 to 40 characters.',
				password: 'The password needs 8 to 128 characters.',
				new_password: 'The new password needs 8 to 128 characters.',
				other: 'Check the form and try again.'
			}
		}
	},

	// The player list, and picking players for a match.
	players: {
		title: 'Players',
		back: 'Home',
		intro: 'Pick these players when you start a match, and their wins and losses add up.',
		add: 'Add a player',
		addLabel: 'New player name',
		adding: 'Adding…',
		added: (name: string) => `${name} is on your list.`,
		restored: (name: string) => `${name} is back on your list.`,
		already: (name: string) => `${name} is already on your list.`,
		none: 'No players yet. Add the people you play with.',
		archived: 'Archived',
		archivedHint: 'Archived players keep their record, and are not offered for new matches.',
		rename: 'Rename',
		openLabel: (name: string) => `Open ${name}’s page`,
		renameLabel: (name: string) => `Rename ${name}`,
		newName: (name: string) => `New name for ${name}`,
		save: 'Save',
		cancel: 'Cancel',
		archive: 'Archive',
		archiveLabel: (name: string) => `Archive ${name}`,
		restore: 'Restore',
		restoreLabel: (name: string) => `Restore ${name}`,
		record: (wins: number, losses: number, draws: number) => `${wins} W · ${losses} L · ${draws} D`,
		// The picker in the new-match form.
		suggestions: (slot: string) => `Players for ${slot}`,
		addToList: (name: string) => `Add ‘${name}’ to my players`,
		fromList: 'From your players',
		listFailed: "Couldn't load your players, so names are typed only.",
		errors: {
			invalid_player: 'Enter a name of 1 to 40 characters.',
			player_exists: 'Another player already has this name.',
			not_found: 'This player is not on your list.'
		}
	},

	// The history page, a player's page, and the result row they share.
	history: {
		title: 'Past matches',
		link: 'History',
		back: 'Home',
		empty: 'Finished matches you started while signed in show up here.',
		loadMore: 'Load more',
		/** A timed match that went to overtime: the tied games, then this. */
		overtimeScore: (games: string) => `${games}, overtime`,
		winner: 'winner',
		draw: 'Draw',
		finalView: 'Final view',
		finalViewLabel: (a: string, b: string) => `Final view of ${a} v ${b}`,
		outcome: { win: 'Won', loss: 'Lost', draw: 'Drawn' },
		record: 'Record',
		/** The record's three numbers: the short label shown, and the word read out. */
		recordParts: [
			{ key: 'wins', short: 'W', long: 'Won' },
			{ key: 'losses', short: 'L', long: 'Lost' },
			{ key: 'draws', short: 'D', long: 'Drawn' }
		],
		placings: 'Placings',
		/** "Sunday cup · Match 1", on an event match. */
		eventMatch: (event: string, label: string) => `${event} · ${label}`,
		/** "Sunday cup: 3rd–4th of 12, tournament". */
		placingIn: (event: string, placing: string) => `${event}: ${placing}`,
		/** "3rd–4th of 12, tournament". */
		placing: (place: number, to: number, size: number, kind: 'tournament' | 'non_stop') =>
			`${to > place ? `${ordinal(place)}–${ordinal(to)}` : ordinal(place)} of ${size}, ${kind === 'tournament' ? 'tournament' : 'non-stop'}`,
		matches: 'Matches',
		noMatches: 'No finished matches with this player yet.',
		archived: 'Archived. Their record is kept.',
		backToPlayers: 'Players'
	},

	// Events: the coordinator's screens, the court phone and the event's viewing page.
	events: {
		versus: (a: string, b: string) => `${a} v ${b}`,
		unknownTeam: 'Unknown team',
		phoneConnected: (match: string) => `Phone connected, ${match}`,
		noPhone: 'No phone yet',
		waitingForMatch: 'Waiting for a match',

		link: 'Events',
		back: 'Home',
		title: 'Events',
		intro: 'Run a tournament or a non-stop: teams, courts with their own phones, and the results.',
		none: "You haven't made any events yet.",
		kinds: { tournament: 'Tournament', non_stop: 'Non-stop' },
		status: { setup: 'Setting up', running: 'Running', finished: 'Finished' },
		counts: (teams: number, courts: number) =>
			`${teams === 1 ? '1 team' : `${teams} teams`} · ${courts === 1 ? '1 court' : `${courts} courts`}`,
		openLabel: (name: string) => `Open ${name}`,

		// The new-event form.
		newEvent: 'New event',
		name: 'Name',
		namePlaceholder: 'Sunday cup',
		kind: 'Kind',
		rules: 'Rules',
		allowDraws: 'Allow draws',
		drawsOptions: { no: 'No', yes: 'Yes' },
		create: 'Create event',
		creating: 'Creating…',

		// The rules line: "Best of 3, super tie-break, golden point", "Timed, golden point, no draws".
		rulesDeciding: { full_set: 'full deciding set', super_tie_break: 'super tie-break' },
		timed: 'Timed',
		drawsAllowed: 'draws allowed',
		noDraws: 'no draws',

		// The event screen.
		allEvents: 'Events',
		loading: 'Loading the event…',
		streamLost: 'Live updates stopped.',
		reload: 'Reload',
		save: 'Save',
		cancel: 'Cancel',
		confirm: 'Confirm',

		teams: 'Teams',
		noTeams: 'No teams yet.',
		addTeam: 'Add team',
		teamPlayer: (n: number) => `Player ${n}`,
		teamName: 'Team name (optional)',
		pickBoth: 'Pick both players from your list, or add them to it.',
		samePlayer: 'Pick two different players.',
		rename: 'Rename',
		renameLabel: (team: string) => `Rename ${team}`,
		newTeamName: (team: string) => `New name for ${team}`,
		newTeamNameHint: 'Leave it empty to show the players’ names.',
		remove: 'Remove',
		removeLabel: (team: string) => `Remove ${team}`,
		noPlayers: 'Add players to your list first, or type a name and add it here.',

		courts: 'Courts',
		addCourt: 'Add court',
		scoring: 'Scoring',
		viewing: 'Viewing',
		scoringQr: (court: string) => `Scoring QR code for ${court}`,
		viewingQr: (court: string) => `Viewing QR code for ${court}`,
		copy: 'Copy link',
		copied: 'Copied',
		copyLabel: (place: string, what: string) => `Copy the ${what.toLowerCase()} link for ${place}`,
		copyFailed: 'Copy the link by hand.',
		rotate: 'New scoring code',
		rotateConfirm: 'The old scoring code stops working, and the phone using it must scan the new one.',
		nextMatch: 'Next match',
		wholeEvent: 'Whole event',
		wholeEventHint: 'Everything, for spectators. It can’t change a score.',
		wholeEventQr: 'Viewing QR code for the whole event',
		wholeEventPlace: 'the whole event',

		matches: 'Matches',
		noMatches: 'No matches yet.',
		newMatch: 'New match',
		teamA: 'Team A',
		teamB: 'Team B',
		court: 'Court',
		label: 'Label (optional)',
		pickTeam: 'Choose a team',
		sameTeam: 'Pick two different teams.',
		create_match: 'Create match',
		notOnCourt: 'Not on a court',
		moveLabel: (match: string) => `Court for ${match}`,
		matchState: { waiting: 'Waiting', live: 'Live', finished: 'Finished' },
		enteredByYou: 'Entered by you',
		enterResult: 'Enter result',
		enterResultLabel: (match: string) => `Enter the result of ${match}`,
		view: 'View',
		viewLabel: (match: string) => `View ${match}`,
		liveScore: (games: string, point: string) => `${games}, ${point}`,
		draw: 'Draw',

		start: 'Start event',
		starting: 'Starting…',
		startHint: 'Teams can’t change once it starts.',
		finish: 'Finish event',
		finishTitle: 'Final positions',
		finishHint: 'Put the teams in finishing order. Mark teams that share a place as tied.',
		moveUp: (team: string) => `Move ${team} up`,
		moveDown: (team: string) => `Move ${team} down`,
		up: 'Up',
		down: 'Down',
		tied: 'Tied',
		tiedLabel: (a: string, b: string) => `${a} and ${b} share a place`,
		finishConfirm: 'Save the positions and finish',
		finishing: 'Finishing…',
		place: (place: number, to: number) => (to > place ? `${ordinal(place)}–${ordinal(to)}` : ordinal(place)),

		// The typed result sheet.
		sheet: {
			title: (match: string) => `Result of ${match}`,
			field: 'Final score',
			hintSets: 'Like 6-4 3-6 10-7. Write a tie-break set as 7-6(5), with the loser’s points.',
			hintTimed: 'Games, like 5-3.',
			placeholderSets: '6-4 3-6 10-7',
			placeholderTimed: '5-3',
			overtime: 'Level: who won the overtime game?',
			overtimeWon: (team: string) => `${team} won it`,
			winner: (team: string) => `Winner: ${team}`,
			draw: 'Result: draw',
			save: 'Save result',
			saving: 'Saving…',
			cancel: 'Cancel',
			conflict: 'The score changed meanwhile. Check it and try again.',
			locked: 'The coordinator already entered this result.'
		},
		parsed: {
			empty: 'Type the final score.',
			syntax: 'Write the score as games, like 6-4 or 6-4 3-6 10-7.',
			needs_tiebreak: 'Add the tie-break loser’s points to a 7-6 set, like 7-6(5).',
			needs_overtime_winner: 'The games are level: pick who won the overtime game.'
		},
		invalidScore: {
			super_tie_break_needs_multiple_sets: 'A super tie-break needs best of 3. Check the event’s rules.',
			invalid_player_index: 'The match rules are invalid.',
			wrong_format: 'That score doesn’t fit this match’s rules.',
			impossible_set: (set: number) => `Set ${set} can’t end like that.`,
			missing_tie_break: (set: number) => `Set ${set} needs its tie-break, like 7-6(5).`,
			match_not_finished: 'Nobody has won yet with that score.',
			extra_sets: 'There are sets after the match was already won.',
			draw_not_allowed: 'This match can’t end in a draw.',
			overtime_winner_when_not_level: 'Only a level score has an overtime game.'
		},

		errors: {
			already_started: 'This match has already started.',
			event_started: 'The event has started, so its teams can’t change.',
			event_not_running: 'Matches can only be made while the event is running.',
			event_finished: 'The event is finished.',
			too_few_teams: 'Add at least two teams first.',
			no_courts: 'Add a court first.',
			too_many_teams: 'An event can have at most 64 teams.',
			too_many_courts: 'An event can have at most 16 courts.',
			player_in_team: 'One of these players is already in a team.',
			court_in_use: 'A match is on this court.',
			match_not_finished: 'The match on this court isn’t finished yet.',
			match_finished: 'This match is finished.',
			conflict: 'The score changed meanwhile. Try again.',
			locked: 'You already entered this result.'
		},
		fields: {
			name: 'Enter a name of 1 to 40 characters.',
			label: 'Enter a label of 1 to 40 characters.',
			format: 'Those rules don’t suit this kind of event.',
			kind: 'Choose a kind of event.',
			players: 'Pick two different players from your list.',
			team_a: 'Choose team A.',
			team_b: 'Choose team B.',
			court_id: 'Choose a court of this event.',
			places: 'Every team needs a place, and the places must add up.',
			settings: 'Check the event’s settings.'
		}
	},

	// The court phone (/c/{token}), the court viewer (/cv/{token}) and the event page (/e/{token}).
	court: {
		loading: 'Loading the court…',
		join: (court: string) => `Join ${court}`,
		joinHint: 'This phone scores the matches on this court. Keep it at the fence.',
		waiting: 'Waiting for a match',
		upNext: (match: string) => `Up next: ${match}`,
		next: (match: string) => `Next: ${match}`,
		nextMatch: (match: string) => `Next match: ${match}`,
		waitingForCoordinator: 'Waiting for the coordinator',
		typeFinal: 'Type the final score',
		locked: 'Result entered by the coordinator',
		replaced: 'This court link has been replaced. Scan the new code.',
		finished: 'This event has finished.',
		loadFailed: 'Couldn’t load this court. Check the connection and reload.',
		reload: 'Reload',
		serveTitle: 'Who serves first?',
		serveHint: 'Not sure? Spin the racket and tap the winner.',
		otherServer: 'First to serve for the other pair',
		startScoring: 'Start scoring',
		starting: 'Starting…',
		changeServer: 'Change who serves',
		startFailed: 'Couldn’t save who serves. Try again.',
		advanceFailed: 'Couldn’t move to the next match. Try again.',
		matchLabel: (label: string, match: string) => `${label}: ${match}`,
		// The court viewer.
		viewerLoading: 'Loading the court…',
		// The event page.
		eventLoading: 'Loading the event…',
		courts: 'Courts',
		courtIdle: 'No match on court',
		watch: 'Watch',
		watchLabel: (court: string) => `Watch ${court}`,
		matches: 'Matches',
		noMatches: 'No matches yet.',
		finalPositions: 'Final positions',
		eventGone: 'This event link doesn’t work any more.'
	},

	// Home, signed in.
	yourPages: 'Your players and history',
	myMatches: 'My matches',
	noMyMatches: 'Matches you start while signed in show up here, on any device.',

	// API failures.
	signedOutPlayers: "You've been signed out. Sign in again to use your players.",
	networkError: "Can't reach the server.",
	serverError: (status: number) => `The server answered ${status}.`
} as const;

/** 1st, 2nd, 3rd, 4th, 11th, 21st. */
function ordinal(n: number): string {
	const tens = n % 100;
	if (tens >= 11 && tens <= 13) return `${n}th`;
	return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
}
