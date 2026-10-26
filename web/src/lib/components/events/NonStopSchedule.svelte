<!--
	Owned by phase 8 (the non-stop). The slots grouped by round: each match with its teams, its
	court and the live score or the result. The current round is open, finished ones collapsed.
-->
<script lang="ts">
	import { liveScoreText, teamText } from '#lib/events.ts';
	import type { NonStopData, SlotView } from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventMatch, EventView, EventViewMatch } from '#lib/types.ts';

	const t = strings.nonStop;

	let { data, event }: { data: NonStopData; event: EventDetail | EventView } = $props();

	type Row = EventMatch | EventViewMatch;

	const byId = $derived(new Map<string, Row>(event.matches.map((m) => [m.id, m])));
	const rounds = $derived.by(() => {
		const out: { round: number; slots: SlotView[] }[] = [];
		for (const s of data.slots) {
			const last = out[out.length - 1];
			if (last && last.round === s.round) last.slots.push(s);
			else out.push({ round: s.round, slots: [s] });
		}
		return out;
	});
	const currentRound = $derived(
		(data.clock && data.clock.state !== 'done' ? data.clock.round : null) ??
			rounds.find((r) => r.slots.some((s) => s.state !== 'done'))?.round ??
			null
	);

	const teamName = (id: string) => {
		const tm = event.teams.find((x) => x.id === id);
		return tm ? teamText(tm) : strings.events.unknownTeam;
	};
	const courtName = (id: string | null) => event.courts.find((c) => c.id === id)?.name;
	const scoreText = (m: Row) => {
		if (m.state === 'finished' && m.result) {
			const r = m.result;
			return r.sets.length > 0 ? r.sets.map((s) => `${s.a}-${s.b}`).join(' ') : `${r.games[0]}-${r.games[1]}`;
		}
		return m.live ? liveScoreText(m.live) : '';
	};
	const matchesOf = (s: SlotView) => s.matches.map((id) => byId.get(id)).filter((m): m is Row => m !== undefined);
</script>

<div class="schedule">
	{#each rounds as r (r.round)}
		<details open={r.round === currentRound}>
			<summary>{t.roundHeading(r.round)}</summary>
			{#each r.slots as s (s.slot)}
				{#if data.waves > 1}<h4>{t.waveOf(s.wave, data.waves)}</h4>{/if}
				<ul>
					{#each matchesOf(s) as m (m.id)}
						<li>
							<span class="vs">{teamName(m.team_a)} v {teamName(m.team_b)}</span>
							<span class="meta">
								{courtName(m.court_id) ? t.courtName(courtName(m.court_id) ?? '') : t.noCourt}
							</span>
							{#if scoreText(m)}<span class="score" class:live={m.state === 'live'}>{scoreText(m)}</span>{/if}
						</li>
					{/each}
				</ul>
			{/each}
		</details>
	{/each}
</div>

<style>
	.schedule {
		display: grid;
		gap: 8px;
	}
	details {
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		padding: 0 12px;
		min-width: 0;
	}
	summary {
		display: flex;
		align-items: center;
		min-height: var(--tap);
		font-weight: 700;
		cursor: pointer;
	}
	summary:focus-visible {
		outline: 2px solid var(--ink);
		outline-offset: 2px;
	}
	h4 {
		margin: 4px 0;
		font-size: 13px;
		color: var(--muted);
	}
	ul {
		list-style: none;
		margin: 0 0 8px;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	li {
		display: grid;
		gap: 2px;
		padding-top: 8px;
		border-top: 1px solid var(--line);
		min-width: 0;
	}
	.vs {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.meta {
		font-size: 13px;
		color: var(--muted);
	}
	.score {
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.score.live {
		color: var(--warn);
	}
</style>
