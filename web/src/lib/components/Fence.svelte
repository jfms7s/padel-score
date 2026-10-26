<script lang="ts">
	// The fence screen for the scorer: the live score, big enough to read from 10 m, with the
	// whole team half (sideways) or the point buttons (upright) as the scoring remote.
	//
	// Used by a match's scoring link (/s/{token}) and by the court phone, which passes a `banner`
	// (court, match, what's next), `extraActions` (typing the final score) and `finishedAction`
	// (the next match). A new `token`
	// starts over: a fresh load, stream and queue, keyed by token, so an old match's taps never
	// go to the new one.
	import type { Snippet } from 'svelte';
	import { ApiError, getScore, postCommands, replace, streamUrl } from '#lib/api.ts';
	import { unlockAudio } from '#lib/audio.ts';
	import { engineReady, initEngine, replayLocal } from '#lib/engine.ts';
	import { holdUpdates } from '#lib/holds.svelte.ts';
	import { newClientId } from '#lib/ids.ts';
	import {
		latestView,
		loadLayout,
		nextLayout,
		saveLayout,
		setsSummary,
		shareLinks,
		spectatorFromHash,
		type Layout
	} from '#lib/fence.ts';
	import { createQueue, localView, type Dropped, type Problem, type Queue, type QueueState } from '#lib/queue.ts';
	import { loadView, saveView } from '#lib/lastview.ts';
	import { matchLabel, recentSpectator } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import { openStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { ErrorKind, MatchView, Team } from '#lib/types.ts';
	import { fenceModel } from '#lib/view.ts';
	import { keepAwake, type Awake } from '#lib/wakelock.ts';
	import BoardView from '#lib/components/BoardView.svelte';
	import ConflictSheet from '#lib/components/ConflictSheet.svelte';
	import CourtView from '#lib/components/CourtView.svelte';
	import ShareSheet from '#lib/components/ShareSheet.svelte';
	import UndoButton from '#lib/components/UndoButton.svelte';

	type Tone = 'court' | 'board';

	let {
		token,
		banner,
		extraActions,
		finishedAction
	}: {
		token: string;
		/** Shown above the score. */
		banner?: Snippet;
		/** More buttons beside share and layout, styled for the layout's tone. */
		extraActions?: Snippet<[Tone]>;
		/** In place of the finished card's "New match" link (the court phone's next match). */
		finishedAction?: Snippet;
	} = $props();

	const f = strings.fence;

	let view = $state.raw<MatchView | null>(null);
	let loadError = $state<string | null>(null);
	let stream = $state<StreamStatus | null>(null);
	const idle: QueueState = {
		pending: [],
		queued: [],
		sending: 0,
		base: null,
		offline: false,
		conflict: null,
		resolving: false,
		problem: null,
		unsaved: false
	};
	let sending = $state.raw<QueueState>(idle);
	let engine = $state(engineReady());
	let notice = $state<string | null>(null);
	let layoutPref = $state<Layout>('auto');
	let landscape = $state(false);
	let shareOpen = $state(false);
	let spectator = $state<string | null>(null);
	let origin = $state('');

	let sender: Queue | null = null;
	let awake: Awake | null = null;
	let noticeTimer: ReturnType<typeof setTimeout> | null = null;

	// Optimistic taps: the points still on their way, and the ones queued offline, laid over the
	// server's view until a newer view comes in (which already holds them, or tells otherwise).
	const shown = $derived(view && engine ? localView(view, sending, replayLocal) : view);
	const model = $derived(shown ? fenceModel(shown) : null);
	const layout = $derived(layoutPref === 'auto' ? (landscape ? 'court' : 'board') : layoutPref);
	const offline = $derived(sending.offline || stream === 'offline');
	const pending = $derived(sending.pending.length > 0 || sending.queued.length > 0);
	const syncLabel = $derived(offline ? f.offline : stream === null ? f.connecting : f.synced);
	const links = $derived(shareLinks(origin, token, spectator));
	// The coordinator typed the result: this link can't change it any more.
	const locked = $derived(view?.locked === true);
	// A pending tap can always be taken back (locally, if it hasn't gone yet).
	const canUndo = $derived(
		!locked &&
			((!!shown && shown.events.length > 0) ||
				sending.pending.some((p) => p.intent.kind === 'point') ||
				sending.queued.some((p) => p.kind === 'point'))
	);
	const problemText = $derived(sending.problem ? problemLine(sending.problem) : null);

	function accept(incoming: MatchView) {
		view = latestView(view, incoming);
		saveView(safeStorage(), token, view);
		sender?.viewArrived();
	}

	function say(message: string) {
		notice = message;
		if (noticeTimer !== null) clearTimeout(noticeTimer);
		noticeTimer = setTimeout(() => (notice = null), 3500);
	}

	function rejectedLine(e: ErrorKind) {
		return typeof e === 'string' ? f.rejected[e] : f.rejected.invalid_rules;
	}

	function problemLine(p: Problem): string {
		return strings.queue.failed(p.kind === 'rejected' ? rejectedLine(p.error) : p.message);
	}

	function dropped(d: Dropped) {
		switch (d.reason.kind) {
			case 'rejected':
				say(rejectedLine(d.reason.error));
				break;
			case 'conflict':
				say(f.undoConflict);
				break;
			case 'nothing':
				say(f.nothingToUndo);
				break;
			case 'full':
				say(strings.queue.full);
				break;
			case 'failed':
				say(f.failed(d.reason.message));
				break;
			case 'locked':
				say(f.lockedDropped(d.reason.count));
				break;
		}
	}

	/**
	 * A send made outside the fence (a typed final score) was answered 409 locked: take the
	 * server's view and drop this phone's waiting taps, as a locked tap does.
	 */
	export function lockedOut(incoming: MatchView) {
		if (sender) sender.locked(incoming);
		else accept(incoming);
	}

	$effect(() => {
		const t = token;
		origin = location.origin;
		spectator = spectatorFromHash(location.hash) ?? recentSpectator(safeStorage(), t);
		view = null;
		loadError = null;
		stream = null;

		let live = true;
		const s = openStream(
			streamUrl('score', t),
			(v) => live && accept(v),
			(st) => {
				if (!live) return;
				stream = st;
				if (st === 'gone') loadError = f.gone;
				// The stream back means the line is: send what was queued.
				else sender?.connection(st);
			}
		);
		getScore(t).then(
			(v) => live && accept(v),
			(e: unknown) => {
				if (!live) return;
				// A retryable failure is just offline: the stream catches up once it is back.
				if (e instanceof ApiError && e.retryable) {
					// Cold open without a line: the last view this device saw, so scoring goes on
					// (taps queue against it; if the server moved meanwhile, the sheet asks).
					const stored = loadView(safeStorage(), t);
					if (stored) {
						view = latestView(view, stored);
						stream ??= 'offline';
						sender?.viewArrived();
					}
					return;
				}
				loadError = e instanceof ApiError && e.code === 'not_found' ? f.notFound : f.loadFailed;
				s.close();
			}
		);
		const q = createQueue(t, safeStorage(), { post: postCommands, replace }, replayLocal, {
			view: () => view,
			newClientId: () => newClientId(),
			setTimeout: (fn, ms) => setTimeout(fn, ms),
			clearTimeout: (id) => clearTimeout(id),
			onView: (v) => live && accept(v),
			onDropped: (d) => live && dropped(d),
			onState: (st) => {
				if (live) sending = st;
			}
		});
		sender = q;
		sending = q.state();
		if (q.discardedOnLoad) say(strings.queue.lost);
		const online = () => q.reconnect();
		const offline = () => q.connection('offline');
		addEventListener('online', online);
		addEventListener('offline', offline);
		const lock = keepAwake(navigator, document);
		awake = lock;
		return () => {
			live = false;
			s.close();
			removeEventListener('online', online);
			removeEventListener('offline', offline);
			q.stop();
			sender = null;
			lock.release();
			awake = null;
			sending = idle;
		};
	});

	// A new deploy waits until every tap here is on the server.
	$effect(() => {
		if (sending.unsaved) return holdUpdates();
	});

	$effect(() => {
		initEngine().then(
			() => {
				engine = true;
				// A conflict sheet opened before the engine loaded has no preview of this phone's
				// score: look again, so "Keep this phone's score" works.
				if (sender) sending = sender.state();
				sender?.viewArrived();
			},
			() => {
				// No local engine: taps still score, without the instant preview.
			}
		);
		layoutPref = loadLayout(safeStorage());
		const mq = matchMedia('(orientation: landscape)');
		landscape = mq.matches;
		const onChange = () => (landscape = mq.matches);
		mq.addEventListener('change', onChange);
		return () => {
			mq.removeEventListener('change', onChange);
			if (noticeTimer !== null) clearTimeout(noticeTimer);
		};
	});

	/** A tap is a user gesture: the moment to unlock audio and ask again for a refused wake lock. */
	function gesture() {
		unlockAudio();
		awake?.retry();
	}

	function point(team: Team) {
		gesture();
		if (model?.finished || locked) return;
		// A second tap within TAP_DEBOUNCE_MS is ignored by the queue as a double tap.
		sender?.point(team);
	}

	function undoLast() {
		gesture();
		if (locked) return;
		sender?.undo();
	}

	function keepServer() {
		sender?.keepServer();
	}

	function keepMine() {
		void sender?.keepMine();
	}

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

<svelte:window
	onbeforeunload={(e) => {
		// Taps not yet on the server are kept on this device and sent after a reload, but warn
		// before the page goes with any still waiting.
		if (sending.unsaved) {
			e.preventDefault();
			e.returnValue = '';
		}
	}}
/>

<svelte:head>
	<title>{title}</title>
</svelte:head>

{#snippet sync(tone: Tone)}
	<span class="sync {tone}" class:off={offline} role="status">
		{#if pending}<span class="dot" aria-hidden="true"></span><span class="visually-hidden">{f.sending}</span>{/if}
		{syncLabel}
	</span>
{/snippet}

{#snippet tools(tone: Tone)}
	{@render extraActions?.(tone)}
	<button type="button" class="tool {tone}" aria-label={f.share} title={f.share} onclick={() => (shareOpen = true)}>
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"
			><path
				d="M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h2v2h-2zM18 18h2v2h-2zM14 18h2v2h-2zM18 14h2v2h-2z"
				fill="currentColor"
			/></svg
		>
	</button>
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
			{#if finishedAction}{@render finishedAction()}{:else}<a class="new" href="/new">{f.newMatch}</a>{/if}
		</div>
	{/if}
{/snippet}

<main class="fence {layout}" class:with-banner={!!banner}>
	{#if banner}<div class="fence-banner">{@render banner()}</div>{/if}
	{#if loadError}
		<div class="message">
			<p>{loadError}</p>
			<a class="new" href="/">{f.home}</a>
		</div>
	{:else if !model}
		<div class="message">
			<p>{f.loading}</p>
			<!-- Opened from the cached shell with no line: the score can't load until it is back. -->
			{#if offline}<p class="sync board off" role="status">{f.offline}</p>{/if}
		</div>
	{:else if layout === 'court'}
		<CourtView {model} onpoint={point} banner={model.finished ? finishedBanner : undefined}>
			{#snippet controls()}
				<div class="court-left"><UndoButton onundo={undoLast} disabled={!canUndo} tone="court" /></div>
				{@render sync('court')}
				<div class="court-right">{@render tools('court')}</div>
			{/snippet}
		</CourtView>
	{:else}
		<BoardView {model} onpoint={point} banner={model.finished ? finishedBanner : undefined}>
			{#snippet meta()}
				{@render sync('board')}
				{@render tools('board')}
			{/snippet}
			{#snippet undo()}
				<UndoButton onundo={undoLast} disabled={!canUndo} tone="board" />
			{/snippet}
		</BoardView>
	{/if}

	{#if notice}<p class="notice" role="alert">{notice}</p>{/if}
	{#if problemText}
		<div class="problem" role="alert">
			<p>{problemText}</p>
			<button type="button" onclick={() => sender?.discard()}>{strings.queue.discard}</button>
		</div>
	{/if}
	<p class="visually-hidden" aria-live="polite">{announce}</p>
</main>

<ShareSheet bind:open={shareOpen} scoring={links.scoring} viewing={links.viewing} />
<ConflictSheet
	conflict={sending.conflict}
	resolving={sending.resolving}
	onkeepserver={keepServer}
	onkeepmine={keepMine}
/>

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
	.dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--ball);
		box-shadow: 0 0 0 1px rgba(5, 11, 18, 0.4);
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
	.court-left {
		justify-content: flex-end;
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
	.new {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		padding: 0 22px;
		margin-top: 6px;
		border-radius: 14px;
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
		text-decoration: none;
	}
	.problem {
		position: fixed;
		left: 16px;
		right: 16px;
		bottom: 16px;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 10px;
		margin: 0 auto;
		max-width: 40rem;
		padding: 12px 14px;
		border-radius: 14px;
		background: var(--warn);
		color: #fff;
		z-index: 2;
	}
	.problem p {
		flex: 1 1 14rem;
		margin: 0;
	}
	.problem button {
		min-height: var(--tap);
		padding: 0 16px;
		border-radius: 12px;
		border: 0;
		background: #fff;
		color: var(--ink);
		font-weight: 650;
	}
	.notice {
		position: fixed;
		left: 50%;
		top: 16px;
		transform: translateX(-50%);
		max-width: calc(100vw - 32px);
		margin: 0;
		padding: 8px 14px;
		border-radius: 12px;
		background: var(--ink);
		color: var(--glass);
		font-size: 14px;
		z-index: 2;
	}
</style>
