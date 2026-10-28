<!--
	The desk's header on every tab: the event's name, kind, rules and status, the spectator
	link to copy, Start (in setup) or a link to the Finish tab (while running), and a running
	non-stop's clock with its buttons.
-->
<script lang="ts">
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import { rulesText } from '#lib/events.ts';
	import { scheduleText } from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail } from '#lib/types.ts';

	let {
		detail,
		busy,
		copied,
		copyFailed,
		oncopy,
		onstart,
		onfinish
	}: {
		detail: EventDetail;
		busy: boolean;
		/** The key of the link just copied; this one's is `spectator`. */
		copied: string | null;
		/** The key of the link that couldn't be copied. */
		copyFailed: string | null;
		oncopy: () => void;
		onstart: () => void;
		/** Opens the Finish tab. */
		onfinish: () => void;
	} = $props();

	const t = strings.events;
	const d = strings.desk;

	const ev = $derived(detail.event);
	const status = $derived(ev.status);

	function finishClick(e: MouseEvent) {
		if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
		e.preventDefault();
		onfinish();
	}
</script>

<header class="head">
	<div class="title">
		<h1>{ev.name}</h1>
		<p class="meta">
			<span>{t.kinds[ev.kind]}</span>
			<span class="badge" class:off={status === 'finished' || status === 'cancelled'}>{t.status[status]}</span>
		</p>
		{#if scheduleText(ev.schedule) !== ''}<p class="when">{scheduleText(ev.schedule)}</p>{/if}
		<p class="hint">{rulesText(ev.format)}</p>
	</div>
	<div class="acts">
		<button class="secondary" type="button" onclick={oncopy}>{copied === 'spectator' ? t.copied : d.spectatorLink}</button>
		{#if status === 'setup'}
			<button class="primary" type="button" disabled={busy} onclick={onstart}>{t.start}</button>
		{:else if status === 'running'}
			<a class="secondary" href="?tab=finish" onclick={finishClick}>{t.finish}</a>
		{/if}
	</div>
	{#if status === 'setup'}<p class="hint start-hint">{t.startHint}</p>{/if}
	{#if copyFailed === 'spectator'}<p class="hint start-hint" role="alert">{t.copyFailed}</p>{/if}
</header>

{#if ev.kind === 'non_stop' && status === 'running'}
	<div class="clock-strip">
		<KindPanel event={detail} audience="coordinator" parts="clock" clockVariant="strip" />
	</div>
{/if}

<style>
	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		gap: 8px 16px;
	}
	.title {
		flex: 1 1 16rem;
		min-width: 0;
	}
	.title h1 {
		margin-bottom: 4px;
		overflow-wrap: anywhere;
	}
	.meta {
		display: flex;
		align-items: center;
		gap: 4px;
		margin: 0 0 4px;
	}
	.when {
		margin: 0 0 4px;
		font-weight: 650;
	}
	.title .hint {
		margin: 0;
	}
	.acts {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.start-hint {
		flex-basis: 100%;
		margin: 0;
	}
	.clock-strip {
		margin-top: 12px;
	}
</style>
