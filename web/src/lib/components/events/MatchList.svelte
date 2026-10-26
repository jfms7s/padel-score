<!--
	An event's matches in three groups: on court, waiting and finished. Each row has a score
	chip. `view` is read-only (the spectator page); `entry` types results; `manage` adds the
	court select and the View link.

	At 700 px and wider a row's click or `/` opens a result field in the row: Enter saves and
	moves on to the next unfinished match, Esc closes it, ↑/↓ move between rows. A level timed
	score asks who won the overtime game with two buttons under the field. On a phone a tap
	opens the typed-score sheet under the row instead.

	`onsave` throws to refuse; the refusal shows in place. Its client id stays the same while
	the same score is sent again from one open field or sheet, so a retry is replayed.
-->
<script lang="ts" generics="M extends EventViewMatch">
	import { tick } from 'svelte';
	import { MediaQuery } from 'svelte/reactivity';
	import TypedScoreSheet from '#lib/components/TypedScoreSheet.svelte';
	import { groupMatches, moveRow, nextUnfinished } from '#lib/desk.ts';
	import {
		liveScoreText,
		matchText,
		matchViewPath,
		parsedErrorText,
		parseTypedScore,
		resultRefusalText,
		teamText,
		withOvertimeWinner
	} from '#lib/events.ts';
	import { stickyClientId } from '#lib/ids.ts';
	import { scoreText } from '#lib/players.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventTeam, EventViewCourt, EventViewMatch, Format, Team, Teams, TypedScore } from '#lib/types.ts';

	let {
		matches,
		courts,
		teams,
		format,
		mode,
		onsave,
		onmove,
		busy = false
	}: {
		matches: M[];
		courts: EventViewCourt[];
		teams: EventTeam[];
		format: Format;
		mode: 'view' | 'entry' | 'manage';
		onsave?: (m: M, score: TypedScore, clientId: string) => Promise<void>;
		onmove?: (m: M, courtId: string) => void;
		busy?: boolean;
	} = $props();

	const t = strings.desk;
	const te = strings.events;
	const uid = $props.id();

	const groups = $derived(groupMatches(matches, courts));
	const sections = $derived(
		[
			{ key: 'on-court', title: t.onCourt, list: groups.onCourt },
			{ key: 'waiting', title: t.waiting, list: groups.waiting },
			{ key: 'finished', title: t.finishedGroup, list: groups.finished }
		].filter((s) => s.list.length > 0)
	);
	/** The rows top to bottom, as the keys move through them. */
	const rowIds = $derived(sections.flatMap((s) => s.list.map((m) => m.id)));
	const unfinished = $derived(new Set(matches.filter((m) => m.state !== 'finished').map((m) => m.id)));
	const editable = $derived(mode !== 'view' && onsave !== undefined);

	/** Wide screens type in the row; phones use the sheet. */
	const wideQuery = new MediaQuery('(min-width: 700px)');
	const wide = $derived(wideQuery.current);

	/** The row whose field (wide) or sheet (narrow) is open. */
	let openId = $state<string | null>(null);
	let text = $state('');
	let failure = $state<string | null>(null);
	let saving = $state(false);
	/** A level timed score waiting for the overtime game's winner. */
	let overtime = $state<TypedScore | null>(null);
	let field = $state<HTMLInputElement | null>(null);
	const rowButtons: Record<string, HTMLButtonElement | null> = {};
	const ids = stickyClientId();

	/**
	 * A field opened by the user (a click, `/`, or moving on after a save) takes the cursor once
	 * it is rendered. A row re-created by a stream update keeps the field but not the cursor.
	 */
	async function focusField() {
		await tick();
		field?.focus();
	}

	const courtName = (cid: string | null) => courts.find((c) => c.id === cid)?.name ?? null;

	/** A finished match's winner or draw, and in the coordinator's modes whether they typed it. */
	function outcome(m: M): string | null {
		const parts: string[] = [];
		if (m.state === 'finished' && m.result) {
			if (m.winner === null) parts.push(te.draw);
			else parts.push(te.sheet.winner(teamName(m.winner === 'a' ? m.team_a : m.team_b)));
		}
		if (mode !== 'view' && m.locked) parts.push(te.enteredByYou);
		return parts.length > 0 ? parts.join(' · ') : null;
	}

	function chip(m: M): { kind: 'live' | 'done' | 'wait'; text: string } {
		if (m.state === 'live') return { kind: 'live', text: m.live ? liveScoreText(m.live) : te.matchState.live };
		if (m.state === 'finished') return { kind: 'done', text: m.result ? scoreText(m.result) : te.matchState.finished };
		return { kind: 'wait', text: courtName(m.court_id) ?? t.nextFree };
	}

	const teamName = (tid: string) => {
		const tm = teams.find((x) => x.id === tid);
		return tm ? teamText(tm) : te.unknownTeam;
	};

	/** The players' names as the sheet shows them. */
	function teamsOf(m: M): Teams {
		const names = (tid: string): [string, string] => {
			const tm = teams.find((x) => x.id === tid);
			return tm ? [tm.players[0].name, tm.players[1].name] : [te.unknownTeam, te.unknownTeam];
		};
		return { a: names(m.team_a), b: names(m.team_b) };
	}

	function open(mid: string) {
		if (openId !== mid) {
			ids.reset();
			text = '';
			failure = null;
			overtime = null;
		}
		openId = mid;
		// Also puts the cursor back when the open row is clicked again.
		void focusField();
	}

	async function close(focusRow: boolean) {
		const mid = openId;
		openId = null;
		failure = null;
		overtime = null;
		if (!focusRow || mid === null) return;
		await tick();
		rowButtons[mid]?.focus();
	}

	function save(m: M) {
		const parsed = parseTypedScore(text, format);
		if ('ok' in parsed) return commit(m, parsed.ok);
		if (parsed.error === 'needs_overtime_winner') {
			// The picker under the field finishes the score.
			overtime = parsed.score;
			failure = null;
			return;
		}
		failure = parsedErrorText(parsed.error);
	}

	function pickOvertime(m: M, team: Team) {
		if (overtime !== null) void commit(m, withOvertimeWinner(overtime, team));
	}

	async function commit(m: M, score: TypedScore) {
		if (saving || busy || !onsave) return;
		// The order before the save: the saved row may move to the finished group.
		const order = rowIds;
		const left = new Set(unfinished);
		left.delete(m.id);
		saving = true;
		failure = null;
		let refused: string | null = null;
		try {
			await onsave(m, score, ids.for(score));
		} catch (e) {
			refused = resultRefusalText(e);
		} finally {
			saving = false;
		}
		// Closed (Esc) or moved to another row meanwhile: leave it as the user left it.
		if (openId !== m.id) return;
		if (refused !== null) {
			failure = refused;
			return;
		}
		const next = nextUnfinished(order, left, m.id, true);
		if (next === null) await close(true);
		else open(next);
	}

	function fieldKey(e: KeyboardEvent, m: M) {
		if (e.key === 'Enter') {
			e.preventDefault();
			save(m);
		} else if (e.key === 'Escape') {
			e.preventDefault();
			void close(true);
		}
	}

	/** Esc on the overtime buttons closes them with the field; Enter stays a click. */
	function pickerKey(e: KeyboardEvent) {
		if (e.key !== 'Escape') return;
		e.preventDefault();
		void close(true);
	}

	function rowKey(e: KeyboardEvent, mid: string) {
		if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
		e.preventDefault();
		const to = moveRow(rowIds, mid, e.key === 'ArrowDown' ? 1 : -1);
		if (to !== null) rowButtons[to]?.focus();
	}

	/** Open the first unfinished row's field (the first row when all are finished). */
	export function openFirst(): boolean {
		if (!wide || !editable) return false;
		const mid = nextUnfinished(rowIds, unfinished, null, true) ?? rowIds[0];
		if (mid === undefined) return false;
		open(mid);
		return true;
	}

	function listKey(e: KeyboardEvent) {
		if (e.key !== '/' || !wide || !editable) return;
		const target = e.target as HTMLElement;
		if (target.closest('input, textarea, select')) return;
		e.preventDefault();
		// On the overtime buttons `/` is typing too: it must not drop the pending pick.
		if (target.closest('.overtime')) return;
		// On a focused row `/` opens that row; elsewhere in the list, the first unfinished.
		const focused = rowIds.find((mid) => rowButtons[mid] === target);
		if (focused !== undefined) open(focused);
		else openFirst();
	}

	async function sheetSave(m: M, score: TypedScore) {
		// Throws to the sheet, which explains a refusal in place.
		await onsave?.(m, score, ids.for(score));
		openId = null;
	}
