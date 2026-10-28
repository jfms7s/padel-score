<!--
	A name field that suggests the signed-in user's players (a combobox with a listbox). Typing
	keeps the name free text and drops any picked player; choosing a suggestion picks it; the last
	option adds the typed name to the list and picks the new player. Arrow keys move through the
	options, Enter chooses, Escape closes.
-->
<script lang="ts">
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { addPlayer, nameKey, playerMessage, recordText, suggest } from '#lib/players.ts';
	import { strings } from '#lib/strings.ts';
	import type { Player } from '#lib/types.ts';

	const t = strings.players;

	let {
		value = $bindable(''),
		playerId = $bindable(null),
		players,
		taken,
		placeholder,
		label,
		pickOnly = false,
		onadd
	}: {
		value: string;
		playerId: string | null;
		players: Player[];
		/** The players picked in the other slots: never offered here. */
		taken: string[];
		placeholder: string;
		label: string;
		/** Only a player from the list (or a newly added one) counts: other text is cleared on leaving. */
		pickOnly?: boolean;
		/** A player was added to the list from here. */
		onadd: (player: Player) => void;
	} = $props();

	const uid = $props.id();
	const listId = `${uid}-list`;
	const pickedId = `${uid}-picked`;

	let open = $state(false);
	let active = $state(-1);
	let busy = $state(false);
	let failure = $state<string | null>(null);

	const found = $derived(suggest(players, value, taken));
	/** The options in order: the matching players, then "Add" when the name can be added. */
	const options = $derived<({ kind: 'player'; player: Player } | { kind: 'add'; name: string })[]>([
		...found.matches.map((player) => ({ kind: 'player' as const, player })),
		...(found.canAdd ? [{ kind: 'add' as const, name: value.trim() }] : [])
	]);
	const expanded = $derived(open && options.length > 0);
	const optionId = (i: number) => `${uid}-opt-${i}`;

	function typed() {
		playerId = null;
		failure = null;
		open = true;
		active = -1;
	}

	function pick(player: Player) {
		value = player.name;
		playerId = player.id;
		open = false;
		active = -1;
	}

	async function add(name: string) {
		if (busy) return;
		busy = true;
		failure = null;
		try {
			const player = await addPlayer(name);
			onadd(player);
			// Bind it only if the field still holds that name (it may have been edited meanwhile).
			if (nameKey(value) === nameKey(name)) pick(player);
		} catch (e) {
			failure = playerMessage(e);
		} finally {
			busy = false;
		}
	}

	function choose(i: number) {
		const option = options[i];
		if (!option) return;
		if (option.kind === 'player') pick(option.player);
		else void add(option.name);
	}

	function onkeydown(event: KeyboardEvent) {
		const n = options.length;
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			if (n === 0) return;
			event.preventDefault();
			if (!open) {
				open = true;
				active = event.key === 'ArrowDown' ? 0 : n - 1;
				return;
			}
			active = event.key === 'ArrowDown' ? (active + 1) % n : (active <= 0 ? n : active) - 1;
		} else if (event.key === 'Enter') {
			// Enter on an option chooses it; otherwise it submits the form as usual.
			if (expanded && active >= 0) {
				event.preventDefault();
				choose(active);
			}
		} else if (event.key === 'Escape') {
			if (expanded) {
				event.preventDefault();
				open = false;
				active = -1;
			}
		}
	}

	let listEl = $state<HTMLUListElement | null>(null);

	// A press on the list must not blur the field first (that would close the list before the
	// click). Cancelling mousedown is what keeps focus in Chromium and Firefox; pointerdown too.
	const keepFocus = (event: Event) => event.preventDefault();
</script>

<div class="picker">
	<div class="field">
		<input
			bind:value
			oninput={typed}
			onfocus={() => (open = true)}
			onblur={(event) => {
				// A press on the list keeps focus here (see keepFocus); if focus went into it anyway,
				// leave it open so the click still lands.
				const to = event.relatedTarget;
				if (to instanceof Node && listEl?.contains(to)) return;
				open = false;
				active = -1;
				if (pickOnly && playerId === null && !busy) value = '';
			}}
			{onkeydown}
			{placeholder}
			aria-label={label}
			role="combobox"
			aria-autocomplete="list"
			aria-expanded={expanded}
			aria-controls={listId}
			aria-activedescendant={expanded && active >= 0 ? optionId(active) : undefined}
			aria-describedby={playerId ? pickedId : undefined}
			autocomplete="off"
			maxlength={MAX_NAME_LENGTH}
		/>
		{#if playerId}
			<span class="tick" aria-hidden="true">✓</span>
			<span class="visually-hidden" id={pickedId}>{t.fromList}</span>
		{/if}
	</div>
	<ul
		bind:this={listEl}
		class="list"
		id={listId}
		role="listbox" aria-label={t.suggestions(label)}
		hidden={!expanded}
		onmousedown={keepFocus}
		onpointerdown={keepFocus}
	>
		{#each options as option, i (option.kind === 'player' ? option.player.id : 'add')}
			<!-- The keys are the combobox's: focus stays in the field (aria-activedescendant). -->
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<li
				id={optionId(i)}
				role="option"
				aria-selected={i === active}
				class:active={i === active}
				class:add={option.kind === 'add'}
				onclick={() => choose(i)}
			>
				{#if option.kind === 'player'}
					<span class="name">{option.player.name}</span>
					<span class="rec">{recordText(option.player.record)}</span>
				{:else}
					<span class="name">{t.addToList(option.name)}</span>
				{/if}
			</li>
		{/each}
	</ul>
	{#if busy}<p class="note" role="status">{t.adding}</p>{/if}
	{#if failure}<p class="note error" role="alert">{failure}</p>{/if}
</div>

<style>
	.picker {
		position: relative;
	}
	.field {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	input {
		flex: 1;
		min-width: 0;
		width: 100%;
		min-height: var(--tap);
		border: 0;
		background: transparent;
		padding: 2px 0;
	}
	.tick {
		flex: none;
		color: var(--b);
		font-weight: 800;
	}
	.list {
		position: absolute;
		z-index: 10;
		top: 100%;
		left: -10px;
		right: -10px;
		margin: 2px 0 0;
		padding: 4px;
		list-style: none;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 12px;
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.18);
	}
	.list[hidden] {
		display: none;
	}
	li {
		display: flex;
		flex-direction: column;
		justify-content: center;
		min-height: var(--tap);
		padding: 4px 10px;
		border-radius: 9px;
		cursor: pointer;
		overflow-wrap: anywhere;
		line-height: 1.25;
	}
	li.active {
		background: var(--a-soft);
		outline: 2px solid var(--a);
		outline-offset: -2px;
	}
	li:hover {
		background: var(--a-soft);
	}
	li.add .name {
		font-weight: 650;
		color: var(--a);
	}
	.rec {
		font-size: 12px;
		color: var(--muted);
	}
	.note {
		margin: 0 0 4px;
		font-size: 12px;
		color: var(--muted);
	}
	.note.error {
		color: var(--warn);
	}
</style>
