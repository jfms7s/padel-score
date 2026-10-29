<script lang="ts">
	// The sheet of one game: its four players in order, the line, the "moved" mark and what the
	// viewer can do (Leave, Give my place, Swap, Change time, Take the open place, Start match).
	// Every call sends the game's `version`; the page runs it and reads the board again, which
	// is also what happens when the server says the game just changed.
	import { goto } from '$app/navigation';
	import { askSwap, askTime, giveGame, leaveGame, startGame, takeGame } from '#lib/boards.ts';
	import { dayLabel, timeRange, timeText } from '#lib/dates.ts';
	import { gameActions, keepPick, requestsOnGame, swapTargets, timeOptions, type GameAction } from '#lib/gamesheet.ts';
	import { fromParam, putStartHandoff } from '#lib/handoff.ts';
	import { matchViewPath } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { BoardView } from '#lib/types.ts';

	const t = strings.gameSheet;
	const tb = strings.boards;
	const LENGTHS = [60, 90] as const;

	interface Props {
		view: BoardView;
		gameId: string;
		token: string;
		/** Unix seconds, kept fresh by the page. */
		now: number;
		zone: string;
		/** An action is running: every button waits. */
		working: boolean;
		/** Queue a call; the page shows its error and reads the board again if it fails. */
		run: (job: () => Promise<BoardView | null>) => void;
		onclose: () => void;
	}

	let { view, gameId, token, now, zone, working, run, onclose }: Props = $props();

	const game = $derived(view.games.find((g) => g.id === gameId) ?? null);
	const me = $derived(view.me?.user_id ?? null);
	const actions = $derived(gameActions(view, gameId, me, now));
	const names = $derived(new Map(view.members.map((m) => [m.user_id, m.name])));
	const guestIds = $derived(new Set(view.members.filter((m) => m.guest).map((m) => m.user_id)));
	const when = (start: number, end: number) => `${dayLabel(start, zone)} · ${timeRange(start, end, zone)}`;

	let mode = $state<'swap' | 'time' | null>(null);
	// A different game starts with its forms closed.
	$effect(() => {
		void gameId;
		mode = null;
	});

	// ---- swap ---------------------------------------------------------------------------

	const targets = $derived(me ? swapTargets(view, gameId, me, now) : []);
	// `null`: the user has not picked, so the first target stands in. Once they have, a target
	// that goes away (a refresh) is shown as gone, never swapped for another player.
	let pickedKey = $state<string | null>(null);
	const key = (x: { gameId: string; userId: string }) => `${x.gameId}:${x.userId}`;
	const keptTarget = $derived(
		keepPick(
			targets.map(key),
			pickedKey
		)
	);
	const picked = $derived(targets.find((x) => key(x) === keptTarget.value) ?? null);
	const targetLabel = (x: { gameId: string; userId: string; name: string }) => {
		const g = view.games.find((o) => o.id === x.gameId);
		const name = guestIds.has(x.userId) ? `${x.name} (${tb.guestMark})` : x.name;
		return g ? t.swapPlayer(name, when(g.start, g.end)) : name;
	};

	function sendSwap(event: SubmitEvent) {
		event.preventDefault();
		if (!game || !picked) return;
		const { gameId: other, userId } = picked;
		mode = null;
		run(() => askSwap(token, game.id, game.version, other, userId));
	}

	// ---- change time --------------------------------------------------------------------

	let minutes = $state<(typeof LENGTHS)[number]>(60);
	let startPick = $state<number | null>(null);
	let agreed = $state(false);
	const options = $derived(timeOptions(view, gameId, now, minutes * 60));
	const keptTime = $derived(keepPick(options, startPick));
	const chosen = $derived(keptTime.value);
	const startLabel = (s: number) => `${dayLabel(s, zone)} ${timeText(s, zone)}`;

	function sendTime(event: SubmitEvent) {
		event.preventDefault();
		if (!game || chosen === null) return;
		const start = chosen;
		const end = chosen + minutes * 60;
		const direct = agreed;
		mode = null;
		agreed = false;
		run(() => askTime(token, game.id, game.version, start, end, direct));
	}

	function openForm(which: 'swap' | 'time') {
		mode = mode === which ? null : which;
		pickedKey = null;
		startPick = null;
	}

	// ---- the rest -----------------------------------------------------------------------

	function simple(call: typeof leaveGame) {
		if (!game) return;
		const { id, version } = game;
		mode = null;
		run(() => call(token, id, version));
	}

	function start() {
		if (!game) return;
		const id = game.id;
		run(async () => {
			const players = await startGame(token, id);
			putStartHandoff(players);
			await goto(`/new?from=${encodeURIComponent(fromParam(players.from))}`);
			return null;
		});
	}

	const handlers: Record<GameAction, () => void> = {
		leave: () => simple(leaveGame),
		give: () => simple(giveGame),
		take: () => simple(takeGame),
		swap: () => openForm('swap'),
		time: () => openForm('time'),
		start
	};
	const ORDER: GameAction[] = ['start', 'take', 'give', 'swap', 'time', 'leave'];
	const shownActions = $derived(ORDER.filter((a) => actions[a].shown));
	const asked = $derived(me ? requestsOnGame(view, gameId, me) : []);
</script>

