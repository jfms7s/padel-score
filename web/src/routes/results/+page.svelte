<script lang="ts">
	// Results: my record and placings, then the matches I played or the ones I recorded.
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import RecordPlacings from '#lib/components/RecordPlacings.svelte';
	import ResultsList from '#lib/components/ResultsList.svelte';
	import { getPrivacy, linkMessage, myResults } from '#lib/linking.ts';
	import { getHistory, playerMessage } from '#lib/players.ts';
	import { playedRow, recordedRow, showFindYourself, viewFromQuery, type ResultsView } from '#lib/results.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { HistoryEntry, LinkedEntry, LinkedHistory, Privacy } from '#lib/types.ts';

	const t = strings.results;
	const VIEWS: ResultsView[] = ['played', 'recorded'];

	const view = $derived(viewFromQuery(new URLSearchParams(page.url.search)));

	let mine = $state.raw<LinkedHistory | null>(null);
	let played = $state.raw<LinkedEntry[]>([]);
	let playedNext = $state<string | null>(null);
	let recorded = $state.raw<HistoryEntry[]>([]);
	let recordedNext = $state<string | null>(null);
	let recordedLoaded = $state(false);
	let privacy = $state.raw<Privacy | null>(null);

	let ready = $state(false);
	// The recorded list's first page failed; the Retry button fetches it again.
	let recordedLoading = $state(false);
	let recordedError = $state<string | null>(null);
	let loadFailure = $state<string | null>(null);
	let busy = $state(false);
	let moreFailure = $state<string | null>(null);

	const rows = $derived(view === 'played' ? played.map((e) => playedRow(e)) : recorded.map((e) => recordedRow(e)));
	const hasMore = $derived(view === 'played' ? playedNext !== null : recordedNext !== null);
	const findYourself = $derived(recordedLoaded && showFindYourself(played.length, recorded.length));

	async function loadRecorded() {
		recordedLoading = true;
		try {
			const first = await getHistory();
			recorded = first.items;
			recordedNext = first.next;
			recordedLoaded = true;
		} finally {
			recordedLoading = false;
		}
	}

	async function retryRecorded() {
		if (recordedLoading) return;
		recordedError = null;
		try {
			await loadRecorded();
		} catch (e) {
			recordedError = playerMessage(e);
		}
	}

	$effect(() => {
		let live = true;
		void (async () => {
			const s = await requireSignedIn();
			if (!live) return;
			if (s.status === 'unavailable') loadFailure = strings.networkError;
			if (s.status !== 'signed_in') return;
			try {
				const [r, p] = await Promise.allSettled([myResults(), getPrivacy()]);
				if (!live) return;
				if (p.status === 'fulfilled') privacy = p.value;
				if (r.status === 'rejected') throw r.reason;
				mine = r.value;
				played = r.value.items;
				playedNext = r.value.next;
			} catch (e) {
				if (live) loadFailure = linkMessage(e);
				return;
			}
			// The recorded list failing leaves the record and the played list usable.
			if (viewFromQuery(new URLSearchParams(page.url.search)) === 'recorded' || played.length === 0) {
				try {
					await loadRecorded();
				} catch (e) {
					if (live) recordedError = playerMessage(e);
				}
			}
			if (live) ready = true;
		})();
		return () => {
			live = false;
		};
	});

	// Switching to Recorded for the first time loads its first page.
	$effect(() => {
		if (!ready || view !== 'recorded' || recordedLoaded) return;
		if (recordedError || recordedLoading) return;
		void retryRecorded();
	});

	function choose(v: ResultsView) {
		if (v === view) return;
		const query = new URLSearchParams(page.url.search);
		query.set('view', v);
		moreFailure = null;
		void goto(`?${query}`, { replace: true, reset: false });
	}

	async function more() {
		if (busy) return;
		busy = true;
		moreFailure = null;
		try {
			if (view === 'played' && playedNext) {
				const h = await myResults(playedNext);
				const seen = new Set(played.map((m) => m.match_id));
				played = [...played, ...h.items.filter((m) => !seen.has(m.match_id))];
				playedNext = h.next;
			} else if (view === 'recorded' && recordedNext) {
				const h = await getHistory(recordedNext);
				const seen = new Set(recorded.map((m) => m.match_id));
				recorded = [...recorded, ...h.items.filter((m) => !seen.has(m.match_id))];
				recordedNext = h.next;
			}
		} catch (e) {
			moreFailure = view === 'played' ? linkMessage(e) : playerMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct page-wide">
	<h1>{t.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && ready && mine}
		<div class="cols">
			<div class="rec"><RecordPlacings record={mine.record} placings={mine.placings} /></div>
			<div class="switch" role="group" aria-label={t.show}>
				{#each VIEWS as v (v)}
					<button type="button" aria-pressed={view === v} onclick={() => choose(v)}>{t[v]}</button>
				{/each}
			</div>
			<div class="list">
				{#if view === 'recorded' && !recordedLoaded && recordedLoading}
					<p class="hint">{strings.auth.loading}</p>
				{:else if view === 'recorded' && recordedError && !recordedLoaded}
					<p class="error" role="alert">{recordedError}</p>
					<button class="secondary more" type="button" disabled={recordedLoading} onclick={retryRecorded}>{t.retry}</button>
				{:else if view === 'recorded' && !recordedLoaded}
					<p class="hint">{strings.auth.loading}</p>
				{:else if rows.length === 0}
					<p class="hint">{t.empty}</p>
				{:else}
					<ResultsList {rows} />
				{/if}
				{#if moreFailure}<p class="error more-error" role="alert">{moreFailure}</p>{/if}
				{#if hasMore}
					<button class="secondary more" type="button" disabled={busy} onclick={more}>{strings.history.loadMore}</button>
				{/if}
			</div>
			{#if findYourself}
				<p class="hint find">
					{t.findYourself}
					<a href="/players">{t.findYourselfLink}</a>
				</p>
			{/if}
			{#if privacy}
				<p class="privacy">
					<span class="hint">{t.privacyRow}</span>
					<span>{t.privacy[privacy.visibility]}</span>
					<a href="/account#privacy">{t.change}</a>
				</p>
			{/if}
		</div>
	{:else if !loadFailure}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	.rec,
	.list {
		min-width: 0;
	}
	.privacy {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 12px;
		margin-top: 16px;
	}
	.privacy a,
	.find a {
		display: flex;
		align-items: center;
		width: fit-content;
		min-height: var(--tap);
		color: var(--ink);
	}
	.find {
		margin-top: 12px;
	}
	.switch {
		display: grid;
		grid-template-columns: 1fr 1fr;
		margin-bottom: 12px;
		border: 1px solid var(--line);
		border-radius: 14px;
		overflow: hidden;
	}
	.switch button {
		min-height: var(--tap);
		border: 0;
		background: var(--panel);
		font-size: 16px;
		font-weight: 650;
	}
	.switch button[aria-pressed='true'] {
		background: var(--ink);
		color: var(--glass);
	}
	.switch button:focus-visible {
		outline: 2px solid var(--ink);
		outline-offset: -4px;
	}
	.switch button[aria-pressed='true']:focus-visible {
		outline-color: var(--glass);
	}
	.more {
		width: 100%;
		margin-top: 12px;
	}
	.more-error {
		margin-top: 12px;
	}
	@media (min-width: 700px) {
		.cols {
			display: grid;
			grid-template-columns: 300px minmax(0, 1fr);
			grid-template-rows: auto 1fr auto auto;
			grid-template-areas:
				'rec switch'
				'rec list'
				'privacy list'
				'. find';
			column-gap: 32px;
			align-items: start;
		}
		.rec {
			grid-area: rec;
		}
		.switch {
			grid-area: switch;
		}
		.list {
			grid-area: list;
		}
		.find {
			grid-area: find;
		}
		.privacy {
			grid-area: privacy;
		}
		.switch {
			display: inline-grid;
			grid-template-columns: repeat(2, 8rem);
		}
		.switch button {
			min-height: 40px;
		}
		.more {
			width: auto;
			min-width: 12rem;
		}
	}
</style>
