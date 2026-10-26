<script lang="ts">
	// The court phone: one phone at the fence scores every match on its court. Join (a tap that
	// unlocks audio and the wake lock), then follow the court: waiting, choosing who serves, the
	// fence for the current match, and its final score with the next match. The fence is keyed
	// by the current match's scorer token, so a new match gets a new queue and an old match's
	// waiting taps never reach it: when the token moves on, those taps are cleared from the
	// phone and a note says how many were lost.
	import { page } from '$app/state';
	import { ApiError, getView, postFinal, putStart, streamUrl } from '#lib/api.ts';
	import { unlockAudio } from '#lib/audio.ts';
	import { courtMatchText, courtScreen, finalRefusal, goneReason, isCourtState, loadJoined, saveJoined } from '#lib/court.ts';
	import { advanceByCourt, courtStreamUrl, getCourt } from '#lib/events.ts';
	import { latestView } from '#lib/fence.ts';
	import { stickyClientId } from '#lib/ids.ts';
	import { takeAbandoned } from '#lib/queue.ts';
	import { safeSessionStorage, safeStorage } from '#lib/storage.ts';
	import { openSseStream, openStream } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtState, MatchView, Start, TypedScore } from '#lib/types.ts';
	import { keepAwake, type Awake } from '#lib/wakelock.ts';
	import Fence from '#lib/components/Fence.svelte';
	import KindCourtBanner from '#lib/components/events/KindCourtBanner.svelte';
	import ServePicker from '#lib/components/ServePicker.svelte';
	import TypedScoreSheet from '#lib/components/TypedScoreSheet.svelte';

	const t = strings.court;
	const token = $derived(page.params.token ?? '');

	/** The court as last heard; null until the first answer, and once the link is dead. */
	let court = $state.raw<CourtState | null>(null);
	let gone = $state<'replaced' | 'finished' | null>(null);
	let loadFailed = $state(false);
	let joined = $state(false);
	/** The current match's view, from its viewing stream. */
	let view = $state.raw<MatchView | null>(null);
	/** The match this phone chose the server for (choosing writes no event). */
	let startedHere = $state<string | null>(null);
	let starting = $state(false);
	let startFailure = $state<string | null>(null);
	let sheetOpen = $state(false);
	let advancing = $state(false);
	let advanceFailure = $state<string | null>(null);
	let fence = $state<ReturnType<typeof Fence> | undefined>();
	let awake: Awake | null = null;
	/** One client id per typed score while the sheet is open, so a retry is replayed. */
	const finalIds = stickyClientId();
	/** Taps lost with a match that left this court before they were sent. */
	let abandoned = $state<string | null>(null);
	/** The scorer token the fence last ran on (not reactive: only compared). */
	let lastScorerToken: string | null = null;

	const current = $derived(court?.current ?? null);
	const next = $derived(court?.next ?? null);
	const screen = $derived(
		gone ? 'gone' : court === null ? null : courtScreen(court, view, joined, startedHere)
	);
	const scorerToken = $derived(current?.scorer_token ?? null);
	const viewToken = $derived(joined ? (current?.spectator_token ?? null) : null);
	const locked = $derived(view?.locked === true);
	/** Nothing scored yet on a match this phone started: the server can still be changed. */
	const canChangeServer = $derived(
		screen === 'live' && view !== null && view.head_seq === 0 && startedHere === current?.id
	);

	function dead(reason: 'replaced' | 'finished') {
		gone = reason;
		court = null;
		sheetOpen = false;
	}

	// Load the court (its names, for the join screen) whenever the link changes.
	$effect(() => {
		const tk = token;
		court = null;
		gone = null;
		loadFailed = false;
		view = null;
		startedHere = null;
		joined = loadJoined(safeSessionStorage(), tk);
		let live = true;
		getCourt(tk).then(
			(s) => {
				if (live) court = s;
			},
			(e: unknown) => {
				if (!live) return;
				const why = goneReason(e);
				if (why) dead(why);
				else loadFailed = true;
			}
		);
		return () => {
			live = false;
		};
	});

	// Once joined, follow the court. This stream is what makes the phone "connected".
	$effect(() => {
		const tk = token;
		if (!joined || gone !== null) return;
		let live = true;
		// The stream retries a 410 like any failure: ask the court why it stopped.
		const probe = () =>
			getCourt(tk).then(
				(s) => {
					if (live) court = s;
				},
				(e: unknown) => {
					if (!live) return;
					const why = goneReason(e);
					if (why) dead(why);
				}
			);
		const s = openSseStream(
			courtStreamUrl(tk),
			{ type: 'court', accept: isCourtState },
			(st) => {
				if (live) {
					court = st;
					loadFailed = false;
				}
			},
			(status) => {
				if (!live) return;
				if (status === 'gone') dead('replaced');
				else if (status === 'offline') void probe();
			}
		);
		return () => {
			live = false;
			s.close();
		};
	});

	// The current match's view, for which screen to show and the typed score's revision.
	$effect(() => {
		const vt = viewToken;
		view = null;
		sheetOpen = false;
		startFailure = null;
		advanceFailure = null;
		if (vt === null) return;
		let live = true;
		const accept = (v: MatchView) => {
			if (live) view = latestView(view, v);
		};
		const s = openStream(streamUrl('view', vt), accept, () => {});
		getView(vt).then(accept, () => {
			// The stream catches up once the line is back.
		});
		return () => {
			live = false;
			s.close();
		};
	});

	// A match that left the court (advanced, moved, or its link rotated) takes its scorer token
	// with it: taps still waiting for that token can never be sent, so clear them and say so.
	$effect(() => {
		const st = scorerToken;
		const old = lastScorerToken;
		if (st === null || st === old) return;
		lastScorerToken = st;
		if (old === null) return;
		const n = takeAbandoned(safeStorage(), old);
		if (n > 0) abandoned = t.abandoned(n);
	});

	// Each opening of the result sheet is a new request; retries inside it reuse its id.
	$effect(() => {
		if (sheetOpen) finalIds.reset();
	});

	// A reload restores the join from this tab: keep the screen on again (the join tap did it
	// the first time). Released when the page goes.
	$effect(() => {
		if (joined) awake ??= keepAwake(navigator, document);
	});

	$effect(() => () => awake?.release());

	function join() {
		// The tap is the user gesture browsers want before playing sound or keeping the screen on.
		unlockAudio();
		awake ??= keepAwake(navigator, document);
		awake.retry();
		saveJoined(safeSessionStorage(), token);
		joined = true;
	}

	async function chooseServer(start: Start) {
		const st = scorerToken;
		const id = current?.id;
		if (!st || !id || starting) return;
		starting = true;
		startFailure = null;
		try {
			const out = await putStart(st, start);
			view = latestView(view, out.view);
			if (out.status === 'applied') startedHere = id;
		} catch (e) {
			// Someone scored already (the coordinator, another phone): just score on.
			if (e instanceof ApiError && e.code === 'already_started') startedHere = id;
			else startFailure = t.startFailed;
		} finally {
			starting = false;
		}
	}

	async function submitFinal(score: TypedScore) {
		const st = scorerToken;
		const v = view;
		if (!st || !v) throw new ApiError(0, 'network', strings.networkError, true);
		const out = await postFinal(st, { client_id: finalIds.for(score), base_revision: v.revision, score });
		if (out.status !== 'invalid_score') view = latestView(view, out.view);
		// The coordinator's result stands: this phone's waiting taps go too.
		if (out.status === 'locked') fence?.lockedOut(out.view);
		const refusal = finalRefusal(out);
		if (refusal) throw refusal;
		sheetOpen = false;
	}

	async function advance() {
		if (advancing) return;
		advancing = true;
		advanceFailure = null;
		try {
			court = await advanceByCourt(token);
		} catch (e) {
			const why = goneReason(e);
			if (why) dead(why);
			else advanceFailure = t.advanceFailed;
		} finally {
			advancing = false;
		}
	}

	const title = $derived(court ? `${court.court.name} · ${court.event.name}` : strings.appName);
	// Set here rather than in <svelte:head>: the scoreboard below sets its own title while it is
	// shown, and an unmounted one leaves its title behind, so this one is set again after it goes.
	$effect(() => {
		if (!(screen === 'live' || screen === 'finished')) document.title = title;
	});
