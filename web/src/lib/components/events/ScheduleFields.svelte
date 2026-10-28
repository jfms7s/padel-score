<!--
	The schedule of an event: date and time, place, and who can sign up (off, link only, public)
	with the minimum, the maximum and the deadline. The parent holds the draft (see
	signups.ts `scheduleInput`) and sends it: this form on the new-event page and in the desk's
	Sign-ups tab. Times are read in the device's zone.
-->
<script lang="ts">
	import { browserTimeZone } from '#lib/dates.ts';
	import { defaultDeadlineInputs, maxChoices, minChoices, type ScheduleDraft } from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventKind, SignupMode } from '#lib/types.ts';

	let {
		draft = $bindable(),
		kind,
		legs,
		prefix
	}: {
		draft: ScheduleDraft;
		kind: EventKind;
		/** A non-stop's legs, which set how many teams fit. */
		legs: number;
		/** Keeps the field ids apart when two forms are on one page. */
		prefix: string;
	} = $props();

	const t = strings.signups;
	const MODES: SignupMode[] = ['off', 'private', 'public'];
	const MAX_PLACE_LENGTH = 80;
	const zone = browserTimeZone();

	const maxes = $derived(maxChoices(kind, legs));
	const mins = $derived(draft.max === null ? [] : minChoices(kind, draft.max));
	const on = $derived(draft.signups !== 'off');

	/** The default deadline follows the start until the organiser picks one. */
	function startChanged() {
		if (draft.deadlineTouched) return;
		const d = defaultDeadlineInputs(draft, undefined, Math.floor(Date.now() / 1000));
		draft.deadlineDate = d.date;
		draft.deadlineTime = d.time;
	}

	function pickMode(mode: SignupMode) {
		draft.signups = mode;
		if (mode !== 'off' && draft.max === null) pickMax(maxes.includes(8) ? 8 : (maxes[maxes.length - 1] ?? null));
		startChanged();
	}

	function pickMax(max: number | null) {
		draft.max = max;
		if (max === null) return;
		if (draft.min > max) draft.min = max;
		const floor = minChoices(kind, max)[0];
		if (floor !== undefined && draft.min < floor) draft.min = floor;
	}

	// A maximum that stops fitting (another kind, fewer legs) moves to the largest that does.
	$effect(() => {
		if (draft.max !== null && !maxes.includes(draft.max)) pickMax(maxes[maxes.length - 1] ?? null);
	});

	function touchDeadline() {
		draft.deadlineTouched = true;
	}
</script>

<fieldset class="schedule">
	<legend>{t.schedule}</legend>
	<div class="pair">
		<label>
			{t.date}
			<input type="date" bind:value={draft.date} onchange={startChanged} />
		</label>
		<label>
			{t.time}
			<input type="time" bind:value={draft.time} onchange={startChanged} />
		</label>
	</div>
	<p class="hint">{t.timeZoneNote(zone)}</p>
	<label>
		{t.place}
		<input bind:value={draft.place} placeholder={t.placePlaceholder} autocomplete="off" maxlength={MAX_PLACE_LENGTH} />
	</label>

	<div role="group" aria-labelledby="{prefix}-mode" aria-describedby="{prefix}-mode-hint">
		<span class="label" id="{prefix}-mode">{t.signupsLabel}</span>
		<div class="seg">
			{#each MODES as mode (mode)}
				<button type="button" aria-pressed={draft.signups === mode} onclick={() => pickMode(mode)}>{t.modes[mode]}</button>
			{/each}
		</div>
		<p class="hint" id="{prefix}-mode-hint">{t.modeHints[draft.signups]}</p>
	</div>

	{#if on}
		<div class="pair">
			<label>
				{t.maxTeams}
				<select value={draft.max} onchange={(e) => pickMax(Number(e.currentTarget.value))}>
					{#each maxes as n (n)}<option value={n} selected={draft.max === n}>{n}</option>{/each}
				</select>
			</label>
			<label>
				{t.minTeams}
				<select bind:value={draft.min}>
					{#each mins as n (n)}<option value={n}>{n}</option>{/each}
				</select>
			</label>
		</div>
		<div role="group" aria-labelledby="{prefix}-deadline" aria-describedby="{prefix}-deadline-hint">
			<span class="label" id="{prefix}-deadline">{t.deadline}</span>
			<div class="pair">
				<input type="date" aria-label="{t.deadline}: {t.date}" bind:value={draft.deadlineDate} oninput={touchDeadline} />
				<input type="time" aria-label="{t.deadline}: {t.time}" bind:value={draft.deadlineTime} oninput={touchDeadline} />
			</div>
			<p class="hint" id="{prefix}-deadline-hint">{t.deadlineHint}</p>
		</div>
	{/if}
</fieldset>

<style>
	fieldset {
		display: flex;
		flex-direction: column;
		gap: 14px;
		min-width: 0;
		margin: 0;
		padding: 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
	}
	legend {
		padding: 0 6px;
		font-size: 13px;
		font-weight: 650;
		color: var(--muted);
	}
	.pair {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
	}
	.hint {
		margin: 0;
	}
	.label {
		display: block;
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 6px;
	}
	select {
		min-height: var(--tap);
		width: 100%;
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		font-size: 16px;
		color: var(--ink);
	}
	.seg {
		display: flex;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 12px;
		padding: 3px;
	}
	.seg button {
		flex: 1;
		min-width: 0;
		font-size: 14px;
		border: 0;
		background: transparent;
		color: var(--ink);
		padding: 8px 4px;
		border-radius: 9px;
		overflow-wrap: anywhere;
	}
	.seg button[aria-pressed='true'] {
		background: var(--ink);
		color: var(--glass);
		font-weight: 650;
	}
</style>