{#if game}
	<section class="sheet" aria-label={tb.gameOpen}>
		<h2>{when(game.start, game.end)}</h2>
		<p class="hint">
			{tb.gameState[game.state]}
			{#if game.moved}<span class="badge moved" title={t.movedHint}>{t.moved}</span>{/if}
		</p>

		<h3>{t.playersLabel}</h3>
		<ol class="people" aria-label={t.playersLabel}>
			{#each game.players as p (p.user_id)}
				<li>
					{p.name}{#if p.guest}<span class="hint"> ({tb.guestMark})</span>{/if}{#if p.user_id === me}<span class="hint"> ({t.you})</span>{/if}
				</li>
			{/each}
		</ol>

		<h3>{t.line}</h3>
		{#if game.line.length === 0}
			<p class="hint">{t.lineEmpty}</p>
		{:else}
			<ol class="people" aria-label={t.line}>
				{#each game.line as id (id)}
					<li>{names.get(id) ?? ''}{#if guestIds.has(id)}<span class="hint"> ({tb.guestMark})</span>{/if}</li>
				{/each}
			</ol>
		{/if}

		{#each asked as r (r.id)}
			<p class="hint" role="status">{t.asked[r.state]}</p>
		{/each}

		{#if game.match_spectator_token}
			<a class="secondary" href={matchViewPath(game.match_spectator_token)}>{t.viewMatch}</a>
		{/if}

		{#if shownActions.length > 0}
			<ul class="acts" aria-label={tb.gameOpen}>
				{#each shownActions as a (a)}
					<li>
						<button
							type="button"
							class={a === 'start' ? 'primary' : a === 'leave' ? 'danger' : 'secondary'}
							disabled={working || !actions[a].enabled}
							aria-expanded={a === 'swap' || a === 'time' ? mode === a : undefined}
							aria-describedby={!actions[a].enabled && actions[a].reason ? `why-${a}` : undefined}
							onclick={handlers[a]}
						>
							{t[a]}
						</button>
						{#if !actions[a].enabled && actions[a].reason}
							<p class="hint why" id="why-{a}">{actions[a].reason}</p>
						{:else if a === 'leave'}
							<p class="hint why">{t.leaveNote}</p>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		{#if mode === 'swap' && (actions.swap.enabled || keptTarget.gone)}
			<form class="form" onsubmit={sendSwap}>
				<h3>{t.swapTitle}</h3>
				<label>
					{t.swapPick}
					<select value={picked ? key(picked) : ''} onchange={(e) => (pickedKey = e.currentTarget.value)}>
						{#if keptTarget.gone}<option value="" disabled>{t.swapGone}</option>{/if}
						{#each targets as x (key(x))}
							<option value={key(x)}>{targetLabel(x)}</option>
						{/each}
					</select>
					{#if keptTarget.gone}<span class="gone" role="alert">{t.swapGone}</span>{/if}
				</label>
				<div class="actions">
					<button type="submit" class="primary" disabled={working || !picked}>{t.swapSend}</button>
					<button type="button" class="secondary" onclick={() => (mode = null)}>{t.cancel}</button>
				</div>
			</form>
		{/if}

		{#if mode === 'time' && (actions.time.enabled || keptTime.gone)}
			<form class="form" onsubmit={sendTime}>
				<h3>{t.timeTitle}</h3>
				<label>
					{t.timeLength}
					<select value={minutes} onchange={(e) => {
						minutes = e.currentTarget.value === '90' ? 90 : 60;
						startPick = null;
					}}>
						{#each LENGTHS as len (len)}<option value={len}>{t.timeLengths[len]}</option>{/each}
					</select>
				</label>
				<label>
					{t.timeStart}
					<select value={chosen ?? ''} onchange={(e) => (startPick = Number(e.currentTarget.value))} disabled={options.length === 0}>
						{#if keptTime.gone}<option value="" disabled>{t.timeGone}</option>{/if}
						{#each options as s (s)}
							<option value={s}>{startLabel(s)}</option>
						{/each}
					</select>
					{#if keptTime.gone}<span class="gone" role="alert">{t.timeGone}</span>{/if}
				</label>
				<label class="check">
					<input type="checkbox" bind:checked={agreed} />
					<span>{t.timeAgreed}</span>
				</label>
				{#if agreed}<p class="hint">{t.timeAgreedHint}</p>{/if}
				<div class="actions">
					<button type="submit" class="primary" disabled={working || chosen === null}>
						{agreed ? t.timeApply : t.timeSend}
					</button>
					<button type="button" class="secondary" onclick={() => (mode = null)}>{t.cancel}</button>
				</div>
			</form>
		{/if}

		<button type="button" class="secondary close" onclick={onclose}>{tb.closeGame}</button>
	</section>
{/if}

<style>
	.sheet {
		margin-bottom: 12px;
		padding: 12px 14px;
		border: 3px solid var(--ball);
		border-radius: 14px;
		background: var(--panel);
	}
	h2 {
		margin: 0 0 4px;
	}
	h3 {
		margin: 12px 0 4px;
		font-size: 14px;
		color: var(--muted);
	}
	.badge.moved {
		margin-left: 0;
	}
	.people {
		margin: 0 0 6px;
		padding-left: 22px;
	}
	.acts {
		display: grid;
		gap: 8px;
		margin: 14px 0 0;
		padding: 0;
		list-style: none;
	}
	.acts button {
		width: 100%;
	}
	.why {
		margin: 4px 0 0;
	}
	.form {
		display: grid;
		gap: 10px;
		margin-top: 12px;
		padding-top: 4px;
		border-top: 1px solid var(--line);
	}
	select {
		min-height: var(--tap);
		width: 100%;
		padding: 0 10px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		font-size: 16px;
		color: var(--ink);
	}
	.gone {
		color: var(--warn);
		font-size: 13px;
	}
	.close {
		margin-top: 14px;
	}
</style>
