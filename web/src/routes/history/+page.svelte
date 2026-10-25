<script lang="ts">
	// Past matches: finished matches the signed-in user started, newest first, a page at a time.
	import { onMount } from 'svelte';
	import ResultRow from '#lib/components/ResultRow.svelte';
	import { getHistory, playerMessage } from '#lib/players.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { HistoryEntry } from '#lib/types.ts';

	const t = strings.history;

	let items = $state.raw<HistoryEntry[]>([]);
	let next = $state<string | null>(null);
	let loaded = $state(false);
	let loadFailure = $state<string | null>(null);
	let moreFailure = $state<string | null>(null);
	let busy = $state(false);

	onMount(() => {
		void load();
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		try {
			const page = await getHistory();
			items = page.items;
			next = page.next;
			loaded = true;
		} catch (e) {
			loadFailure = playerMessage(e);
		}
	}

	async function more() {
		if (busy || !next) return;
		busy = true;
		moreFailure = null;
		try {
			const page = await getHistory(next);
			const seen = new Set(items.map((m) => m.match_id));
			items = [...items, ...page.items.filter((m) => !seen.has(m.match_id))];
			next = page.next;
		} catch (e) {
			moreFailure = playerMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<a class="back" href="/">{t.back}</a>
	<h1>{t.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && loaded}
		{#if items.length === 0}
			<p class="hint">{t.empty}</p>
		{:else}
			<ul class="list" aria-label={t.title}>
				{#each items as entry (entry.match_id)}
					<ResultRow {entry} />
				{/each}
			</ul>
			{#if moreFailure}<p class="error more-error" role="alert">{moreFailure}</p>{/if}
			{#if next}
				<button class="secondary more" type="button" disabled={busy} onclick={more}>{t.loadMore}</button>
			{/if}
		{/if}
	{:else if !loadFailure}
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
	.more {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
</style>
