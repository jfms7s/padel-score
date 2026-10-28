<script lang="ts">
	// A game board: the week grid for painting free time, and the games that formed. Anyone with
	// the link can look; a signed-in visitor joins and adds times. There is no stream: the board
	// is read again after the user's own actions, when the page comes back into view and every
	// 30 seconds while it is in view.
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError, redirectToSignIn } from '#lib/api.ts';
	import {
		addRange,
		boardMessage,
		deleteBoard,
		getBoard,
		joinBoard,
		leaveBoard,
		removeRange,
		updateBoard
	} from '#lib/boards.ts';
	import WeekGrid from '#lib/components/boards/WeekGrid.svelte';
	import { browserTimeZone, dayLabel, monthYear, timeRange } from '#lib/dates.ts';
	import { shareLink } from '#lib/share.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { BoardView, GameOut } from '#lib/types.ts';

	const t = strings.boards;
	const REFRESH_MS = 30_000;
	const MAX_PLACE_LENGTH = 80;

	const token = $derived(page.params.token ?? '');

	let view = $state<BoardView | null>(null);
	let loadFailure = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let working = $state(0);
	let shareNote = $state<string | null>(null);
	let confirming = $state<'leave' | 'delete' | null>(null);
	let editing = $state(false);
	let editTitle = $state('');
	let editPlace = $state('');
	let openGameId = $state<string | null>(null);
	let askedForGame = false;

	const me = $derived(view?.me ?? null);
	const signedIn = $derived(me !== null);
	const member = $derived(me?.member === true);
	const creator = $derived(me?.creator === true);
	const over = $derived(view?.board.over === true);
	// Every time on the page is the viewer's own, like the notices.
	const zone = browserTimeZone();
	const myGames = $derived(
		view && me ? view.games.filter((g) => g.players.some((p) => p.user_id === me.user_id)) : []
	);
	const openGame = $derived(view?.games.find((g) => g.id === openGameId) ?? null);
	const weekText = $derived(
		view
			? t.weekHeader(
					dayLabel(view.board.starts_at, zone),
					dayLabel(view.board.ends_at - 1, zone),
					monthYear(view.board.ends_at - 1, zone)
				)
			: ''
	);

	// ---- reading ------------------------------------------------------------------------

	// A response only counts if nothing newer was asked for since.
	let latest = 0;

	async function load(forToken: string, quiet = false) {
		const mine = ++latest;
		try {
			const next = await getBoard(forToken);
			if (mine !== latest) return;
			view = next;
			loadFailure = null;
			if (!askedForGame) {
				askedForGame = true;
				const wanted = page.url.searchParams.get('game');
				if (wanted && next.games.some((g) => g.id === wanted)) openGameId = wanted;
			}
		} catch (e) {
			if (mine !== latest || quiet) return;
			loadFailure = e instanceof ApiError && e.code === 'not_found' ? t.notFound : boardMessage(e);
		}
	}

	$effect(() => {
		const current = token;
		untrack(() => {
			view = null;
			loadFailure = null;
			askedForGame = false;
			openGameId = null;
			void load(current);
		});
		const again = () => {
			if (document.visibilityState === 'visible' && working === 0) void load(current, true);
		};
		const timer = setInterval(again, REFRESH_MS);
		document.addEventListener('visibilitychange', again);
		window.addEventListener('focus', again);
		return () => {
			clearInterval(timer);
			document.removeEventListener('visibilitychange', again);
			window.removeEventListener('focus', again);
		};
	});

	// ---- the user's own actions ---------------------------------------------------------

	// One at a time, in the order they were made: a trim is two removals.
	let chain: Promise<void> = Promise.resolve();

	function act(job: () => Promise<BoardView | null>) {
		actionError = null;
		working += 1;
		chain = chain
			.then(async () => {
				const next = await job();
				if (next) {
					latest += 1;
					view = next;
				}
			})
			.catch((e) => {
				actionError = boardMessage(e);
				// Show what the server has, as the failed action may have been refused for a reason.
				void load(token, true);
			})
			.finally(() => {
				working -= 1;
			});
	}

	const add = (start: number, end: number) => act(() => addRange(token, start, end));
	const remove = (start: number, end: number) => act(() => removeRange(token, start, end));
	const join = () => act(() => joinBoard(token));

	async function leave() {
		confirming = null;
		act(async () => {
			await leaveBoard(token);
			await goto('/');
			return null;
		});
	}

	async function destroy() {
		confirming = null;
		act(async () => {
			await deleteBoard(token);
			await goto('/');
			return null;
		});
	}

	let menuOpen = $state(false);
	let confirmEl: HTMLElement | undefined = $state();
	let menuButton: HTMLElement | undefined = $state();

	// The confirmation takes focus when it opens and hands it back to the menu button on cancel.
	$effect(() => {
		if (confirming && confirmEl) confirmEl.focus();
	});

	function closeConfirm() {
		confirming = null;
		queueMicrotask(() => menuButton?.focus());
	}

	function ask(what: 'leave' | 'delete') {
		menuOpen = false;
		confirming = what;
	}

	function closeMenu(e: Event) {
		if (!menuOpen) return;
		if (e instanceof KeyboardEvent ? e.key !== 'Escape' : e.target instanceof Element && e.target.closest('.menu')) return;
		menuOpen = false;
	}

	function startEdit() {
		if (!view) return;
		menuOpen = false;
		editTitle = view.board.title;
		editPlace = view.board.place ?? '';
		editing = true;
		confirming = null;
	}

	function saveEdit(event: SubmitEvent) {
		event.preventDefault();
		const title = editTitle.trim();
		if (title === '') return;
		editing = false;
		act(() => updateBoard(token, { title, place: editPlace.trim() === '' ? null : editPlace.trim() }));
	}

	async function share() {
		if (!view) return;
		const url = `${location.origin}/b/${encodeURIComponent(token)}`;
		const result = await shareLink(url, t.shareTitle(view.board.title));
		shareNote = result === 'copied' ? strings.share.linkCopied : result === 'failed' ? strings.share.linkFailed : null;
	}

	const gameWhen = (g: GameOut) => `${dayLabel(g.start, zone)} · ${timeRange(g.start, g.end, zone)}`;
