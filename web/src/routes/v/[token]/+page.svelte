<script lang="ts">
	// The spectator view: the same scoreboard as the fence, read-only. It only ever reads
	// /api/view/{token}; there are no taps, undo, share or offline queue here.
	import { page } from '$app/state';
	import { ApiError, getView, streamUrl } from '#lib/api.ts';
	import { latestView, loadLayout, nextLayout, saveLayout, setsSummary, type Layout } from '#lib/fence.ts';
	import { loadView, saveView } from '#lib/lastview.ts';
	import { getSlots } from '#lib/linking.ts';
	import { session } from '#lib/session.ts';
	import { matchLabel } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import { openStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { MatchView, SlotState } from '#lib/types.ts';
	import { fenceModel } from '#lib/view.ts';
	import { keepAwake } from '#lib/wakelock.ts';
	import BoardView from '#lib/components/BoardView.svelte';
	import ClaimSheet from '#lib/components/ClaimSheet.svelte';
	import CourtView from '#lib/components/CourtView.svelte';

	const f = strings.fence;
	const token = $derived(page.params.token ?? '');

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

	// "That's me" on the picked players, only when a session is known. Signed out, or on any
	// failure (401, 403, 404, offline), there are no slots and the page is unchanged.
	let slots = $state.raw<SlotState[]>([]);
	let claiming = $state<SlotState | null>(null);
	const signedIn = $derived($session.status === 'signed_in');

	async function loadSlots(t: string, live: () => boolean) {
		try {
			const list = await getSlots(t);
			if (live()) slots = list;
		} catch {
			if (live()) slots = [];
		}
	}

	$effect(() => {
		const t = token;
		slots = [];
		claiming = null;
		if (!signedIn) return;
		let live = true;
		void loadSlots(t, () => live);
		return () => {
			live = false;
		};
	});

	let listEl = $state<HTMLElement | undefined>();
	/** Slots someone can still claim: only these keep the compact bar on screen. */
	const open = $derived(slots.filter((x) => x.state === 'claimable'));

	function barAction() {
		if (open.length === 1) claiming = open[0]!;
		else {
			listEl?.scrollIntoView({ block: 'center' });
			listEl?.focus();
		}
	}

	const claimable = $derived(slots.filter((x) => x.state !== 'linked' && x.state !== 'declined'));

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

{#snippet banner()}
	{#if model?.finished}
		{@const w = model.finished.winner}
		<div class="finished">
			<p class="winner">{w ? f.wins(model[w].name) : f.draw}</p>
			{#if setsSummary(model)}<p class="summary">{setsSummary(model)}</p>{/if}
		</div>
	{/if}
{/snippet}

<main class="fence {layout}">
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
		<CourtView {model} banner={model.finished ? banner : undefined}>
			{#snippet controls()}
				<div class="court-left"></div>
				{@render sync('court')}
				<div class="court-right">{@render tools('court')}</div>
			{/snippet}
		</CourtView>
	{:else}
		<BoardView {model} banner={model.finished ? banner : undefined}>
			{#snippet meta()}
				{@render sync('board')}
				{@render tools('board')}
			{/snippet}
		</BoardView>
	{/if}
	<p class="visually-hidden" aria-live="polite">{announce}</p>
</main>

{#if claimable.length > 0}
	<section class="claims" aria-label={strings.linking.thatsMe} bind:this={listEl} tabindex="-1">
		<ul>
			{#each claimable as slot (`${slot.team}${slot.position}`)}
				<li>
					<span class="who">{slot.name}</span>
					{#if slot.state === 'claimable'}
						<button type="button" aria-label={strings.linking.thatsMeLabel(slot.name)} onclick={() => (claiming = slot)}
							>{strings.linking.thatsMe}</button
						>
					{:else if slot.state === 'pending'}
						<span class="state">{strings.linking.claimSentShort}</span>
					{:else if slot.state === 'yours'}
						<span class="state">{strings.linking.you}</span>
					{/if}
				</li>
			{/each}
		</ul>
	</section>
{/if}

{#if open.length > 0}
	<div class="bar">
		<button type="button" onclick={barAction}
			>{open.length === 1 ? strings.linking.thatsMeLabel(open[0]!.name) : strings.linking.barMany}</button
		>
	</div>
{/if}

{#if claiming}
	<ClaimSheet
		{token}
		slot={claiming}
		onclose={() => {
			claiming = null;
			void loadSlots(token, () => true);
		}}
	/>
{/if}

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
	/* In flow below the board: the full list, never over the live score. */
	.claims {
		padding: 8px 16px 24px;
		background: var(--glass);
		color: var(--ink);
	}
	.claims:focus {
		outline: none;
	}
	/* One compact row stuck to the bottom while a slot is claimable. */
	.bar {
		position: sticky;
		bottom: 0;
		padding: 4px 16px calc(4px + env(safe-area-inset-bottom));
		border-top: 1px solid var(--line);
		background: var(--panel);
	}
	.bar button {
		display: block;
		width: 100%;
		max-width: 28rem;
		margin: 0 auto;
		min-height: var(--tap);
		border: 0;
		border-radius: 12px;
		background: var(--ink);
		color: var(--glass);
		font-size: 16px;
		font-weight: 650;
	}
	.claims ul {
		list-style: none;
		margin: 0 auto;
		padding: 0;
		max-width: 28rem;
		display: grid;
		gap: 6px;
	}
	.claims li {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		min-height: var(--tap);
		padding: 4px 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.who {
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.claims button {
		min-height: var(--tap);
		padding: 0 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: transparent;
		color: var(--ink);
		font-weight: 650;
	}
	.state {
		color: var(--muted);
		font-size: 14px;
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
