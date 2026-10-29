<script lang="ts">
	// The game boards the user is in.
	import { browserTimeZone, dayLabel } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';
	import type { BoardStrip } from '#lib/types.ts';

	let { boards }: { boards: BoardStrip[] } = $props();

	const t = strings.calendar;
	const zone = browserTimeZone();
</script>

<section aria-labelledby="boards">
	<h2 id="boards">{t.boards}</h2>
	{#if boards.length > 0}
		<ul>
			{#each boards as b (b.token)}
				<li>
					<a class="row" href="/b/{encodeURIComponent(b.token)}">
						<span class="title">{b.title}</span>
						<span class="sub"
							>{dayLabel(b.starts_at, zone)} – {dayLabel(b.ends_at - 1, zone)} · {t.boardGames(b.my_games)}</span
						>
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
	ul {
		list-style: none;
		margin: 0 0 8px;
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
	.title {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.sub {
		font-size: 13px;
		color: var(--muted);
	}
</style>