</script>

<!-- The wrapper only catches `/` bubbling up from the rows; it takes no focus itself. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="matches" onkeydown={listKey}>
	{#each sections as s (s.key)}
		<section aria-labelledby="{uid}-{s.key}">
			<h3 id="{uid}-{s.key}">{s.title}</h3>
			<ul>
				{#each s.list as m (m.id)}
					{@const vs = matchText(m, teams)}
					{@const c = chip(m)}
					{@const out = outcome(m)}
					{@const court = s.key === 'on-court' ? courtName(m.court_id) : null}
					{@const isOpen = openId === m.id}
					<li class:sel={isOpen && wide}>
						<div class="line">
							{#if editable}
								<button
									class="row-main"
									type="button"
									bind:this={rowButtons[m.id]}
									aria-expanded={isOpen}
									onclick={() => open(m.id)}
									onkeydown={(e) => rowKey(e, m.id)}
								>
									<span class="l">{m.label}{#if court}{' · '}{court}{/if}</span>
									<span class="v">{vs}</span>
									{#if out}<span class="o">{out}</span>{/if}
									<span class="chip {c.kind}">{c.text}</span>
								</button>
							{:else}
								<div class="row-main">
									<span class="l">{m.label}{#if court}{' · '}{court}{/if}</span>
									<span class="v">{vs}</span>
									{#if out}<span class="o">{out}</span>{/if}
									<span class="chip {c.kind}">{c.text}</span>
								</div>
							{/if}
							{#if mode !== 'entry'}
								<div class="actions">
									{#if mode === 'manage' && m.state !== 'finished' && onmove}
										<label>
											<span class="visually-hidden">{te.moveLabel(vs)}</span>
											<select value={m.court_id ?? ''} disabled={busy} onchange={(e) => onmove(m, e.currentTarget.value)}>
												<option value="">{te.notOnCourt}</option>
												{#each courts as ct (ct.id)}
													<option value={ct.id}>{ct.name}</option>
												{/each}
											</select>
										</label>
									{/if}
									<a class="view" href={matchViewPath(m.spectator_token)} aria-label={te.viewLabel(vs)}>{te.view}</a>
								</div>
							{/if}
						</div>
						{#if isOpen && wide}
							<div class="entry">
								<input
									class="result"
									bind:this={field}
									bind:value={text}
									oninput={() => ((failure = null), (overtime = null))}
									onkeydown={(e) => fieldKey(e, m)}
									aria-label={t.typeResult(m.label)}
									aria-describedby={failure ? `${uid}-failure` : undefined}
									placeholder={t.resultPlaceholder}
									readonly={saving || busy}
									inputmode="text"
									autocomplete="off"
									autocapitalize="off"
									spellcheck="false"
									maxlength="60"
								/>
							</div>
							{#if overtime !== null}
								<div class="overtime" role="group" aria-labelledby="{uid}-ot">
									<p id="{uid}-ot">{te.sheet.overtime}</p>
									{#each [['a', m.team_a], ['b', m.team_b]] as const as [team, tid] (team)}
										<button type="button" class={team} disabled={saving || busy} onclick={() => pickOvertime(m, team)} onkeydown={pickerKey}>
											{te.sheet.overtimeWon(teamName(tid))}
										</button>
									{/each}
								</div>
							{/if}
							{#if failure}<p class="error" id="{uid}-failure" role="alert">{failure}</p>{/if}
						{:else if isOpen}
							<TypedScoreSheet
								{format}
								teams={teamsOf(m)}
								title={te.sheet.title(vs)}
								onsubmit={(score) => sheetSave(m, score)}
								oncancel={() => (openId = null)}
							/>
						{/if}
					</li>
				{/each}
			</ul>
		</section>
	{/each}
	{#if editable}
		<p class="keys" aria-hidden="true">
			<span><kbd>Enter</kbd> {t.keys.save}</span>
			<span><kbd>Esc</kbd> {t.keys.cancel}</span>
			<span><kbd>↑</kbd><kbd>↓</kbd> {t.keys.move}</span>
		</p>
	{/if}
</div>

<style>
	.matches {
		min-width: 0;
	}
	h3 {
		margin: 0;
		padding: 8px 14px 4px;
		font-size: 12px;
		font-weight: 700;
		color: var(--muted);
		background: var(--glass);
		border-bottom: 1px solid var(--line);
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	li {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 6px 8px;
		border-bottom: 1px solid var(--line);
	}
	li.sel {
		background: var(--a-soft);
		box-shadow: inset 3px 0 0 var(--a);
	}
	.line {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.row-main {
		flex: 1 1 16rem;
		min-width: 0;
		min-height: var(--tap);
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		gap: 0 10px;
		align-items: center;
		padding: 4px 6px;
		border: 0;
		border-radius: 10px;
		background: transparent;
		color: var(--ink);
		font: inherit;
		font-size: 14px;
		text-align: left;
	}
	button.row-main {
		cursor: pointer;
	}
	button.row-main:hover {
		background: var(--a-soft);
	}
	.l {
		font-weight: 700;
		overflow-wrap: anywhere;
	}
	.v,
	.o {
		grid-column: 1;
		color: var(--muted);
		overflow-wrap: anywhere;
	}
	.o {
		font-size: 13px;
	}
	.chip {
		grid-row: 1 / span 3;
		grid-column: 2;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 2px 10px;
		border-radius: 999px;
		font-size: 13px;
		font-weight: 650;
		font-feature-settings: 'tnum';
		white-space: nowrap;
		background: var(--a-soft);
		color: var(--ink);
	}
	.chip.live::before {
		content: '';
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--ball);
		box-shadow: 0 0 0 2px color-mix(in srgb, var(--ball) 30%, transparent);
	}
	.chip.done {
		background: transparent;
		border: 1px solid var(--line);
		color: var(--muted);
	}
	.chip.wait {
		background: transparent;
		border: 1px dashed var(--line);
		color: var(--muted);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
	}
	select,
	.view {
		min-height: var(--tap);
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-weight: 650;
	}
	.view {
		display: inline-flex;
		align-items: center;
		text-decoration: none;
	}
	.entry {
		padding: 0 6px;
	}
	.result {
		width: 100%;
		max-width: 20rem;
		min-height: 40px;
		padding: 0 10px;
		border: 2px solid var(--a);
		border-radius: 9px;
		background: var(--panel);
		color: var(--ink);
		font-size: 15px;
		font-weight: 700;
		font-feature-settings: 'tnum';
	}
	.overtime {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		padding: 0 6px;
	}
	.overtime p {
		flex-basis: 100%;
		margin: 0;
		font-weight: 650;
	}
	.overtime button {
		min-height: 40px;
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 10px;
		background: var(--panel);
		color: var(--ink);
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.overtime button.a {
		border-color: var(--a);
	}
	.overtime button.b {
		border-color: var(--b);
	}
	.error {
		margin: 0 6px;
		color: var(--warn);
		font-size: 14px;
	}
	.keys {
		display: none;
		flex-wrap: wrap;
		gap: 10px;
		margin: 0;
		padding: 8px 14px;
		font-size: 12px;
		color: var(--muted);
	}
	kbd {
		font: inherit;
		font-size: 12px;
		font-weight: 650;
		padding: 1px 6px;
		border-radius: 6px;
		border: 1px solid var(--line);
		border-bottom-width: 2px;
		background: var(--panel);
		color: var(--muted);
	}
	@media (min-width: 700px) {
		.keys {
			display: flex;
		}
	}
</style>
