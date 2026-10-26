<script lang="ts">
	// The court viewer: a court's live score, read only, moving to each new match on the court.
	// It follows the court with the court's viewing token, and the current match's viewing
	// stream through the spectator scoreboard.
	import { page } from '$app/state';
	import { courtMatchText, goneReason, isCourtState } from '#lib/court.ts';
	import { courtStreamUrl, getCourt } from '#lib/events.ts';
	import { openSseStream } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtState } from '#lib/types.ts';
	import Viewer from '#lib/components/Viewer.svelte';
	import KindCourtBanner from '#lib/components/events/KindCourtBanner.svelte';

	const t = strings.court;
	const token = $derived(page.params.token ?? '');

	let court = $state.raw<CourtState | null>(null);
	let gone = $state(false);
	let loadFailed = $state(false);

	const current = $derived(court?.current ?? null);
	const next = $derived(court?.next ?? null);

	$effect(() => {
		const tk = token;
		court = null;
		gone = false;
		loadFailed = false;
		let live = true;
		getCourt(tk).then(
			(s) => {
				if (live) court = s;
			},
			(e: unknown) => {
				if (!live) return;
				if (goneReason(e)) gone = true;
				else loadFailed = true;
			}
		);
		const s = openSseStream(
			courtStreamUrl(tk),
			{ type: 'court', accept: isCourtState },
			(st) => {
				if (!live) return;
				court = st;
				loadFailed = false;
			},
			(status) => {
				if (live && status === 'gone') gone = true;
			}
		);
		return () => {
			live = false;
			s.close();
		};
	});

	const title = $derived(court ? `${court.court.name} · ${court.event.name}` : strings.appName);
	// Set here rather than in <svelte:head>: the scoreboard below sets its own title while it is
	// shown, and an unmounted one leaves its title behind, so this one is set again after it goes.
	$effect(() => {
		if (!(court !== null && current !== null)) document.title = title;
	});
</script>

{#snippet banner()}
	{#if court && current}
		<div class="banner">
			<KindCourtBanner {court} viewer />
			<p><strong>{court.court.name}</strong> · {current.label}</p>
			{#if next}<p class="next">{t.next(courtMatchText(next))}</p>{/if}
		</div>
	{/if}
{/snippet}

{#if court && current}
	<Viewer token={current.spectator_token} {banner} />
{:else}
	<main class="court-screen">
		<div class="center">
			{#if gone}
				<p class="big-text">{strings.fence.gone}</p>
			{:else if court}
				<p class="muted">{court.event.name}</p>
				<h1>{court.court.name}</h1>
				<p class="big-text">{t.waiting}</p>
			{:else if loadFailed}
				<p>{t.loadFailed}</p>
				<button type="button" class="big" onclick={() => location.reload()}>{t.reload}</button>
			{:else}
				<p class="muted">{t.viewerLoading}</p>
			{/if}
		</div>
	</main>
{/if}

<style>
	.court-screen {
		min-height: 100vh;
		min-height: 100dvh;
		background: var(--glass);
		color: var(--ink);
		padding: 16px;
	}
	.center {
		min-height: calc(100dvh - 32px);
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		text-align: center;
	}
	.center p {
		margin: 0;
	}
	h1 {
		margin: 0;
		font-size: clamp(28px, 8vw, 44px);
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		line-height: 1.1;
		overflow-wrap: anywhere;
	}
	.muted {
		color: var(--muted);
	}
	.big-text {
		font-size: 22px;
		font-weight: 700;
	}
	.big {
		min-height: 56px;
		padding: 0 24px;
		border: 0;
		border-radius: 16px;
		background: var(--ink);
		color: var(--glass);
		font-weight: 750;
	}
	.banner {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 12px;
		padding: 8px 16px;
		background: var(--panel);
		border-bottom: 1px solid var(--line);
		color: var(--ink);
		font-size: 15px;
	}
	.banner p {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.next {
		color: var(--muted);
	}
</style>
