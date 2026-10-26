<!--
	Owned by phase 8 (the non-stop). The Finish screen once the non-stop computes its own places
	(`places_computed` is true): it previews the standings and finishes with no places.
-->
<script lang="ts">
	import { nonStopData } from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, Place } from '#lib/types.ts';
	import Standings from './Standings.svelte';

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

	const data = $derived(nonStopData(event.kind_data));
</script>

{#if data}
	<h3>{strings.nonStop.finishPreview}</h3>
	<Standings rows={data.standings} teams={event.teams} />
{/if}
<p class="hint">{strings.nonStop.computedPlacesHint}</p>
<div class="actions">
	<button class="primary" type="button" disabled={busy} onclick={() => onfinish()}>{busy ? t.finishing : t.finish}</button>
	<button class="secondary" type="button" onclick={oncancel}>{t.cancel}</button>
</div>

<style>
	h3 {
		margin: 0 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
</style>
