<script lang="ts">
	// A team's names, "Ana / Rui", with a racket beside the player serving the next point. Only a
	// named server gets the racket (see TeamModel.servingIndex); screen readers hear "Ana serving / Rui".
	import { strings } from '#lib/strings.ts';
	import type { TeamModel } from '#lib/view.ts';

	let { team }: { team: TeamModel } = $props();
</script>

{#snippet player(i: 0 | 1)}
	{team.players[i]}{#if team.servingIndex === i}<svg
			class="racket"
			viewBox="0 0 20 28"
			aria-hidden="true"
			focusable="false"
			><path
				fill-rule="evenodd"
				d="M1.5 9.5a8.5 9 0 1 0 17 0a8.5 9 0 1 0 -17 0Z M5.8 7a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0Z M11.8 7a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0Z M8.8 10.2a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0Z M5.8 13.4a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0Z M11.8 13.4a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0 -2.4 0Z"
			/><path d="M7.4 17.6 10 20.6 12.6 17.6Z" /><rect x="8.6" y="19.4" width="2.8" height="8.4" rx="1.4" /></svg
		><span class="visually-hidden">{strings.fence.servingMark}</span>{/if}
{/snippet}

{@render player(0)}{strings.teamNameSeparator}{@render player(1)}

<style>
	.racket {
		display: inline-block;
		width: 0.72em;
		height: 1em;
		margin-left: 0.22em;
		vertical-align: -0.14em;
		fill: currentColor;
	}
</style>
