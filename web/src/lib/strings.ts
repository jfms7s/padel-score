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
		failed: (message: string) => `Couldn't send that. ${message}`
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

	// API failures.
	networkError: "Can't reach the server.",
	serverError: (status: number) => `The server answered ${status}.`
} as const;
