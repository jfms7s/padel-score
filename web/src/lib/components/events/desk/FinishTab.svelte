<!--
	The desk's Finish tab while the event runs: Finish event, then the kind's finishing flow,
	with a warning for the matches still unfinished.
-->
<script lang="ts">
	import KindFinish from '#lib/components/events/KindFinish.svelte';
	import { unfinishedCount } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventMatch, Place } from '#lib/types.ts';

	let {
		detail,
		matches,
		busy,
		failure,
		onopen,
		onfinish
	}: {
		detail: EventDetail;
		matches: EventMatch[];
		busy: boolean;
		failure: string | undefined;
		/** Opening the flow clears the page's failures. */
		onopen: () => void;
		/** With the places, or with none when the kind computes them (see KindFinish). */
		onfinish: (places?: Place[]) => void;
	} = $props();

	const t = strings.events;

	let finishing = $state(false);
	const unfinished = $derived(unfinishedCount(matches));

	function open() {
		finishing = true;
		onopen();
	}
</script>

<section aria-labelledby="h-lifecycle">
	<h2 id="h-lifecycle">{finishing ? t.finishTitle : t.finish}</h2>
	{#if finishing}
		{#if unfinished > 0}<p class="warn" role="status">{t.unfinishedWarning(unfinished)}</p>{/if}
		<KindFinish event={detail} {busy} {onfinish} oncancel={() => (finishing = false)} />
	{:else}
		<button class="primary wide" type="button" disabled={busy} onclick={open}>{t.finish}</button>
	{/if}
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

<style>
	section {
		margin-top: 8px;
		max-width: 28rem;
	}
	.wide {
		width: 100%;
		margin-top: 12px;
	}
	p.error {
		margin-top: 12px;
	}
	p.warn {
		margin: 0 0 12px;
		color: var(--warn);
		font-weight: 650;
	}
</style>
