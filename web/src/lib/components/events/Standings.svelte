<!--
	Owned by phase 8 (the non-stop). The standings table: the place shown once per shared range,
	level teams highlighted. Headers are the short W D L so it fits 390 px.
-->
<script lang="ts">
	import { teamText } from '#lib/events.ts';
	import { placeText, type StandingRow } from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventTeam } from '#lib/types.ts';

	const t = strings.nonStop;

	let { rows, teams }: { rows: StandingRow[]; teams: EventTeam[] } = $props();

	const nameOf = (id: string) => {
		const tm = teams.find((x) => x.id === id);
		return tm ? teamText(tm) : strings.events.unknownTeam;
	};
	const shared = (r: StandingRow) => r.place_to > r.place;
	// A shared range is printed on its first row only.
	const shown = (i: number) => i === 0 || rows[i - 1].place !== rows[i].place;
</script>

<table class="standings">
	<caption class="sr">{t.standings}</caption>
	<thead>
		<tr>
			<th scope="col" class="pl">#</th>
			<th scope="col" class="tm">{t.team}</th>
			<th scope="col" title={t.played}>{t.played}</th>
			<th scope="col">{t.won}</th>
			<th scope="col">{t.drawn}</th>
			<th scope="col">{t.lost}</th>
			<th scope="col" title={t.games}>G</th>
			<th scope="col">{t.points}</th>
		</tr>
	</thead>
	<tbody>
		{#each rows as r, i (r.team_id)}
			<tr class:level={shared(r)}>
				<td class="pl">{shown(i) ? placeText(r) : ''}</td>
				<td class="tm">{nameOf(r.team_id)}</td>
				<td>{r.played}</td>
				<td>{r.won}</td>
				<td>{r.drawn}</td>
				<td>{r.lost}</td>
				<td>{r.games_won}</td>
				<td class="pts">{r.points}</td>
			</tr>
		{/each}
	</tbody>
</table>

<style>
	.standings {
		width: 100%;
		border-collapse: collapse;
		table-layout: fixed;
		font-size: 14px;
	}
	th,
	td {
		padding: 8px 2px;
		text-align: center;
		border-bottom: 1px solid var(--line);
		font-feature-settings: 'tnum';
	}
	th {
		color: var(--muted);
		font-weight: 600;
		font-size: 13px;
	}
	.pl {
		width: 2.8rem;
		font-weight: 750;
	}
	.tm {
		text-align: left;
		overflow-wrap: anywhere;
		padding-left: 6px;
	}
	th:nth-child(n + 3),
	td:nth-child(n + 3) {
		width: 2.1rem;
	}
	.pts {
		font-weight: 750;
	}
	tr.level {
		background: var(--a-soft);
	}
	.sr {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
	}
</style>
