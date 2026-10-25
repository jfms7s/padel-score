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

	// API failures.
	networkError: "Can't reach the server.",
	serverError: (status: number) => `The server answered ${status}.`
} as const;
