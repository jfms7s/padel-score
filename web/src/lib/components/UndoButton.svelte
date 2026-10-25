<script lang="ts">
	// Undo sits behind a 600 ms press-and-hold so a stray touch can't take a point back. The
	// fill shows the hold building; with reduced motion it is a static bar instead.
	import { strings } from '#lib/strings.ts';

	const HOLD_MS = 600;

	let {
		onundo,
		disabled = false,
		tone = 'board'
	}: { onundo: () => void; disabled?: boolean; tone?: 'board' | 'court' } = $props();

	let holding = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;
	/** When a key last went down or up: the click a key makes is not an assistive-tech press. */
	let lastKey = 0;

	function start() {
		if (disabled || holding) return;
		holding = true;
		timer = setTimeout(() => {
			timer = null;
			holding = false;
			onundo();
		}, HOLD_MS);
	}

	function cancel() {
		if (timer !== null) clearTimeout(timer);
		timer = null;
		holding = false;
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key !== ' ' && e.key !== 'Enter') return;
		e.preventDefault();
		lastKey = Date.now();
		if (!e.repeat) start();
	}

	function onkeyup(e: KeyboardEvent) {
		if (e.key !== ' ' && e.key !== 'Enter') return;
		lastKey = Date.now();
		cancel();
	}

	// A screen reader's activation is a bare click with no pointer or key down: it can't hold,
	// so it undoes at once.
	function onclick(e: MouseEvent) {
		if (disabled || e.detail !== 0 || Date.now() - lastKey < 1000) return;
		onundo();
	}

	$effect(() => cancel);
</script>

<button
	type="button"
	class="undo {tone}"
	class:holding
	{disabled}
	aria-label={strings.fence.undoLabel}
	title={strings.fence.undoHint}
	onpointerdown={(e) => {
		if (e.button === 0) start();
	}}
	onpointerup={cancel}
	onpointerleave={cancel}
	onpointercancel={cancel}
	oncontextmenu={(e) => e.preventDefault()}
	{onkeydown}
	{onkeyup}
	onblur={cancel}
	{onclick}
>
	<span class="fill" aria-hidden="true"></span>
	<span class="label">{strings.fence.undo}</span>
</button>

<style>
	.undo {
		position: relative;
		overflow: hidden;
		min-height: var(--tap);
		min-width: 72px;
		padding: 0 18px;
		border-radius: 14px;
		font-weight: 650;
		touch-action: manipulation;
		user-select: none;
		-webkit-user-select: none;
		-webkit-touch-callout: none;
	}
	.undo.board {
		background: transparent;
		color: var(--muted);
		border: 1px solid var(--line);
	}
	.undo.court {
		background: rgba(5, 11, 18, 0.55);
		color: #fff;
		border: 0;
		border-radius: 999px;
		font-size: 14px;
	}
	.undo:disabled {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.fill {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 100%;
		width: 100%;
		background: var(--ball);
		opacity: 0.55;
		transform: scaleX(0);
		transform-origin: left;
	}
	.holding .fill {
		transform: scaleX(1);
		transition: transform 600ms linear;
	}
	.label {
		position: relative;
	}
	@media (prefers-reduced-motion: reduce) {
		/* No sweep: a full, static bar along the bottom says "keep holding". */
		.fill {
			height: 4px;
			opacity: 1;
		}
		.holding .fill {
			transition: none;
		}
	}
</style>