</script>

{#snippet banner()}
	{#if court && current}
		<div class="banner">
			<KindCourtBanner {court} />
			<p class="where"><strong>{court.court.name}</strong> · {current.label}</p>
			{#if next}<p class="next">{t.next(courtMatchText(next))}</p>{/if}
			{#if locked}<p class="locked" role="status">{t.locked}</p>{/if}
			{#if canChangeServer}
				<button type="button" class="link" onclick={() => (startedHere = null)}>{t.changeServer}</button>
			{/if}
		</div>
	{/if}
{/snippet}

{#snippet typeFinal(tone: 'court' | 'board')}
	<!-- An icon, like share and layout beside it: a text label would crowd the meta row at 390 px. -->
	<button type="button" class="type-final {tone}" aria-label={t.typeFinal} title={t.typeFinal} onclick={() => (sheetOpen = true)}>
		<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"
			><path d="M4 17.5V20h2.5L17.8 8.7l-2.5-2.5zM19.7 6.8a1 1 0 0 0 0-1.4l-1.1-1.1a1 1 0 0 0-1.4 0l-1 1 2.5 2.5z" fill="currentColor" /></svg
		>
	</button>
{/snippet}

{#snippet nextAction()}
	{#if next}
		<button type="button" class="big" disabled={advancing} onclick={advance}>{t.nextMatch(courtMatchText(next))}</button>
	{:else}
		<p class="waiting-coord">{t.waitingForCoordinator}</p>
	{/if}
	{#if advanceFailure}<p class="error" role="alert">{advanceFailure}</p>{/if}
{/snippet}

{#if (screen === 'live' || screen === 'finished') && scorerToken}
	<Fence bind:this={fence} token={scorerToken} {banner} extraActions={typeFinal} finishedAction={nextAction} />
{:else}
	<main class="court-screen">
		{#if court}<KindCourtBanner {court} />{/if}
		{#if screen === 'gone'}
			<div class="center">
				<p class="big-text">{gone === 'finished' ? t.finished : t.replaced}</p>
			</div>
		{:else if screen === null}
			<div class="center">
				{#if loadFailed}
					<p>{t.loadFailed}</p>
					<button type="button" class="big" onclick={() => location.reload()}>{t.reload}</button>
				{:else}
					<p class="muted">{t.loading}</p>
				{/if}
			</div>
		{:else if court && screen === 'join'}
			<div class="center">
				<p class="muted">{court.event.name}</p>
				<h1>{court.court.name}</h1>
				<button type="button" class="big join" onclick={join}>{t.join(court.court.name)}</button>
				<p class="muted small">{t.joinHint}</p>
			</div>
		{:else if court && screen === 'waiting'}
			<div class="center">
				<p class="muted">{court.event.name} · {court.court.name}</p>
				<h1>{t.waiting}</h1>
				{#if next}<p>{t.upNext(courtMatchText(next))}</p>{/if}
			</div>
		{:else if court && current && screen === 'loading'}
			<div class="center">
				<p class="muted">{court.court.name} · {current.label}</p>
				<p class="muted">{strings.fence.loading}</p>
			</div>
		{:else if court && current && view && screen === 'serve'}
			<div class="serve">
				<p class="muted">{court.court.name} · {current.label}</p>
				<p class="teams">{courtMatchText(current)}</p>
				{#key current.id}
					<ServePicker teams={view.teams} start={view.start} busy={starting} failure={startFailure} onstart={chooseServer} />
				{/key}
				<button type="button" class="secondary-big" onclick={() => (sheetOpen = true)}>{t.typeFinal}</button>
				{#if next}<p class="muted">{t.next(courtMatchText(next))}</p>{/if}
			</div>
		{/if}
	</main>
{/if}

{#if sheetOpen && view && current}
	<div class="sheet-backdrop">
		<div class="sheet-wrap">
			<TypedScoreSheet
				format={view.format}
				teams={view.teams}
				title={strings.events.sheet.title(courtMatchText(current))}
				onsubmit={submitFinal}
				oncancel={() => (sheetOpen = false)}
			/>
		</div>
	</div>
{/if}

{#if abandoned}
	<div class="abandoned" role="status">
		<p>{abandoned}</p>
		<button type="button" class="dismiss" onclick={() => (abandoned = null)}>{t.dismiss}</button>
	</div>
{/if}

<style>
	.abandoned {
		position: fixed;
		left: 16px;
		right: 16px;
		bottom: 16px;
		z-index: 30;
		display: flex;
		align-items: center;
		gap: 12px;
		max-width: 28rem;
		margin: 0 auto;
		padding: 10px 12px 10px 16px;
		border: 1px solid var(--warn);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		box-shadow: 0 6px 24px rgb(0 0 0 / 0.25);
	}
	.abandoned p {
		flex: 1;
		margin: 0;
		font-size: 15px;
	}
	.dismiss {
		flex: none;
		min-height: var(--tap);
		padding: 0 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: transparent;
		color: var(--ink);
		font: inherit;
		font-weight: 650;
	}
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
		max-width: 28rem;
		margin: 0 auto;
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
	.small {
		font-size: 14px;
	}
	.big-text {
		font-size: 22px;
		font-weight: 700;
	}
	.big {
		width: 100%;
		min-height: 64px;
		padding: 8px 20px;
		border: 0;
		border-radius: 16px;
		background: var(--ink);
		color: var(--glass);
		font-size: 20px;
		font-weight: 750;
		overflow-wrap: anywhere;
		touch-action: manipulation;
	}
	.big:disabled {
		opacity: 0.6;
	}
	.join {
		min-height: 88px;
		font-size: 24px;
		margin: 12px 0;
	}
	.serve {
		display: flex;
		flex-direction: column;
		gap: 12px;
		max-width: 28rem;
		margin: 0 auto;
	}
	.serve p {
		margin: 0;
	}
	.teams {
		font-size: 18px;
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.secondary-big {
		min-height: var(--tap);
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		font-weight: 650;
	}
	.banner {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
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
	.where {
		flex: 1 1 auto;
	}
	.next {
		color: var(--muted);
		flex: 1 1 100%;
	}
	.locked {
		flex: 1 1 100%;
		font-weight: 700;
		color: var(--warn);
	}
	.link {
		min-height: var(--tap);
		padding: 0;
		border: 0;
		background: none;
		color: var(--a);
		font-weight: 650;
		text-decoration: underline;
	}
	.type-final {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-width: var(--tap);
		min-height: var(--tap);
		padding: 0;
		border-radius: 999px;
		touch-action: manipulation;
	}
	.type-final.court {
		border: 0;
		color: #fff;
		background: rgba(5, 11, 18, 0.55);
	}
	.type-final.board {
		border: 1px solid var(--line);
		color: var(--muted);
		background: transparent;
	}
	.waiting-coord {
		margin: 6px 0 0;
		font-weight: 650;
		color: var(--muted);
	}
	.error {
		margin: 6px 0 0;
		color: var(--warn);
	}
	.sheet-backdrop {
		position: fixed;
		inset: 0;
		z-index: 5;
		display: flex;
		align-items: flex-end;
		justify-content: center;
		background: rgba(5, 11, 18, 0.45);
		padding: 16px;
	}
	.sheet-wrap {
		width: 100%;
		max-width: 28rem;
		max-height: 100%;
		overflow-y: auto;
	}
	.sheet-wrap :global(.sheet) {
		background: var(--panel);
	}
	@media (min-width: 700px) {
		.center,
		.serve {
			max-width: 32rem;
		}
	}
</style>
