<script lang="ts">
	// A swap or a time change waiting for my answer. Shared by the board page and the calendar:
	// both map their own request shape to these props.
	import { browserTimeZone, dayLabel, timeRange } from '#lib/dates.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.gameSheet;

	interface Props {
		kind: 'swap' | 'time';
		/** Who asks. */
		from: string;
		/** [start, end] of my game now. */
		current: [number, number];
		/** [start, end] of the game I would trade into, or the proposed time. */
		proposed: [number, number];
		busy?: boolean;
		onanswer: (accept: boolean) => void;
	}

	let { kind, from, current, proposed, busy = false, onanswer }: Props = $props();

	const zone = browserTimeZone();
	const when = (r: [number, number]) => `${dayLabel(r[0], zone)} · ${timeRange(r[0], r[1], zone)}`;
	const labels = $derived(kind === 'swap' ? [t.yourGame, t.theirGame] : [t.nowAt, t.proposedAt]);
</script>

<section class="request" aria-label={kind === 'swap' ? t.requestSwap(from) : t.requestTime(from)}>
	<h3>{kind === 'swap' ? t.requestSwap(from) : t.requestTime(from)}</h3>
	<dl>
		<div>
			<dt>{labels[0]}</dt>
			<dd>{when(current)}</dd>
		</div>
		<div>
			<dt>{labels[1]}</dt>
			<dd>{when(proposed)}</dd>
		</div>
	</dl>
	<div class="actions">
		<button type="button" class="primary" disabled={busy} onclick={() => onanswer(true)}>{t.accept}</button>
		<button type="button" class="secondary" disabled={busy} onclick={() => onanswer(false)}>{t.decline}</button>
	</div>
</section>

<style>
	.request {
		padding: 12px 14px;
		border: 2px solid var(--ball);
		border-radius: 14px;
		background: var(--panel);
	}
	h3 {
		margin: 0 0 6px;
		font-size: 16px;
		overflow-wrap: anywhere;
	}
	dl {
		display: grid;
		gap: 4px;
		margin: 0 0 10px;
	}
	dl div {
		display: flex;
		flex-wrap: wrap;
		gap: 0 8px;
	}
	dt {
		min-width: 5.5rem;
		font-size: 13px;
		color: var(--muted);
	}
	dd {
		margin: 0;
		font-weight: 600;
	}
</style>
