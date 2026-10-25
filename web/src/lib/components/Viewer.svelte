<script lang="ts">
	// The spectator view: the same scoreboard as the fence, read-only. It only ever reads
	// /api/view/{token}; there are no taps, undo, share or offline queue here.
	//
	// Used by a match's viewing link (/v/{token}) and by the court viewer (/cv/{token}), which
	// passes a `banner` (the court) and a new `token` whenever the court's match changes.
	import type { Snippet } from 'svelte';
	import { ApiError, getView, streamUrl } from '#lib/api.ts';
	import { latestView, loadLayout, nextLayout, saveLayout, setsSummary, type Layout } from '#lib/fence.ts';
	import { loadView, saveView } from '#lib/lastview.ts';
	import { matchLabel } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import { openStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { MatchView } from '#lib/types.ts';
	import { fenceModel } from '#lib/view.ts';
	import { keepAwake } from '#lib/wakelock.ts';
	import BoardView from '#lib/components/BoardView.svelte';
	import CourtView from '#lib/components/CourtView.svelte';

	const f = strings.fence;
	let {
		token,
		banner
	}: {
		token: string;
		/** Shown above the score. */
		banner?: Snippet;
	} = $props();

	let view = $state.raw<MatchView | null>(null);
	let loadError = $state<string | null>(null);
	let stream = $state<StreamStatus | null>(null);
	let layoutPref = $state<Layout>('auto');
	let landscape = $state(false);

	const model = $derived(view ? fenceModel(view) : null);
	const layout = $derived(layoutPref === 'auto' ? (landscape ? 'court' : 'board') : layoutPref);
	const offline = $derived(stream === 'offline');
	const syncLabel = $derived(offline ? f.offlineView : stream === null ? f.connecting : f.live);

	$effect(() => {
		const t = token;
		view = null;
		loadError = null;
		stream = null;

		let live = true;
		// Never show an older revision than the one already on screen.
		const accept = (v: MatchView) => {
			if (!live) return;
			view = latestView(view, v);
			saveView(safeStorage(), t, view);
		};
		const s = openStream(streamUrl('view', t), accept, (st) => {
			if (!live) return;
			stream = st;
			if (st === 'gone') loadError = f.gone;
		});
		getView(t).then(accept, (e: unknown) => {
			if (!live) return;
			// A retryable failure is just offline: the stream catches up once it is back.
			if (e instanceof ApiError && e.retryable) {
				// Cold open without a line: show the last view this device saw.
				const stored = loadView(safeStorage(), t);
				if (stored) {
					view = latestView(view, stored);
					stream ??= 'offline';
				}
				return;
			}
			loadError = e instanceof ApiError && e.code === 'not_found' ? f.gone : f.loadFailed;
			s.close();
		});
		// A spectator phone can hang on the fence too.
		const lock = keepAwake(navigator, document);
		return () => {
			live = false;
			s.close();
			lock.release();
		};
	});

	$effect(() => {
		layoutPref = loadLayout(safeStorage());
		const mq = matchMedia('(orientation: landscape)');
		landscape = mq.matches;
		const onChange = () => (landscape = mq.matches);
		mq.addEventListener('change', onChange);
		return () => mq.removeEventListener('change', onChange);
	});

	function toggleLayout() {
		layoutPref = nextLayout(layoutPref);
		saveLayout(safeStorage(), layoutPref);
	}

	const title = $derived(view ? matchLabel(view.teams) : strings.appName);
	const announce = $derived.by(() => {
		if (!model) return '';
		if (model.finished) return model.finished.winner ? f.wins(model[model.finished.winner].name) : f.draw;
		const { a, b } = model;
		return `${a.name} ${a.points}, ${b.name} ${b.points}. ${model.callout ?? ''}`;
	});
</script>

<svelte:head>
	<title>{title}</title>
</svelte:head>

{#snippet sync(tone: 'court' | 'board')}
	<span class="sync {tone}" class:off={offline} role="status">{syncLabel}</span>
{/snippet}

{#snippet tools(tone: 'court' | 'board')}
	<button type="button" class="tool {tone}" aria-label={f.layout[layoutPref]} title={f.layoutHint} onclick={toggleLayout}>
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
			{#if layoutPref === 'court'}
				<rect x="2" y="6" width="20" height="12" rx="2" fill="none" stroke="currentColor" stroke-width="2" />
				<path d="M12 6v12" stroke="currentColor" stroke-width="2" />
			{:else if layoutPref === 'board'}
				<rect x="6" y="2" width="12" height="20" rx="2" fill="none" stroke="currentColor" stroke-width="2" />
				<path d="M6 12h12" stroke="currentColor" stroke-width="2" />
			{:else}
				<path d="M4 9a8 8 0 0 1 14-4l2-2v6h-6l2-2a5 5 0 0 0-9 2zM20 15a8 8 0 0 1-14 4l-2 2v-6h6l-2 2a5 5 0 0 0 9-2z" fill="currentColor" />
			{/if}
		</svg>
	</button>
{/snippet}

{#snippet finishedBanner()}
	{#if model?.finished}
		{@const w = model.finished.winner}
		<div class="finished">
			<p class="winner">{w ? f.wins(model[w].name) : f.draw}</p>
			{#if setsSummary(model)}<p class="summary">{setsSummary(model)}</p>{/if}
		</div>
	{/if}
{/snippet}

<main class="fence {layout}" class:with-banner={!!banner}>
	{#if banner}<div class="fence-banner">{@render banner()}</div>{/if}
	{#if loadError}
		<div class="message">
			<p>{loadError}</p>
			<a class="home" href="/">{f.home}</a>
		</div>
	{:else if !model}
		<div class="message">
			<p>{f.loading}</p>
			<!-- Opened from the cached shell with no line: the score can't load until it is back. -->
			{#if offline}<p class="sync board off" role="status">{f.offlineView}</p>{/if}
		</div>
	{:else if layout === 'court'}
		<CourtView {model} banner={model.finished ? finishedBanner : undefined}>
			{#snippet controls()}
				<div class="court-left"></div>
				{@render sync('court')}
				<div class="court-right">{@render tools('court')}</div>
			{/snippet}
		</CourtView>
	{:else}
		<BoardView {model} banner={model.finished ? finishedBanner : undefined}>
			{#snippet meta()}
				{@render sync('board')}
				{@render tools('board')}
			{/snippet}
		</BoardView>
	{/if}
	<p class="visually-hidden" aria-live="polite">{announce}</p>
</main>

<style>
	.fence {
		position: relative;
		background: var(--glass);
	}
	.fence.court {
		height: 100vh;
		height: 100dvh;
		overflow: hidden;
	}
	.fence.board {
		min-height: 100vh;
		min-height: 100dvh;
		display: flex;
		flex-direction: column;
	}
	.fence.board > :global(.board) {
		flex: 1;
	}
	/* With a banner the court view takes what is left of the screen below it. */
	.fence.court.with-banner {
		display: flex;
		flex-direction: column;
	}
	.fence.court.with-banner > :global(.court) {
		flex: 1;
		min-height: 0;
		height: auto;
	}
	.fence-banner {
		flex: none;
	}
	.message {
		min-height: 100dvh;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		padding: 16px;
		text-align: center;
		color: var(--muted);
	}
	.sync {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		padding: 3px 10px;
		border-radius: 999px;
		white-space: nowrap;
	}
	.sync.court {
		color: #fff;
		background: rgba(5, 11, 18, 0.55);
	}
	.sync.board {
		color: var(--muted);
		border: 1px solid var(--line);
	}
	.sync.off {
		color: #fff;
		background: var(--warn);
		border-color: var(--warn);
	}
	.tool {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: var(--tap);
		min-height: var(--tap);
		padding: 0;
		border-radius: 999px;
		touch-action: manipulation;
	}
	.tool.court {
		border: 0;
		color: #fff;
		background: rgba(5, 11, 18, 0.55);
	}
	.tool.board {
		border: 1px solid var(--line);
		color: var(--muted);
		background: transparent;
	}
	.court-left,
	.court-right {
		display: flex;
		gap: 8px;
	}
	.finished {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 18px 20px;
		border-radius: 18px;
		background: var(--panel);
		color: var(--ink);
		border: 1px solid var(--line);
		box-shadow: 0 20px 40px -24px rgba(11, 34, 54, 0.45);
		text-align: center;
	}
	.winner {
		margin: 0;
		font-size: clamp(22px, 4vw, 36px);
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
		line-height: 1.1;
	}
	.summary {
		margin: 0;
		color: var(--muted);
		font-feature-settings: 'tnum';
	}
	.home {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		padding: 0 22px;
		border-radius: 14px;
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
		text-decoration: none;
	}
</style>
