<script lang="ts">
	// Organise: the events, matches and boards you run, by when they happen.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import SummaryPane from '#lib/components/organise/SummaryPane.svelte';
	import { eventMessage } from '#lib/events.ts';
	import {
		bySegment,
		getOrganised,
		organiseRows,
		segmentFromQuery,
		type OrganiseRow,
		type Segment
	} from '#lib/organise.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { Organised } from '#lib/types.ts';

	const t = strings.organise;
	const SEGMENTS: Segment[] = ['coming', 'running', 'past'];

	let data = $state.raw<Organised | null>(null);
	let loadFailure = $state<string | null>(null);
	let wide = $state(false);
	// Read once the data arrives; rows are the same for the life of the page.
	let now = $state(Math.floor(Date.now() / 1000));

	const all = $derived(data ? organiseRows(data, now) : []);
	const seg = $derived(segmentFromQuery(new URLSearchParams(page.url.search)));
	const rows = $derived(bySegment(all, seg));
	const counts = $derived(Object.fromEntries(SEGMENTS.map((s) => [s, bySegment(all, s).length])) as Record<Segment, number>);
	const selected = $derived(rows.find((r) => r.key === page.url.searchParams.get('item')) ?? rows[0] ?? null);
	const selectedData = $derived.by(() => {
		if (!data || !selected) return null;
		const id = selected.key.slice(selected.key.indexOf(':') + 1);
		if (selected.kind === 'event') return data.events.find((e) => e.id === id) ?? null;
		if (selected.kind === 'match') return data.matches.find((m) => m.id === id) ?? null;
		return data.boards.find((b) => b.token === id) ?? null;
	});

	onMount(() => {
		const query = matchMedia('(min-width: 1100px)');
		const sync = () => (wide = query.matches);
		sync();
		query.addEventListener('change', sync);
		void (async () => {
			const s = await requireSignedIn();
			if (s.status === 'unavailable') loadFailure = strings.networkError;
			if (s.status !== 'signed_in') return;
			try {
				data = await getOrganised();
				now = Math.floor(Date.now() / 1000);
			} catch (e) {
				loadFailure = eventMessage(e);
			}
		})();
		return () => query.removeEventListener('change', sync);
	});

	/** Change one query parameter in place: no new history entry, the other one is kept. */
	function setParam(name: 'seg' | 'item', value: string | null) {
		const query = new URLSearchParams(page.url.search);
		if (value === null) query.delete(name);
		else query.set(name, value);
		if (name === 'seg') query.delete('item');
		void goto(`?${query}`, { replace: true, reset: false });
	}

	function pick(event: MouseEvent, row: OrganiseRow) {
		if (!wide || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
		event.preventDefault();
		setParam('item', row.key);
	}
</script>

<main class="acct page-wide-desk">
	<header>
		<h1>{t.title}</h1>
		<a class="secondary" href="/new/event">{t.newEvent}</a>
	</header>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && data}
		<div class="switch" role="group" aria-label={t.segmentsLabel}>
			{#each SEGMENTS as s (s)}
				<button type="button" aria-pressed={seg === s} onclick={() => seg !== s && setParam('seg', s)}>
					{t.segment(t.segments[s], counts[s])}
				</button>
			{/each}
		</div>

		<div class="cols">
			<div class="listcol">
				{#if rows.length === 0}
					<p class="hint">{t.empty}</p>
				{:else}
					<ul class="items" aria-label={t.listLabel}>
						{#each rows as row (row.key)}
							<li>
								<a
									class="row {row.kind}"
									href={row.href}
									aria-current={wide && selected?.key === row.key ? 'true' : undefined}
									onclick={(e) => pick(e, row)}
								>
									<span class="what">
										<span class="first">
											<strong>{row.title}</strong>
											<span class="badge">{row.chip}</span>
										</span>
										<span class="hint">{row.place === null ? row.when : `${row.when} · ${row.place}`}</span>
										{#if row.needs}<span class="needs">{row.needs}</span>{/if}
									</span>
								</a>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
			{#if wide && selected && selectedData}
				<SummaryPane row={selected} data={selectedData} />
			{/if}
		</div>
	{:else if !loadFailure}
		<p class="hint">{strings.auth.loading}</p>
	{/if}
</main>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	header h1 {
		margin-right: auto;
	}
	.switch {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		margin-bottom: 12px;
		border: 1px solid var(--line);
		border-radius: 14px;
		overflow: hidden;
	}
	.switch button {
		min-height: var(--tap);
		min-width: 0;
		padding: 0 4px;
		border: 0;
		background: var(--panel);
		font-size: 15px;
		font-weight: 650;
		overflow-wrap: anywhere;
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
	.listcol {
		min-width: 0;
	}
	.items {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	a.row {
		flex-wrap: nowrap;
		justify-content: flex-start;
		border-left-width: 4px;
		color: inherit;
		text-decoration: none;
	}
	a.row.event {
		border-left-color: var(--ev);
	}
	a.row.match {
		border-left-color: var(--b);
	}
	a.row.board {
		border-left-color: var(--a);
	}
	a.row:focus-visible {
		outline: 2px solid var(--ink);
		outline-offset: 2px;
	}
	a.row[aria-current='true'] {
		background: var(--a-soft);
	}
	.first {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 6px;
	}
	.first .badge {
		margin-left: 0;
	}
	.needs {
		color: var(--ink);
		font-weight: 650;
	}
	@media (min-width: 700px) {
		.switch button {
			min-height: 40px;
		}
	}
	@media (min-width: 1100px) {
		.cols {
			display: grid;
			grid-template-columns: 380px 1fr;
			gap: 32px;
			align-items: start;
		}
	}
</style>
