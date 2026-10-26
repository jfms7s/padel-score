<!--
	Owned by phase 8 (the non-stop). The big countdown, the round it belongs to and (for the
	coordinator) the one button that fits. The count-in shows its digit while it runs. No sound
	here: the horn is the court phone's.
-->
<script lang="ts">
	import { ApiError } from '#lib/api.ts';
	import {
		clockButtons,
		clockText,
		countInMs,
		isLow,
		postClock,
		remainingMs,
		serverClock,
		type Clock,
		type ClockButton
	} from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.nonStop;

	let {
		eventId,
		clock,
		rounds,
		waves,
		controls
	}: { eventId: string; clock: Clock | null; rounds: number; waves: number; controls: boolean } = $props();

	let now = $state(serverClock().now());
	let busy = $state(false);
	let error = $state('');
	let ending = $state(false);

	// About four frames a second is enough for a mm:ss readout.
	$effect(() => {
		now = serverClock().now();
		const id = setInterval(() => (now = serverClock().now()), 250);
		return () => clearInterval(id);
	});

	const left = $derived(clock === null ? 0 : remainingMs(clock, now));
	const countIn = $derived(clock === null ? 0 : countInMs(clock, now));
	const buttons = $derived(controls ? clockButtons(clock) : []);
	const stateText = $derived(
		clock === null
			? ''
			: clock.state === 'ready'
				? t.stateReady
				: clock.state === 'paused'
					? t.paused
					: clock.state === 'done'
						? t.stateDone
						: countIn > 0
							? t.countIn
							: t.stateRunning
	);

	async function run(action: ClockButton) {
		if (clock === null || busy) return;
		busy = true;
		error = '';
		try {
			await postClock(eventId, action, clock.slot);
			ending = false;
		} catch (e) {
			const errors: Record<string, string> = t.clockErrors;
			error = e instanceof ApiError ? (errors[e.code] ?? t.clockFailed) : t.clockFailed;
		} finally {
			busy = false;
		}
	}
</script>

{#if clock !== null}
	<section class="clock" aria-label={t.roundOf(clock.round, rounds)}>
		<p class="round">
			{t.roundOf(clock.round, rounds)}{#if waves > 1}, {t.waveOf(clock.wave, waves).toLowerCase()}{/if}
		</p>
		{#if countIn > 0}
			<p class="state">{stateText}</p>
			<p class="digit" role="timer" aria-live="off">{Math.ceil(countIn / 1000)}</p>
		{:else}
			<p class="time" class:low={clock.state === 'running' && isLow(left)} role="timer" aria-live="off">
				{clockText(left)}
			</p>
			<p class="state">{stateText}</p>
		{/if}

		{#if buttons.length > 0}
			<div class="actions">
				{#each buttons as b (b)}
					{#if b === 'end'}
						{#if !ending}
							<button class="secondary" type="button" disabled={busy} onclick={() => (ending = true)}>{t.endNow}</button>
						{/if}
					{:else}
						<button class="primary" type="button" disabled={busy} onclick={() => run(b)}>
							{b === 'start'
								? t.start(t.slotText(clock.round, clock.wave, waves))
								: b === 'pause'
									? t.pause
									: t.resume}
						</button>
					{/if}
				{/each}
			</div>
			{#if ending}
				<div class="confirm" role="group" aria-label={t.endNow}>
					<p>{t.endNowConfirm}</p>
					<div class="actions">
						<button class="danger" type="button" disabled={busy} onclick={() => run('end')}>{t.endNow}</button>
						<button class="secondary" type="button" onclick={() => (ending = false)}>{strings.events.cancel}</button>
					</div>
				</div>
			{/if}
			{#if clock.state === 'ready'}
				<p class="hint">{t.practiceNote}</p>
			{/if}
		{/if}
		{#if error}<p class="error" role="alert">{error}</p>{/if}
	</section>
{/if}

<style>
	.clock {
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 16px;
		background: var(--glass);
		text-align: center;
		min-width: 0;
	}
	p {
		margin: 0;
	}
	.round {
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.time,
	.digit {
		font-size: clamp(56px, 22vw, 96px);
		font-weight: 800;
		line-height: 1.1;
		font-feature-settings: 'tnum';
	}
	.time.low {
		color: var(--warn);
	}
	.state {
		color: var(--muted);
		font-size: 14px;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		justify-content: center;
		margin-top: 12px;
	}
	.actions button {
		flex: 1 1 8rem;
		min-height: var(--tap);
	}
	.confirm {
		margin-top: 12px;
	}
	.confirm p {
		font-size: 14px;
		margin-bottom: 8px;
	}
	.hint {
		margin-top: 8px;
	}
	.error {
		margin-top: 12px;
	}
</style>
