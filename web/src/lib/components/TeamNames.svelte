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
			viewBox="-13.6 -17.6 28.6 35"
			aria-hidden="true"
			focusable="false"
			><g transform="rotate(35) translate(0 -18.4)" fill-rule="evenodd"
				><path d="M0 0C9.5 0 10 12 5 19H-5C-10 12-9.5 0 0 0ZM-4.65 4.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-1.25 4.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM2.15 4.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-6.35 8a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-2.95 8a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM0.45 8a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM3.85 8a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-4.65 11.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-1.25 11.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM2.15 11.5a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM-2.95 15a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0ZM0.45 15a1.25 1.25 0 1 0 2.5 0a1.25 1.25 0 1 0 -2.5 0Z" /><path d="M-5.2 17.5H5.2L1.9 27H-1.9ZM-3 19.6H3L0 25Z" /><rect
					x="-1.9"
					y="26.5"
					width="3.8"
					height="9"
					rx="1.3"
				/><rect x="-2.4" y="34.8" width="4.8" height="2" rx="1" /></g
			></svg
		><span class="visually-hidden">{strings.fence.servingMark}</span>{/if}
{/snippet}

{@render player(0)}{strings.teamNameSeparator}{@render player(1)}

<style>
	/*
	 * A padel racket tilted as if held: a holed head, the open throat, the grip. About 1.3 times
	 * the name's cap height, centred on the capitals; the negative margins keep the line from
	 * growing.
	 */
	.racket {
		display: inline-block;
		height: 1.3em;
		width: 1.06em;
		margin: -0.2em 0 -0.2em 0.25em;
		vertical-align: -0.1em;
		fill: currentColor;
	}
</style>
