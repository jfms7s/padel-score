<!--
	The desk's Live tab: a "needs you" alert for a free court, the court tiles, then the kind's
	tables beside a match list that takes typed results (stacked below 1100 px, the kind first).
	A phone shows only the waiting matches, as "Up next", and leaves the kind to its own tab.
	Once finished, the final places come first.
-->
<script lang="ts">
	import CourtGrid from '#lib/components/events/CourtGrid.svelte';
	import CourtTile from '#lib/components/events/CourtTile.svelte';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import MatchList from '#lib/components/events/MatchList.svelte';
	import { MediaQuery } from 'svelte/reactivity';
	import { groupMatches, isJumpKey } from '#lib/desk.ts';
	import { teamText } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtOut, EventDetail, EventMatch, TypedScore } from '#lib/types.ts';

	let {
		detail,
		matches,
		busy,
		failure,
		onnext,
		onsave
	}: {
		detail: EventDetail;
		/** The matches in their order. */
		matches: EventMatch[];
		busy: boolean;
		/** A failed Next match. */
		failure: string | undefined;
		onnext: (court: CourtOut) => void;
		onsave: (m: EventMatch, score: TypedScore, clientId: string) => Promise<void>;
	} = $props();

	const t = strings.desk;

	/** The list that takes `/` (the one on screen). */
	let list = $state<{ openFirst: () => boolean } | undefined>();
	function jump(e: KeyboardEvent) {
		if (isJumpKey(e) && list?.openFirst()) e.preventDefault();
	}
	const te = strings.events;

	const status = $derived(detail.event.status);
	const running = $derived(status === 'running');
	const courts = $derived(detail.courts);
	const teams = $derived(detail.teams);

	/** The phone, tablet and desktop layouts differ in what they show, not only in where. */
	type Size = 'phone' | 'tablet' | 'desk';
	// Read at once, so a wide screen doesn't flash the phone layout first.
	const tablet = new MediaQuery('(min-width: 700px)');
	const desk = new MediaQuery('(min-width: 1100px)');
	const size = $derived<Size>(desk.current ? 'desk' : tablet.current ? 'tablet' : 'phone');

	const byId = (mid: string | null) => (mid === null ? undefined : matches.find((m) => m.id === mid));
	/** The first court whose match is over while another waits for it. */
	const freeCourt = $derived(
		running ? courts.find((c) => byId(c.current)?.state === 'finished' && c.queue.length > 0) : undefined
	);
	const waiting = $derived(groupMatches(matches, courts).waiting);

	const finalPlaces = $derived([...(detail.places ?? [])].sort((x, y) => x.place - y.place));
	const nameOf = (tid: string) => {
		const tm = teams.find((x) => x.id === tid);
		return tm ? teamText(tm) : te.unknownTeam;
	};
</script>

{#if status === 'finished'}
	<section aria-labelledby="h-places">
		<h2 id="h-places">{te.finishTitle}</h2>
		<ol class="list final">
			{#each finalPlaces as p (p.team_id)}
				<li class="row">
					<span class="pos">{te.place(p.place, p.place_to)}</span>
					<span class="what"><strong>{nameOf(p.team_id)}</strong></span>
				</li>
			{/each}
		</ol>
	</section>
{/if}

<svelte:window onkeydown={jump} />

<div class="live">
	{#if freeCourt}
		{@const court = freeCourt}
		<div class="alert" role="status">
			<span>{t.freeCourt(court.name)}</span>
			<button type="button" disabled={busy} onclick={() => onnext(court)}>{t.nextMatch}</button>
		</div>
	{/if}
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}

	{#if courts.length > 0}
		<CourtGrid count={courts.length}>
			{#each courts as court (court.id)}
				<CourtTile {court} {matches} {teams} {busy} compact={size !== 'desk'} onnext={running ? () => onnext(court) : undefined} />
			{/each}
		</CourtGrid>
	{/if}

	{#if size === 'phone'}
		{#if waiting.length > 0}
			<section class="panel" aria-labelledby="h-up-next">
				<h2 id="h-up-next">{t.upNext}</h2>
				<MatchList bind:this={list} matches={waiting} {courts} {teams} format={detail.event.format} mode="entry" onsave={running ? onsave : undefined} {busy} />
			</section>
		{/if}
	{:else}
		<div class="live-split">
			<div class="kind">
				<KindPanel event={detail} audience="coordinator" parts="tables" />
			</div>
			{#if matches.length > 0}
				<section class="panel" aria-labelledby="h-live-matches">
					<h2 id="h-live-matches">{te.matches}</h2>
					<MatchList
						bind:this={list}
						{matches}
						{courts}
						{teams}
						format={detail.event.format}
						mode={running ? 'entry' : 'view'}
						onsave={running ? onsave : undefined}
						{busy}
					/>
				</section>
			{/if}
		</div>
	{/if}
</div>

<style>
	.live {
		display: grid;
		gap: 16px;
		margin-top: 12px;
	}
	.alert {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 10px;
		padding: 10px 12px;
		border-radius: 14px;
		background: var(--ball);
		color: #0b2236;
		font-size: 15px;
		font-weight: 650;
	}
	.alert span {
		flex: 1 1 8rem;
		overflow-wrap: anywhere;
	}
	.alert button {
		min-height: var(--tap);
		padding: 0 16px;
		border: 0;
		border-radius: 12px;
		background: #0b2236;
		color: #fff;
		font: inherit;
		font-weight: 650;
		cursor: pointer;
	}
	.alert button:disabled {
		opacity: 0.6;
		cursor: default;
	}
	.live-split {
		display: grid;
		gap: 16px;
		align-items: start;
	}
	.live-split > * {
		min-width: 0;
	}
	/* The kind's own sections bring their top margin; the column starts flush. */
	.kind :global(section:first-child),
	.kind :global(h3:first-child) {
		margin-top: 0;
	}
	.kind :global(h2:first-child) {
		margin-top: 0;
	}
	.panel {
		border: 1px solid var(--line);
		border-radius: 16px;
		background: var(--panel);
		overflow: hidden;
	}
	.panel h2 {
		margin: 0;
		padding: 12px 14px;
		font-size: 16px;
		border-bottom: 1px solid var(--line);
	}
	.row .what {
		flex: 1 1 10rem;
	}
	.pos {
		flex: none;
		min-width: 4.5rem;
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.final {
		margin: 0;
	}
	p.error {
		margin: 0;
	}
	@media (min-width: 1100px) {
		.live-split {
			grid-template-columns: minmax(0, 1fr) 360px;
		}
	}
</style>
