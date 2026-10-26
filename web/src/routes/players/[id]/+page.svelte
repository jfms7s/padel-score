<script lang="ts">
	// One player: their record, any placings, then their matches, a page at a time.
	import { page } from '$app/state';
	import LinkCode from '#lib/components/LinkCode.svelte';
	import LinkedResultRow from '#lib/components/LinkedResultRow.svelte';
	import ResultRow from '#lib/components/ResultRow.svelte';
	import { ApiError } from '#lib/api.ts';
	import {
		codeDeadline,
		createLinkCode,
		linkLabel,
		linkMe,
		linkMessage,
		linkedHistory,
		unlink
	} from '#lib/linking.ts';
	import { getPlayerPage, playerMessage } from '#lib/players.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { CreatedLinkCode, LinkedEntry, Placing, Player, PlayerHistoryEntry } from '#lib/types.ts';

	const t = strings.history;
	const l = strings.linking;
	const id = $derived(page.params.id ?? '');

	let player = $state.raw<Player | null>(null);
	let placings = $state.raw<Placing[]>([]);
	let items = $state.raw<PlayerHistoryEntry[]>([]);
	let next = $state<string | null>(null);
	let loadFailure = $state<string | null>(null);
	let moreFailure = $state<string | null>(null);
	let busy = $state(false);

	// Linking.
	let origin = $state('');
	let code = $state<CreatedLinkCode | null>(null);
	// When the code runs out on this device's clock.
	let codeUntil = $state(0);
	let linkFailure = $state<string | null>(null);
	let linkDone = $state<string | null>(null);
	let confirmingUnlink = $state(false);

	// The combined history of a linked player.
	let wider = $state.raw<LinkedEntry[]>([]);
	let widerNext = $state<string | null>(null);
	let widerPhase = $state<'none' | 'loaded' | 'private' | 'failed'>('none');
	let widerBusy = $state(false);

	// Again for each player: a name in a match row links to another player's page.
	$effect(() => {
		const wanted = id;
		let live = true;
		player = null;
		placings = [];
		items = [];
		next = null;
		loadFailure = null;
		moreFailure = null;
		code = null;
		linkFailure = null;
		linkDone = null;
		confirmingUnlink = false;
		wider = [];
		widerNext = null;
		widerPhase = 'none';
		origin = location.origin;
		void (async () => {
			const s = await requireSignedIn();
			if (!live) return;
			if (s.status === 'unavailable') loadFailure = strings.networkError;
			if (s.status !== 'signed_in') return;
			try {
				const p = await getPlayerPage(wanted);
				if (!live) return;
				player = p.player;
				placings = p.placings;
				items = p.items;
				next = p.next;
				if (p.player.link) void loadWider(wanted);
			} catch (e) {
				if (live) loadFailure = playerMessage(e);
			}
		})();
		return () => {
			live = false;
		};
	});

	/** The player's combined history; a 403 means they keep it private. */
	async function loadWider(wanted: string, cursor?: string) {
		if (widerBusy) return;
		widerBusy = true;
		try {
			const h = await linkedHistory(wanted, cursor);
			if (wanted !== id) return;
			const seen = new Set(wider.map((m) => m.match_id));
			wider = [...wider, ...h.items.filter((m) => !seen.has(m.match_id))];
			widerNext = h.next;
			widerPhase = 'loaded';
		} catch (e) {
			if (wanted !== id) return;
			widerPhase = e instanceof ApiError && e.status === 403 ? 'private' : 'failed';
		} finally {
			widerBusy = false;
		}
	}

	/** A change to the link: run it, then read the player again to show the new state. */
	async function relink(run: () => Promise<void>, message: string) {
		if (busy || !player) return;
		const wanted = id;
		busy = true;
		linkFailure = null;
		linkDone = null;
		try {
			await run();
			const p = await getPlayerPage(wanted);
			if (wanted !== id) return;
			player = p.player;
			code = null;
			confirmingUnlink = false;
			wider = [];
			widerNext = null;
			widerPhase = 'none';
			linkDone = message;
			if (p.player.link) void loadWider(wanted);
		} catch (e) {
			if (wanted === id) linkFailure = linkMessage(e);
		} finally {
			busy = false;
		}
	}

	/** A new code, now and again each time the last one runs out (it voids the old one). */
	async function newCode() {
		const wanted = id;
		linkFailure = null;
		try {
			const made = await createLinkCode(wanted);
			if (wanted === id) {
				code = made;
				codeUntil = codeDeadline(made.expires_in, Date.now());
			}
		} catch (e) {
			if (wanted === id) {
				code = null;
				linkFailure = linkMessage(e);
			}
		}
	}

	async function more() {
		if (busy || !next) return;
		const wanted = id;
		busy = true;
		moreFailure = null;
		try {
			const p = await getPlayerPage(wanted, next);
			if (wanted !== id) return;
			const seen = new Set(items.map((m) => m.match_id));
			items = [...items, ...p.items.filter((m) => !seen.has(m.match_id))];
			next = p.next;
		} catch (e) {
			if (wanted === id) moreFailure = playerMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct page-wide-desk">
	<a class="back" href="/players">{t.backToPlayers}</a>
	{#if loadFailure}
		<h1>{strings.players.title}</h1>
		<p class="error" role="alert">{loadFailure}</p>
	{:else if $session.status === 'signed_in' && player}
		<h1>{player.name}</h1>
		{#if player.archived}<p class="hint">{t.archived}</p>{/if}

		{@const label = linkLabel(player)}
		<div class="cols">
		<div class="side">
		<section class="link" aria-label={l.linkToAccount}>
			{#if label}
				<p class="label"><strong>{label}</strong></p>
				{#if confirmingUnlink}
					<p>{l.unlinkConfirm(player.name)}</p>
					<div class="actions">
						<button class="danger" type="button" disabled={busy} onclick={() => relink(() => unlink(id), l.unlinked)}
							>{l.unlinkYes}</button
						>
						<button class="secondary" type="button" onclick={() => (confirmingUnlink = false)}>{l.keep}</button>
					</div>
				{:else}
					<button class="secondary" type="button" disabled={busy} onclick={() => (confirmingUnlink = true)}
						>{l.unlink}</button
					>
				{/if}
			{:else if !player.archived || code}
				<div class="actions">
					<button
						class="primary"
						type="button"
						disabled={busy}
						onclick={() => (code ? (code = null) : void newCode())}>{code ? l.hideCode : l.linkToAccount}</button
					>
					<button class="secondary" type="button" disabled={busy} onclick={() => relink(() => linkMe(id), l.linkedMe)}
						>{l.thisIsMe}</button
					>
				</div>
				{#if code}
					<LinkCode
						url={origin + code.url}
						title={l.linkCodeTitle(player.name)}
						hint={l.linkHint}
						deadline={codeUntil}
						onexpire={newCode}
					/>
				{/if}
			{/if}
			{#if linkFailure}<p class="error" role="alert">{linkFailure}</p>{/if}
			{#if linkDone}<p class="ok" role="status">{linkDone}</p>{/if}
		</section>

		<dl class="record" aria-label={t.record}>
			{#each t.recordParts as part (part.key)}
				<div>
					<dt><span aria-hidden="true">{part.short}</span><span class="visually-hidden">{part.long}</span></dt>
					<dd>{player.record[part.key]}</dd>
				</div>
			{/each}
		</dl>

		{#if placings.length > 0}
			<h2>{t.placings}</h2>
			<ul class="list" aria-label={t.placings}>
				{#each placings as p (p.event_id)}
					<li class="row">
						<span>{t.placingIn(p.event_name, t.placing(p.place, p.place_to, p.field_size, p.event_kind))}</span>
						<time class="hint" datetime={new Date(p.finished_at * 1000).toISOString()}
							>{new Date(p.finished_at * 1000).toLocaleDateString()}</time
						>
					</li>
				{/each}
			</ul>
		{/if}

		</div>
		<div class="main">
		<h2 class="first">{t.matches}</h2>
		{#if items.length === 0}
			<p class="hint">{t.noMatches}</p>
		{:else}
			<ul class="list" aria-label={t.matches}>
				{#each items as entry (entry.match_id)}
					<ResultRow {entry} outcome={entry.outcome} />
				{/each}
			</ul>
			{#if moreFailure}<p class="error more-error" role="alert">{moreFailure}</p>{/if}
			{#if next}
				<button class="secondary more" type="button" disabled={busy} onclick={more}>{t.loadMore}</button>
			{/if}
		{/if}

		{#if player.link}
			<h2>{l.allMatches}</h2>
			{#if widerPhase === 'private'}
				<p class="hint">{l.private}</p>
			{:else if widerPhase === 'failed'}
				<p class="error" role="alert">{l.loadFailed}</p>
			{:else if widerPhase === 'loaded'}
				<p class="hint">{l.allMatchesHint}</p>
				{#if wider.length === 0}
					<p class="hint">{l.noAllMatches}</p>
				{:else}
					<ul class="list" aria-label={l.allMatches}>
						{#each wider as entry (entry.match_id)}
							<LinkedResultRow {entry} />
						{/each}
					</ul>
					{#if widerNext}
						<button class="secondary more" type="button" disabled={widerBusy} onclick={() => loadWider(id, widerNext!)}
							>{t.loadMore}</button
						>
					{/if}
				{/if}
			{:else}
				<p class="hint">{strings.auth.loading}</p>
			{/if}
		{/if}
		</div>
		</div>
	{:else}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	h1 {
		overflow-wrap: anywhere;
	}
	.link {
		margin-bottom: 12px;
	}
	.link .label {
		margin: 0 0 8px;
	}
	.record {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 8px;
		margin: 0 0 8px;
	}
	.record div {
		display: flex;
		flex-direction: column-reverse;
		align-items: center;
		padding: 10px 8px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.record dt {
		color: var(--muted);
		font-size: 13px;
		font-weight: 650;
	}
	.record dd {
		margin: 0;
		font-size: 40px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}
	@media (min-width: 1100px) {
		.cols {
			display: grid;
			grid-template-columns: 22rem minmax(0, 1fr);
			gap: 32px;
			align-items: start;
		}
		.main .first {
			margin-top: 0;
		}
	}
	.more {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
</style>
