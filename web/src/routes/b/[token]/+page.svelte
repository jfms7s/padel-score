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
		answerBoardRequest,
		answerRequest,
		boardMessage,
		deleteBoard,
		getBoard,
		joinBoard,
		leaveBoard,
		removeRange,
		updateBoard
	} from '#lib/boards.ts';
	import GameSheet from '#lib/components/boards/GameSheet.svelte';
	import GuestJoin from '#lib/components/boards/GuestJoin.svelte';
	import RequestCard from '#lib/components/boards/RequestCard.svelte';
	import WeekGrid from '#lib/components/boards/WeekGrid.svelte';
	import { browserTimeZone, dayLabel, monthYear, timeRange } from '#lib/dates.ts';
	import { requestCards } from '#lib/gamesheet.ts';
	import { claimGuest, forgetGuest, guestKey, removeGuest, rememberedGuestId } from '#lib/guests.ts';
	import { guestsToPick, rememberedGuest } from '#lib/guestjoin.ts';
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
	type Confirming = 'leave' | 'delete' | { kind: 'removeGuest' | 'claimGuest'; id: string };
	let confirming = $state<Confirming | null>(null);
	// Whether the "Were you …?" banner was dismissed; the kept key is forgotten with it.
	let dismissedWereYou = $state(false);
	let editing = $state(false);
	let editTitle = $state('');
	let editPlace = $state('');
	let editAllowGuests = $state(false);
	let openGameId = $state<string | null>(null);
	let askedForGame = false;

	const me = $derived(view?.me ?? null);
	const signedIn = $derived(me !== null);
	const guest = $derived(me?.guest === true);
	const nameWithMark = (p: { name: string; guest: boolean }) => (p.guest ? `${p.name} (${t.guestMark})` : p.name);
	const userSignedIn = $derived(me !== null && !guest);
	const over = $derived(view?.board.over === true);
	const guestRows = $derived(view ? guestsToPick(view) : []);
	const wasGuest = $derived(
		userSignedIn && view && !over && !dismissedWereYou ? rememberedGuest(view, rememberedGuestId(token)) : null
	);
	const member = $derived(me?.member === true);
	const creator = $derived(me?.creator === true);
	// Every time on the page is the viewer's own, like the notices.
	const zone = browserTimeZone();
	const myGames = $derived(
		view && me ? view.games.filter((g) => g.players.some((p) => p.user_id === me.user_id)) : []
	);
	const openGame = $derived(view?.games.find((g) => g.id === openGameId) ?? null);
	const waiting = $derived(view ? requestCards(view) : []);
	// Unix seconds, read again with the board, so the sheet's time rules follow the clock.
	let now = $state(Math.floor(Date.now() / 1000));
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
			now = Math.floor(Date.now() / 1000);
			loadFailure = null;
			// A kept guest key the board no longer honours: the guest was removed.
			// A guest the open confirm step names may be gone by now.
			if (confirming && typeof confirming !== 'string') {
				const askedId = confirming.id;
				if (!guestsToPick(next).some((g) => g.user_id === askedId)) confirming = null;
			}
			const heldId = rememberedGuestId(forToken);
			if (guestKey(forToken) !== null && (next.me === null || (heldId !== null && !rememberedGuest(next, heldId))))
				forgetGuest(forToken);
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
			dismissedWereYou = false;
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
					now = Math.floor(Date.now() / 1000);
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

	const answer = (id: string, accept: boolean) =>
		act(() => (guest ? answerBoardRequest(token, id, accept) : answerRequest(id, accept)));
	const requestFromGuest = (id: string) => view?.requests.find((r) => r.id === id)?.from.guest === true;
	const joinedAsGuest = (next: BoardView) => {
		latest += 1;
		view = next;
		now = Math.floor(Date.now() / 1000);
	};
	const add = (start: number, end: number) => act(() => addRange(token, start, end));
	const remove = (start: number, end: number) => act(() => removeRange(token, start, end));
	const join = () => act(() => joinBoard(token));

	async function leave() {
		confirming = null;
		act(async () => {
			const asGuest = guest;
			await leaveBoard(token);
			// A guest has no home to go to: stay on the board, signed out, with the join form.
			if (asGuest) return getBoard(token);
			await goto('/');
			return null;
		});
	}

	function runGuestStep(step: { kind: 'removeGuest' | 'claimGuest'; id: string }) {
		confirming = null;
		act(() => (step.kind === 'removeGuest' ? removeGuest(token, step.id) : claimGuest(token, step.id)));
	}

	function dismissWereYou() {
		forgetGuest(token);
		dismissedWereYou = true;
	}

	async function destroy() {
		confirming = null;
		act(async () => {
			await deleteBoard(token);
			await goto('/');
			return null;
		});
	}

	// Opening a game brings its sheet into view and hands it the focus, as on a phone it sits
	// below the grid.
	let sheetEl: HTMLElement | undefined = $state();
	$effect(() => {
		// Only a newly opened game does this, not the board being read again.
		if (openGameId && sheetEl) {
			const el = sheetEl;
			untrack(() => {
				el.scrollIntoView({ block: 'nearest' });
				el.focus({ preventScroll: true });
			});
		}
	});

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

	function ask(what: Confirming) {
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
		editAllowGuests = view.board.allow_guests;
		editing = true;
		confirming = null;
	}

	function saveEdit(event: SubmitEvent) {
		event.preventDefault();
		const title = editTitle.trim();
		if (title === '') return;
		editing = false;
		act(() => updateBoard(token, {
				title,
				place: editPlace.trim() === '' ? null : editPlace.trim(),
				allow_guests: editAllowGuests
			}));
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
			{@const step = typeof confirming === 'string' ? null : confirming}
			{@const guestName = step ? (guestRows.find((g) => g.user_id === step.id)?.name ?? '') : ''}
			{@const label = step ? t[step.kind] : confirming === 'delete' ? t.delete : t.leave}
			<div class="confirm" role="alertdialog" tabindex="-1" bind:this={confirmEl} aria-label={label}>
				<p>
					{step
						? step.kind === 'removeGuest'
							? t.removeGuestConfirm(guestName)
							: t.claimGuestConfirm(guestName)
						: confirming === 'delete'
							? t.deleteConfirm
							: t.leaveConfirm}
				</p>
				<div class="actions">
					<button
						type="button"
						class={step?.kind === 'claimGuest' ? 'primary' : 'danger'}
						onclick={step ? () => runGuestStep(step) : confirming === 'delete' ? destroy : leave}
					>
						{label}
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
				<label class="check">
					<input type="checkbox" bind:checked={editAllowGuests} aria-describedby="edit-allow-guests-note" />
					<span>{t.allowGuests}</span>
				</label>
				<p class="hint" id="edit-allow-guests-note">{t.allowGuestsNote}</p>
				<div class="actions">
					<button type="submit" class="primary">{t.save}</button>
					<button type="button" class="secondary" onclick={() => (editing = false)}>{t.cancel}</button>
				</div>
			</form>
		{/if}

		{#if over}<p class="hint">{t.over}</p>{/if}
		{#if actionError && !openGame}<p class="error" role="alert">{actionError}</p>{/if}

		{#if wasGuest}
			<div class="confirm" role="group" aria-label={t.wereYou(wasGuest.name)}>
				<p>{t.wereYou(wasGuest.name)}</p>
				<div class="actions">
					<button type="button" class="primary" disabled={working > 0} onclick={() => ask({ kind: 'claimGuest', id: wasGuest.user_id })}>
						{t.claimGuest}
					</button>
					<button type="button" class="secondary" onclick={dismissWereYou}>{t.dismiss}</button>
				</div>
			</div>
		{/if}

		{#if !signedIn && !over}
			<div class="join">
				<button type="button" class="primary" onclick={redirectToSignIn}>{t.signInToAdd}</button>
				{#if view.board.allow_guests}
					<GuestJoin {view} {token} onjoined={joinedAsGuest} />
				{/if}
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
				{#if waiting.length > 0}
					<h2>{strings.gameSheet.needsAnswer}</h2>
					<ul class="list requests">
						{#each waiting as r (r.id)}
							<li>
								<RequestCard
									kind={r.kind}
									from={r.from}
									guest={requestFromGuest(r.id)}
									current={r.current}
									proposed={r.proposed}
									busy={working > 0}
									onanswer={(accept) => answer(r.id, accept)}
								/>
							</li>
						{/each}
					</ul>
				{/if}
				{#if openGame}
					{#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
					<div class="sheetbox" tabindex="-1" bind:this={sheetEl}>
						<GameSheet
							{view}
							gameId={openGame.id}
							{token}
							{now}
							{zone}
							working={working > 0}
							run={act}
							onclose={() => (openGameId = null)}
						/>
					</div>
				{/if}
				{#if guestRows.length > 0 && signedIn}
					<h2>{t.guests}</h2>
					<ul class="list">
						{#each guestRows as g (g.user_id)}
							<li class="row">
								<span class="what"><strong>{g.name}</strong></span>
								<span class="actions">
									{#if userSignedIn && !over}
										<button type="button" class="secondary" disabled={working > 0} onclick={() => ask({ kind: 'claimGuest', id: g.user_id })}>
											{t.claimGuest}
										</button>
									{/if}
									{#if creator && !over}
										<button type="button" class="secondary" disabled={working > 0} onclick={() => ask({ kind: 'removeGuest', id: g.user_id })}>
											{t.removeGuest}
										</button>
									{/if}
								</span>
							</li>
						{/each}
					</ul>
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
											<span class="hint">{g.players.map(nameWithMark).join(', ')}</span>
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
	.requests {
		margin-bottom: 12px;
	}
	.sheetbox:focus-visible {
		outline-offset: 3px;
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
