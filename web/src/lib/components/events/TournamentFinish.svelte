<!--
	Owned by phase 7 (the tournament). The Finish screen: the final positions worked out from the
	results, or why they can't be yet. It finishes with no places: the server computes them.
-->
<script lang="ts">
	import { teamText } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import { finishBlocker, tournamentData } from '#lib/tournament.ts';
	import type { EventDetail, Place } from '#lib/types.ts';

	const t = strings.events;
	const tt = strings.tournament;

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

	const data = $derived(tournamentData(event.kind_data));
	const places = $derived(data?.places ?? null);
	const blocker = $derived(data ? finishBlocker(data) : 'unfinished');
	const nameOf = (tid: string) => {
		const tm = event.teams.find((x) => x.id === tid);
		return tm ? teamText(tm) : t.unknownTeam;
	};
</script>

{#if places && blocker === null}
	<p class="hint">{t.finishComputedHint}</p>
	<ol class="places">
		{#each places as p (p.team_id)}
			<li class="row">
				<span class="pos">{t.place(p.place, p.place_to)}</span>
				<span class="what"><strong>{nameOf(p.team_id)}</strong></span>
			</li>
		{/each}
	</ol>
{:else}
	<p class="warn" role="status">{tt.finishedBlocker[blocker ?? 'unfinished']}</p>
{/if}
<div class="actions">
	<button class="primary" type="button" disabled={busy || blocker !== null} onclick={() => onfinish()}>{busy ? t.finishing : t.finish}</button>
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
	.pos {
		flex: none;
		min-width: 4.5rem;
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.what {
		flex: 1 1 6rem;
	}
	.warn {
		margin: 0 0 12px;
		color: var(--warn);
		font-weight: 650;
	}
</style>
