<!--
	Who serves first in a match made elsewhere (an event match on the court phone): the four
	players, then the other pair's first server, as on New match. `onstart` saves the choice.
-->
<script lang="ts">
	import { strings } from '#lib/strings.ts';
	import type { CourtPlayer, Start, Team, Teams } from '#lib/types.ts';

	let {
		teams,
		start,
		busy = false,
		failure = null,
		onstart
	}: {
		teams: Teams;
		/** The match's current choice, shown picked at first. */
		start: Start;
		busy?: boolean;
		failure?: string | null;
		onstart: (start: Start) => void;
	} = $props();

	const t = strings.court;
	const uid = $props.id();
	const TEAMS: Team[] = ['a', 'b'];
	const INDEXES = [0, 1] as const;

	// Seeded once from the match's choice; the phone's own taps change it after that.
	// svelte-ignore state_referenced_locally
	let first = $state<CourtPlayer>({ ...start.first_server });
	// svelte-ignore state_referenced_locally
	let other = $state<number>(start.other_first_server);

	const otherTeam = $derived<Team>(first.team === 'a' ? 'b' : 'a');

	function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		onstart({ first_server: { ...first }, other_first_server: other });
	}
</script>

<form class="serve" onsubmit={submit}>
	<div class="field" role="group" aria-labelledby="{uid}-first">
		<h2 id="{uid}-first">{t.serveTitle}</h2>
		<div class="grid">
			{#each TEAMS as team (team)}
				{#each INDEXES as index (index)}
					<button
						type="button"
						class="pick {team}"
						aria-pressed={first.team === team && first.index === index}
						onclick={() => (first = { team, index })}
					>
						{teams[team][index]}
					</button>
				{/each}
			{/each}
		</div>
		<p class="hint">{t.serveHint}</p>
	</div>

	<div class="field" role="group" aria-labelledby="{uid}-other">
		<h3 id="{uid}-other">{t.otherServer}</h3>
		<div class="grid">
			{#each INDEXES as index (index)}
				<button type="button" class="pick {otherTeam}" aria-pressed={other === index} onclick={() => (other = index)}>
					{teams[otherTeam][index]}
				</button>
			{/each}
		</div>
	</div>

	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
	<button class="go" type="submit" disabled={busy}>{busy ? t.starting : t.startScoring}</button>
</form>

<style>
	.serve {
		display: flex;
		flex-direction: column;
		gap: 20px;
	}
	.field {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	h2,
	h3 {
		margin: 0;
		font-weight: 750;
	}
	h2 {
		font-size: 22px;
	}
	h3 {
		font-size: 17px;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
	}
	.pick {
		min-height: 64px;
		padding: 8px 10px;
		border-radius: 14px;
		border: 2px solid var(--line);
		background: var(--panel);
		color: var(--ink);
		font-size: 18px;
		font-weight: 700;
		overflow-wrap: anywhere;
		touch-action: manipulation;
	}
	.pick.a[aria-pressed='true'] {
		background: var(--a);
		border-color: var(--a);
		color: var(--on-team);
	}
	.pick.b[aria-pressed='true'] {
		background: var(--b);
		border-color: var(--b);
		color: var(--on-team);
	}
	.hint {
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	.error {
		margin: 0;
		padding: 10px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	.go {
		min-height: 64px;
		border: 0;
		border-radius: 16px;
		background: var(--ink);
		color: var(--glass);
		font-size: 20px;
		font-weight: 750;
	}
	.go:disabled {
		opacity: 0.6;
	}
</style>
