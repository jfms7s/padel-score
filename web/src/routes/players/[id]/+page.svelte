<script lang="ts">
	// One player: their record, any placings, then their matches, a page at a time.
	import { page } from '$app/state';
	import ResultRow from '#lib/components/ResultRow.svelte';
	import { getPlayerPage, playerMessage } from '#lib/players.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { Placing, Player, PlayerHistoryEntry } from '#lib/types.ts';

	const t = strings.history;
	const id = $derived(page.params.id ?? '');

	let player = $state.raw<Player | null>(null);
	let placings = $state.raw<Placing[]>([]);
	let items = $state.raw<PlayerHistoryEntry[]>([]);
	let next = $state<string | null>(null);
	let loadFailure = $state<string | null>(null);
	let moreFailure = $state<string | null>(null);
	let busy = $state(false);

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
			} catch (e) {
				if (live) loadFailure = playerMessage(e);
			}
		})();
		return () => {
			live = false;
		};
	});

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

<main class="acct">
	<a class="back" href="/players">{t.backToPlayers}</a>
	{#if loadFailure}
		<h1>{strings.players.title}</h1>
		<p class="error" role="alert">{loadFailure}</p>
	{:else if $session.status === 'signed_in' && player}
		<h1>{player.name}</h1>
		{#if player.archived}<p class="hint">{t.archived}</p>{/if}

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
						<span>{t.placing(p.place, p.place_to, p.field_size, p.event_kind)}</span>
						<time class="hint" datetime={new Date(p.finished_at * 1000).toISOString()}
							>{new Date(p.finished_at * 1000).toLocaleDateString()}</time
						>
					</li>
				{/each}
			</ul>
		{/if}

		<h2>{t.matches}</h2>
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
	.more {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
</style>
