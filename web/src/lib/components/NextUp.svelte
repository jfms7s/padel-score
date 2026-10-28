<script lang="ts">
	// The next thing on the calendar, as one tappable card on Home.
	import { itemDetail, itemHref, itemNote, itemTime, itemTitle } from '#lib/calendar.ts';
	import { browserTimeZone, dayLabel } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';
	import type { CalendarItem } from '#lib/types.ts';

	let { item }: { item: CalendarItem } = $props();

	const zone = browserTimeZone();
	const start = $derived(item.kind === 'game' ? item.start : item.starts_at);
</script>

<a class="next" href={itemHref(item)}>
	<span class="eyebrow">{strings.calendar.nextUp}</span>
	<span class="title">{itemTitle(item)}</span>
	<span class="when">{dayLabel(start, zone)} · {itemTime(item, zone)} · {itemNote(item)}</span>
	{#if itemDetail(item)}<span class="when">{itemDetail(item)}</span>{/if}
</a>

<style>
	.next {
		display: grid;
		gap: 2px;
		min-height: var(--tap);
		margin-top: 8px;
		padding: 12px 14px;
		border: 2px solid var(--ball);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.eyebrow {
		font-size: 12px;
		font-weight: 700;
		color: var(--muted);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	.title {
		font-size: 17px;
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.when {
		color: var(--muted);
		font-size: 14px;
	}
</style>
