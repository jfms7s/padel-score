<!--
	Owned by phase 8 (the non-stop). What the court page shows of the clock: the round and court,
	the countdown (red in the last minute), the count-in digit with its beeps, "Paused", "Waiting
	for the clock" and, once the slot is done, "Time's up". The court phone also sounds the horn
	(`viewer` pages don't). Overtime is not shown here: the fence's rules line already says it.
-->
<script module lang="ts">
	// Module-level, so a sound already made isn't made again when the court page swaps between
	// the fence and the other screens and this banner mounts afresh.
	const played = { beeps: new Set<string>(), horns: new Set<string>() };
</script>

<script lang="ts">
	import { unlockAudio } from '#lib/audio.ts';
	import {
		clockText,
		countInMs,
		isLow,
		msToEnd,
		nonStopCourtData,
		playBeep,
		playHorn,
		remainingMs,
		serverClock,
		soundsDue
	} from '#lib/nonstop.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtState } from '#lib/types.ts';

	const t = strings.nonStop;

	let { court, viewer = false }: { court: CourtState; viewer?: boolean } = $props();

	const data = $derived(nonStopCourtData(court.kind_data));
	const clock = $derived(data?.clock ?? null);

	let now = $state(serverClock().now());

	// About four frames a second is enough for a mm:ss readout. Syncing is idempotent and the
	// shared clock keeps running when this banner goes.
	$effect(() => {
		void serverClock().sync();
		now = serverClock().now();
		const id = setInterval(() => (now = serverClock().now()), 250);
		return () => clearInterval(id);
	});

	const left = $derived(clock === null ? 0 : remainingMs(clock, now));
	const countIn = $derived(clock === null ? 0 : countInMs(clock, now));
	/** The first go hasn't happened (ready, or paused in the count-in): practice points still show. */
	const beforeGo = $derived(clock !== null && (clock.state === 'ready' || (clock.state === 'paused' && !clock.went)));

	// The last local time-to-zero of the slot being counted, for a slot that turns done early.
	let lastSlot = -1;
	let lastToEnd = 0;

	$effect(() => {
		if (viewer || clock === null) return;
		const ctx = unlockAudio();
		const synced = serverClock().synced();
		if (clock.slot !== lastSlot) {
			lastSlot = clock.slot;
			lastToEnd = 0;
		}
		const toEnd = clock.state === 'done' ? lastToEnd : msToEnd(clock, now);
		const due = soundsDue(clock, now, toEnd, synced, played, court.event.id);
		for (const kind of due.beeps) playBeep(ctx, kind === 'go');
		if (due.horn) playHorn(ctx);
		// Only a synced time to zero counts for a slot that turns done early.
		if (clock.state === 'running' && synced) lastToEnd = toEnd;
	});
</script>

{#if data !== null && clock !== null}
	<div class="clock" class:low={clock.state === 'running' && countIn === 0 && isLow(left)}>
		<p class="line">{t.courtLine(court.court.name, clock.round, data.rounds, clock.wave, data.waves)}</p>
		{#if clock.state === 'done'}
			<p class="status">{t.timeUp}</p>
		{:else if countIn > 0}
			<p class="status">{t.countIn}</p>
			<p class="big" role="timer" aria-live="off">{Math.ceil(countIn / 1000)}</p>
		{:else if clock.state === 'ready'}
			<p class="status">{t.waitingForClock}</p>
		{:else}
			<p class="big time" role="timer" aria-live="off">{clockText(left)}</p>
			{#if clock.state === 'paused'}
				<p class="status">{beforeGo ? t.pausedBeforeGo : t.paused}</p>
			{/if}
		{/if}
		{#if beforeGo}<p class="note">{t.practiceNote}</p>{/if}
	</div>
{/if}

<style>
	.clock {
		text-align: center;
		min-width: 0;
		padding: 8px 12px;
	}
	p {
		margin: 0;
	}
	.line {
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.big {
		font-size: clamp(40px, 14vw, 64px);
		font-weight: 800;
		line-height: 1.1;
		font-feature-settings: 'tnum';
	}
	.low .time {
		color: var(--warn);
	}
	.status {
		font-weight: 600;
	}
	.note {
		color: var(--muted);
		font-size: 14px;
	}
	/* A landscape phone has little height: one compact row instead of a stack. */
	@media (orientation: landscape) and (max-height: 500px) {
		.clock {
			display: flex;
			align-items: center;
			justify-content: center;
			flex-wrap: nowrap;
			gap: 4px 14px;
			max-height: 64px;
			overflow: hidden;
			padding: 4px 12px;
		}
		.big {
			font-size: clamp(28px, 9vh, 44px);
			line-height: 1;
		}
		.line,
		.status,
		.note {
			font-size: 14px;
			line-height: 1.2;
		}
	}
</style>