</script>

<svelte:window onclick={closeMenu} onkeydown={closeMenu} />

<main class="acct page-wide-desk">
	<a class="back" href="/">{strings.appName}</a>

	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if view === null}
		<p class="hint">{t.loading}</p>
	{:else}
		<header class="head">
			<div class="title">
				<h1>{view.board.title}</h1>
				{#if view.board.place}<p class="place">{view.board.place}</p>{/if}
				<p class="hint">
					{weekText} · {t.members(view.members.length)} · {t.by(view.board.creator_name)}
				</p>
			</div>
			<div class="tools">
				<button type="button" class="secondary" onclick={share}>{t.share}</button>
				{#if (member || creator) && !over}
					<details class="menu" bind:open={menuOpen}>
						<summary class="secondary" aria-label={t.menu} bind:this={menuButton}>
							<span aria-hidden="true">⋯</span>
						</summary>
						<div class="items">
							{#if creator && !over}
								<button type="button" onclick={startEdit}>{t.edit}</button>
							{/if}
							{#if member && !over}
								<button type="button" onclick={() => ask('leave')}>{t.leave}</button>
							{/if}
							{#if creator && !over}
								<button type="button" class="warn" onclick={() => ask('delete')}>{t.delete}</button>
							{/if}
						</div>
					</details>
				{/if}
			</div>
		</header>
		<div aria-live="polite">{#if shareNote}<p class="ok">{shareNote}</p>{/if}</div>

		{#if confirming}
			<div class="confirm" role="alertdialog" tabindex="-1" bind:this={confirmEl} aria-label={confirming === 'delete' ? t.delete : t.leave}>
				<p>{confirming === 'delete' ? t.deleteConfirm : t.leaveConfirm}</p>
				<div class="actions">
					<button type="button" class="danger" onclick={confirming === 'delete' ? destroy : leave}>
						{confirming === 'delete' ? t.delete : t.leave}
					</button>
					<button type="button" class="secondary" onclick={closeConfirm}>{t.cancel}</button>
				</div>
			</div>
		{/if}

		{#if editing}
			<form class="edit" onsubmit={saveEdit}>
				<label>
					{t.title}
					<input bind:value={editTitle} autocomplete="off" maxlength={MAX_NAME_LENGTH} required />
				</label>
				<label>
					{t.place}
					<input bind:value={editPlace} autocomplete="off" maxlength={MAX_PLACE_LENGTH} />
				</label>
				<div class="actions">
					<button type="submit" class="primary">{t.save}</button>
					<button type="button" class="secondary" onclick={() => (editing = false)}>{t.cancel}</button>
				</div>
			</form>
		{/if}

		{#if over}<p class="hint">{t.over}</p>{/if}
		{#if actionError}<p class="error" role="alert">{actionError}</p>{/if}

		{#if !signedIn && !over}
			<div class="join">
				<button type="button" class="primary" onclick={redirectToSignIn}>{t.signInToAdd}</button>
			</div>
		{:else if signedIn && !member && !over}
			<div class="join">
				<button type="button" class="primary" disabled={working > 0} onclick={join}>
					{working > 0 ? t.joining : t.join}
				</button>
				<p class="hint">{t.joinNote}</p>
			</div>
		{/if}

		<div class="body">
			<div class="gridcol">
				<WeekGrid {view} me={me?.user_id ?? null} readonly={!signedIn || over} onadd={add} onremove={remove} ongame={(id) => (openGameId = id)} />
			</div>

			<aside class="side" aria-label={t.yourGames}>
				{#if openGame}
					<section class="game-card" aria-label={t.gameOpen}>
						<h2>{gameWhen(openGame)}</h2>
						<p class="hint">{t.gameState[openGame.state]} · {t.line(openGame.line.length)}</p>
						<ul class="people" aria-label={t.players}>
							{#each openGame.players as p (p.user_id)}<li>{p.name}</li>{/each}
						</ul>
						<button type="button" class="secondary" onclick={() => (openGameId = null)}>{t.closeGame}</button>
					</section>
				{/if}
				{#if signedIn}
					<h2>{t.yourGames}</h2>
					{#if myGames.length === 0}
						<p class="hint">{t.noGames}. {t.noGamesHint}</p>
					{:else}
						<ul class="list">
							{#each myGames as g (g.id)}
								<li>
									<button type="button" class="row game-row" onclick={() => (openGameId = g.id)}>
										<span class="what">
											<strong>{gameWhen(g)}</strong>
											<span class="hint">{g.players.map((p) => p.name).join(', ')}</span>
										</span>
										<span class="badge" class:off={g.state === 'short'}>{t.gameState[g.state]}</span>
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				{/if}
			</aside>
		</div>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		justify-content: space-between;
		gap: 8px 16px;
	}
	.title {
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.place {
		margin: 0;
		font-weight: 650;
	}
	.tools {
		display: flex;
		gap: 8px;
		align-items: center;
	}
	.tools :global(.secondary) {
		cursor: pointer;
	}
	.menu {
		position: relative;
	}
	summary {
		list-style: none;
		width: var(--tap);
		padding: 0;
	}
	summary::-webkit-details-marker {
		display: none;
	}
	.items {
		position: absolute;
		right: 0;
		z-index: 10;
		margin-top: 6px;
		min-width: 12rem;
		display: flex;
		flex-direction: column;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		box-shadow: 0 6px 20px rgb(0 0 0 / 0.2);
		overflow: hidden;
	}
	.items button {
		min-height: var(--tap);
		padding: 0 16px;
		border: 0;
		border-top: 1px solid var(--line);
		background: transparent;
		text-align: left;
		font-weight: 600;
	}
	.items button:first-child {
		border-top: 0;
	}
	.items .warn {
		color: var(--warn);
	}
	.confirm,
	.edit,
	.join {
		margin: 12px 0;
	}
	.confirm {
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
	}
	.join {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
	}
	.join .hint {
		margin: 0;
	}
	.body {
		display: grid;
		gap: 16px;
		margin-top: 12px;
	}
	.gridcol {
		min-width: 0;
	}
	.side h2 {
		margin-top: 8px;
	}
	.game-card {
		margin-bottom: 12px;
		padding: 12px 14px;
		border: 3px solid var(--ball);
		border-radius: 14px;
		background: var(--panel);
	}
	.game-card h2 {
		margin: 0 0 4px;
	}
	.people {
		margin: 0 0 10px;
		padding-left: 22px;
	}
	.game-row {
		width: 100%;
		text-align: left;
		flex-wrap: nowrap;
		cursor: pointer;
	}
	@media (min-width: 1100px) {
		.body {
			grid-template-columns: minmax(0, 1fr) 22rem;
			align-items: start;
		}
	}
</style>
