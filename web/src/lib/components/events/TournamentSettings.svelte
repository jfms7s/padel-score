<!--
	Owned by phase 7 (the tournament). The tournament's own settings on the new-event form: one switch,
	"Losers keep playing" (off unless the settings say otherwise), and the team counts that work.
-->
<script lang="ts">
	import { strings } from '#lib/strings.ts';

	let {
		settings,
		onchange
	}: {
		settings: Record<string, unknown>;
		onchange: (settings: Record<string, unknown>) => void;
	} = $props();

	const t = strings.tournament;
	const on = $derived(settings.full_placement === true);
</script>

<div class="tournament-settings">
	<label class="switch">
		<input
			type="checkbox"
			role="switch"
			checked={on}
			aria-describedby="h-full-placement"
			onchange={(e) => onchange({ full_placement: e.currentTarget.checked })}
		/>
		<span class="name">{t.fullPlacement}</span>
	</label>
	<p class="hint" id="h-full-placement">{on ? t.fullPlacementOn : t.fullPlacementOff}</p>
	<p class="hint">{t.teamCounts}</p>
</div>

<style>
	.tournament-settings {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.switch {
		display: flex;
		flex-direction: row;
		align-items: center;
		gap: 12px;
		min-height: var(--tap);
		cursor: pointer;
	}
	.switch input {
		flex: none;
		width: 22px;
		height: 22px;
		min-height: 22px;
		margin: 0;
	}
	.name {
		font-weight: 650;
	}
	.hint {
		margin: 0;
	}
</style>
