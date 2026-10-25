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

	// API failures.
	networkError: "Can't reach the server.",
	serverError: (status: number) => `The server answered ${status}.`
} as const;
