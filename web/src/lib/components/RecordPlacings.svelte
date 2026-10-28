<script lang="ts">
	// A record as three big numbers, then any placings: the top of a player's own results and of a
	// public profile.
	import { dateText } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';
	import type { LinkedPlacing, PlayerRecord } from '#lib/types.ts';

	let { record, placings }: { record: PlayerRecord; placings: LinkedPlacing[] } = $props();

	const t = strings.history;
</script>

<dl class="record" aria-label={t.record}>
	{#each t.recordParts as part (part.key)}
		<div>
			<dt><span aria-hidden="true">{part.short}</span><span class="visually-hidden">{part.long}</span></dt>
			<dd>{record[part.key]}</dd>
		</div>
	{/each}
</dl>

{#if placings.length > 0}
	<h2>{t.placings}</h2>
	<ul class="list" aria-label={t.placings}>
		{#each placings as p (p.event_id)}
			{@const placing = t.placing(p.place, p.place_to, p.field_size, p.event_kind)}
			<li class="row">
				<!-- The event's name only where the viewer may see it (their own, or this coordinator's). -->
				<span>{p.event_name ? t.placingIn(p.event_name, placing) : placing}</span>
				<time class="hint" datetime={new Date(p.finished_at * 1000).toISOString()}
					>{dateText(p.finished_at)}</time
				>
			</li>
		{/each}
	</ul>
{/if}

<style>
	.record {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 8px;
		margin: 0 0 8px;
	}
	.record div {
		display: flex;
		flex-direction: column-reverse;
		align-items: center;
		padding: 10px 8px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.record dt {
		color: var(--muted);
		font-size: 13px;
		font-weight: 650;
	}
	.record dd {
		margin: 0;
		font-size: 40px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}
</style>
