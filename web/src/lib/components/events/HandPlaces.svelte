<!--
	Owned by phase 6 (the events foundation). The Finish screen when the coordinator supplies the
	places (`places_computed` is false): the teams in finishing order, moved up and down, with
	ties marked between neighbours. It sends the places it previews.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { groupsFromTies, moveItem, placesFromOrder, teamText } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, Place } from '#lib/types.ts';

	const t = strings.events;

	let {
		event,
		busy,
		onfinish,
		oncancel
	}: {
		event: EventDetail;
		busy: boolean;
		onfinish: (places?: Place[]) => void;
		oncancel: () => void;
	} = $props();

	// The team order when the screen opened; the event's teams can't change while running.
	const opened = untrack(() => event.teams.map((x) => x.id));
	let order = $state<string[]>(opened);
	let tied = $state<boolean[]>(opened.slice(1).map(() => false));

	const preview = $derived(placesFromOrder(groupsFromTies(order, tied)));
	const nameOf = (tid: string) => {
		const tm = event.teams.find((x) => x.id === tid);
		return tm ? teamText(tm) : t.unknownTeam;
	};

	function shift(i: number, by: number) {
		order = moveItem(order, i, i + by);
	}
</script>

<p class="hint">{t.finishHint}</p>
<ol class="places">
	{#each order as tid, i (tid)}
		{@const place = preview.find((p) => p.team_id === tid)}
		{#if i > 0}
			<li class="tie">
				<label>
					<input type="checkbox" bind:checked={tied[i - 1]} aria-label={t.tiedLabel(nameOf(order[i - 1] ?? ''), nameOf(tid))} />
					<span aria-hidden="true">{t.tied}</span>
				</label>
			</li>
		{/if}
		<li class="row place-row">
			<span class="pos">{place ? t.place(place.place, place.place_to) : ''}</span>
			<span class="what"><strong>{nameOf(tid)}</strong></span>
			<div class="actions">
				<button class="secondary" type="button" disabled={i === 0} aria-label={t.moveUp(nameOf(tid))} onclick={() => shift(i, -1)}>{t.up}</button>
				<button class="secondary" type="button" disabled={i === order.length - 1} aria-label={t.moveDown(nameOf(tid))} onclick={() => shift(i, 1)}>{t.down}</button>
			</div>
		</li>
	{/each}
</ol>
<div class="actions">
	<button class="primary" type="button" disabled={busy} onclick={() => onfinish(preview)}>{busy ? t.finishing : t.finishConfirm}</button>
	<button class="secondary" type="button" onclick={oncancel}>{t.cancel}</button>
</div>

<style>
	.places {
		list-style: none;
		margin: 0 0 12px;
		padding: 0;
		display: grid;
		gap: 4px;
	}
	.tie label {
		flex-direction: row;
		align-items: center;
		justify-content: center;
		gap: 8px;
		min-height: var(--tap);
		color: var(--ink);
		font-size: 14px;
		cursor: pointer;
	}
	.tie input {
		width: 22px;
		min-height: 22px;
		height: 22px;
		margin: 0;
	}
	.pos {
		flex: none;
		min-width: 4.5rem;
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.place-row .what {
		flex: 1 1 6rem;
	}
</style>
