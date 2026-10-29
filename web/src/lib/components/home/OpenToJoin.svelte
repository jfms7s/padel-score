<script lang="ts">
	// Events and matches anyone can join.
	import { itemHref } from '#lib/calendar.ts';
	import { browserTimeZone, dayLabel, timeText } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';
	import type { OpenItem } from '#lib/types.ts';

	let { open }: { open: OpenItem[] } = $props();

	const t = strings.calendar;
	const zone = browserTimeZone();

	function openText(o: OpenItem): string {
		const when = `${dayLabel(o.starts_at, zone)} · ${timeText(o.starts_at, zone)}`;
		return o.kind === 'event'
			? `${when} · ${t.teamsOpen(o.teams, o.max_teams)}`
			: `${when} · ${t.placesOpen(o.open_places)}`;
	}
</script>

<section aria-labelledby="open">
	<h2 id="open">{t.openToJoin}</h2>
	{#if open.length === 0}
		<p class="hint">{t.openNone}</p>
	{:else}
		<ul>
			{#each open as o (`${o.kind}:${o.join_token}`)}
				<li>
					<a class="row {o.kind}" href={itemHref(o)}>
						<span class="title">{o.kind === 'event' ? o.name : strings.home.matchBy(o.owner_name)}</span>
						<span class="sub">{openText(o)}{o.place ? ` · ${o.place}` : ''}</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.row {
		display: grid;
		min-height: var(--tap);
		padding: 8px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.row.event {
		border-left: 4px solid var(--ev);
	}
	.row.match {
		border-left: 4px solid var(--b);
	}
	.title {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.sub {
		font-size: 13px;
		color: var(--muted);
	}
</style>
