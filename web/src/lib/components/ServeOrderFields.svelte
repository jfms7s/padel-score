<!--
	Who serves first, as three two-button choices that are always picked: the serving team, then
	each team's first server (the engine's start is built from them, see serve.ts). Used on New
	match, where the names may still be blank, and inside ServePicker on the court phone.
-->
<script lang="ts">
	import type { ServeOrder } from '#lib/serve.ts';
	import { strings } from '#lib/strings.ts';
	import type { Team, Teams } from '#lib/types.ts';

	let {
		order = $bindable(),
		teams
	}: {
		order: ServeOrder;
		/** The names to label the buttons with; a blank one shows as "Player N". */
		teams: Teams;
	} = $props();

	const t = strings.serve;
	const uid = $props.id();
	const TEAMS: Team[] = ['a', 'b'];
	const INDEXES = [0, 1] as const;

	/** A player's name, or "Player N" (N = 1..4 across team A then team B) while it is blank. */
	const nameOf = (team: Team, index: 0 | 1) =>
		teams[team][index].trim() || strings.setup.player((team === 'a' ? 0 : 2) + index + 1);
	const teamLabel = (team: Team) => strings.teamName(nameOf(team, 0), nameOf(team, 1));
</script>

<div class="field" role="group" aria-labelledby="{uid}-first">
	<span class="label" id="{uid}-first">{t.first}</span>
	<div class="seg">
		{#each TEAMS as team (team)}
			<button type="button" class={team} aria-pressed={order.team === team} onclick={() => (order.team = team)}>
				{teamLabel(team)}
			</button>
		{/each}
	</div>
	<p class="hint">{t.hint}</p>
</div>

<div class="pairs">
	{#each TEAMS as team (team)}
		<div class="field" role="group" aria-labelledby="{uid}-{team}">
			<span class="label" id="{uid}-{team}">{t.firstFor(teamLabel(team))}</span>
			<div class="seg">
				{#each INDEXES as index (index)}
					<button type="button" class={team} aria-pressed={order.first[team] === index} onclick={() => (order.first[team] = index)}>
						{nameOf(team, index)}
					</button>
				{/each}
			</div>
		</div>
	{/each}
</div>

<style>
	.field {
		min-width: 0;
	}
	.label {
		display: block;
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 6px;
		overflow-wrap: anywhere;
	}
	.seg {
		display: flex;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 12px;
		padding: 3px;
	}
	.seg button {
		flex: 1;
		min-width: 0;
		min-height: var(--tap);
		font-size: 14px;
		border: 0;
		background: transparent;
		color: var(--ink);
		padding: 8px 4px;
		border-radius: 9px;
		overflow-wrap: anywhere;
		touch-action: manipulation;
	}
	.seg button.a[aria-pressed='true'] {
		background: var(--a);
		color: var(--on-team);
		font-weight: 650;
	}
	.seg button.b[aria-pressed='true'] {
		background: var(--b);
		color: var(--on-team);
		font-weight: 650;
	}
	.pairs {
		display: grid;
		grid-template-columns: 1fr;
		gap: 16px;
	}
	@media (min-width: 640px) {
		.pairs {
			grid-template-columns: 1fr 1fr;
			gap: 8px;
		}
	}
	.hint {
		font-size: 12px;
		color: var(--muted);
		margin: 6px 0 0;
	}
</style>
