<!--
	Type a match's final score: one field, live feedback as it's typed (the parser, then the
	engine's verdict in words), the overtime game's winner when a level timed score needs one,
	the winner shown before saving, and the server's refusal shown in place.

	`onsubmit` saves the score. It throws to refuse: an ApiError `invalid_score` (with `reason`),
	`conflict` or `locked` is explained here, anything else through `eventMessage`. A court phone
	that gets an outcome instead of a throw (postFinal) throws the matching ApiError itself.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { ApiError } from '#lib/api.ts';
	import { engineReady, initEngine, validateFinal } from '#lib/engine.ts';
	import { eventMessage, invalidScoreText, parsedErrorText, parseTypedScore, withOvertimeWinner } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { Format, Team, Teams, TypedScore } from '#lib/types.ts';

	let {
		format,
		teams,
		title,
		onsubmit,
		oncancel
	}: {
		format: Format;
		teams: Teams;
		/** The heading; defaults to a plain "Final score". */
		title?: string;
		onsubmit: (score: TypedScore) => Promise<void>;
		oncancel: () => void;
	} = $props();

	const t = strings.events.sheet;
	const uid = $props.id();
	const TEAMS: Team[] = ['a', 'b'];
	const names = (team: Team) => strings.teamName(...teams[team]);

	let text = $state('');
	let overtimeWinner = $state<Team | null>(null);
	let busy = $state(false);
	let failure = $state<string | null>(null);
	/** The engine loaded: until then (or if it can't), the server alone judges the score. */
	let engine = $state(engineReady());

	let field = $state<HTMLInputElement | null>(null);

	onMount(() => {
		// The sheet opens on a tap: put the cursor in the field straight away.
		field?.focus();
		if (engine) return;
		initEngine().then(
			() => (engine = true),
			() => {
				// Offline or unsupported: the server still checks the score.
			}
		);
	});

	const timed = $derived('timed' in format);
	const parsed = $derived(parseTypedScore(text, format));
	/** The score to send, or why there isn't one yet. */
	const verdict = $derived.by((): { score: TypedScore; winner: Team | null | undefined } | { problem: string } | { pickOvertime: true } => {
		let score: TypedScore;
		if ('ok' in parsed) score = parsed.ok;
		else if (parsed.error === 'needs_overtime_winner') {
			if (overtimeWinner === null) return { pickOvertime: true };
			score = withOvertimeWinner(parsed.score, overtimeWinner);
		} else return { problem: parsedErrorText(parsed.error) };
		if (!engine) return { score, winner: undefined };
		const judged = validateFinal(format, score);
		if ('error' in judged) return { problem: invalidScoreText(judged.error) };
		return { score, winner: judged.winner };
	});
	const needsOvertime = $derived('error' in parsed && parsed.error === 'needs_overtime_winner');
	/** Nothing typed yet is no problem to show. */
	const showProblem = $derived('problem' in verdict && text.trim() !== '');

	function typed() {
		overtimeWinner = null;
		failure = null;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || !('score' in verdict)) return;
		busy = true;
		failure = null;
		try {
			await onsubmit(verdict.score);
		} catch (e) {
			failure = refusal(e);
		} finally {
			busy = false;
		}
	}

	function refusal(e: unknown): string {
		if (e instanceof ApiError) {
			if (e.code === 'invalid_score' && e.reason !== undefined) return invalidScoreText(e.reason);
			if (e.code === 'conflict') return t.conflict;
			if (e.code === 'locked') return t.locked;
		}
		return eventMessage(e);
	}
</script>

<form class="sheet" onsubmit={submit} aria-labelledby="{uid}-title">
	<h3 id="{uid}-title">{title ?? t.field}</h3>
	<label for="{uid}-score" class="visually-hidden">{t.field}</label>
	<input
		id="{uid}-score"
		bind:this={field}
		bind:value={text}
		oninput={typed}
		placeholder={timed ? t.placeholderTimed : t.placeholderSets}
		aria-describedby="{uid}-hint {uid}-feedback"
		inputmode="text"
		autocomplete="off"
		autocapitalize="off"
		spellcheck="false"
		maxlength="60"
	/>
	<p class="hint" id="{uid}-hint">{timed ? t.hintTimed : t.hintSets}</p>

	{#if needsOvertime}
		<div class="overtime" role="group" aria-labelledby="{uid}-ot">
			<p id="{uid}-ot">{t.overtime}</p>
			<div class="pick">
				{#each TEAMS as team (team)}
					<button type="button" class="secondary {team}" aria-pressed={overtimeWinner === team} onclick={() => (overtimeWinner = team)}>
						{t.overtimeWon(names(team))}
					</button>
				{/each}
			</div>
		</div>
	{/if}

	<div id="{uid}-feedback" aria-live="polite">
		{#if showProblem && 'problem' in verdict}
			<p class="problem">{verdict.problem}</p>
		{:else if 'score' in verdict && verdict.winner !== undefined}
			<p class="winner">{verdict.winner === null ? t.draw : t.winner(names(verdict.winner))}</p>
		{/if}
	</div>

	{#if failure}<p class="error" role="alert">{failure}</p>{/if}

	<div class="actions">
		<button class="primary" type="submit" disabled={busy || !('score' in verdict)}>{busy ? t.saving : t.save}</button>
		<button class="secondary" type="button" onclick={oncancel}>{t.cancel}</button>
	</div>
</form>

<style>
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 8px;
		width: 100%;
		padding: 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--glass);
	}
	h3 {
		margin: 0;
		font-size: 16px;
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	input {
		min-height: var(--tap);
		width: 100%;
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		font-size: 20px;
		font-feature-settings: 'tnum';
		color: var(--ink);
	}
	.hint {
		margin: 0;
		font-size: 13px;
		color: var(--muted);
	}
	.overtime p {
		margin: 0 0 6px;
		font-weight: 650;
	}
	.pick {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.pick button {
		flex: 1 1 10rem;
		min-height: var(--tap);
		border-radius: 12px;
		border: 1px solid var(--line);
		background: var(--panel);
		color: var(--ink);
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.pick button.a[aria-pressed='true'] {
		background: var(--a);
		border-color: var(--a);
		color: var(--on-team);
	}
	.pick button.b[aria-pressed='true'] {
		background: var(--b);
		border-color: var(--b);
		color: var(--on-team);
	}
	.problem {
		margin: 0;
		color: var(--warn);
	}
	.winner {
		margin: 0;
		font-weight: 700;
	}
	.error {
		margin: 0;
		border: 1px solid var(--warn);
		border-radius: 12px;
		padding: 10px;
		color: var(--warn);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.actions button {
		flex: 1 1 8rem;
		min-height: var(--tap);
		border-radius: 14px;
		font-weight: 650;
	}
	.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.primary:disabled {
		opacity: 0.6;
		cursor: default;
	}
	.secondary {
		border: 1px solid var(--line);
		background: var(--panel);
		color: var(--ink);
	}
</style>
