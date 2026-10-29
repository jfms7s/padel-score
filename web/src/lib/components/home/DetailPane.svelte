<script lang="ts">
	// The selected item of the week grid: when, where, who, and the way to open it.
	import { itemHref, itemTime } from '#lib/calendar.ts';
	import { longDate } from '#lib/dates.ts';
	import { agendaNote, agendaTitle } from '#lib/home.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem } from '#lib/types.ts';

	let { item, timeZone }: { item: CalendarItem | null; timeZone: string } = $props();

	const start = $derived(item ? (item.kind === 'game' ? item.start : item.starts_at) : 0);
	const place = $derived(item ? (item.kind === 'game' ? item.board_place : item.place) : null);
</script>

{#if item}
	<div class="pane">
		<h3>{agendaTitle(item)}</h3>
		<p class="when">{longDate(start, timeZone)} · {itemTime(item, timeZone)}</p>
		{#if place}<p class="place">{place}</p>{/if}
		{#if item.kind === 'game'}
			<p class="note">{item.board_title}</p>
			<ul>
				{#each item.players as player, i (i)}
					<li>{#if i === item.me}<b>{player}</b>{:else}{player}{/if}</li>
				{/each}
			</ul>
			<a class="primary" href={itemHref(item)}>{strings.home.openBoard}</a>
		{:else}
			{@const note = agendaNote(item)}
			{#if note}<p class="note">{note}</p>{/if}
			<a class="primary" href={itemHref(item)}>{strings.home.openPage}</a>
		{/if}
	</div>
{:else}
	<p class="none">{strings.home.nothing}</p>
{/if}

<style>
	.pane {
		display: grid;
		gap: 6px;
		padding: 14px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
	}
	h3 {
		margin: 0;
		font-size: 17px;
		font-weight: 750;
		overflow-wrap: anywhere;
	}
	p {
		margin: 0;
	}
	.when {
		font-weight: 650;
	}
	.place,
	.note,
	.none {
		color: var(--muted);
	}
	ul {
		margin: 0;
		padding-left: 18px;
	}
	.primary {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		margin-top: 6px;
		border-radius: 14px;
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
		text-decoration: none;
	}
</style>
