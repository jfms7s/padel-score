<script lang="ts">
	import { onMount } from 'svelte';
	import { answerRequest, boardMessage } from '#lib/boards.ts';
	import {
		byDay,
		dismissPushPrompt,
		getCalendar,
		itemDetail,
		itemHref,
		itemNote,
		itemTime,
		itemTitle,
		pushPromptDismissed,
		shiftWeek,
		startOfToday,
		weekStart
	} from '#lib/calendar.ts';
	import CalendarWeek from '#lib/components/CalendarWeek.svelte';
	import RequestCard from '#lib/components/boards/RequestCard.svelte';
	import { addDays, browserTimeZone, dayLabel, localDate, localMidnight, timeText } from '#lib/dates.ts';
	import { currentPushEnv, devicePushOn, enablePush, pushSupport } from '#lib/push.ts';
	import { requireSignedIn } from '#lib/session.ts';
	import { shareLink } from '#lib/share.ts';
	import { partnerLink } from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import { safeStorage } from '#lib/storage.ts';
	import type { Calendar, OpenItem } from '#lib/types.ts';

	const t = strings.calendar;
	const zone = browserTimeZone();

	let cal = $state<Calendar | null>(null);
	let failure = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let working = $state(false);
	let note = $state<string | null>(null);

	// Recomputed whenever the page comes back, so a page left open past midnight moves on.
	let today = $state(localDate(Date.now() / 1000, zone));
	const thisWeek = $derived(weekStart(today));
	let weekFirst = $state(weekStart(localDate(Date.now() / 1000, zone)));
	const weekEnd = $derived(addDays(weekFirst, 6));

	const days = $derived(cal ? byDay(cal.items, zone) : []);
	const label = (date: string) => dayLabel(localMidnight(date, zone) + 43200, zone);

	// The notifications card: shown once, to someone whose browser can and who has not said no.
	let pushCard = $state(false);
	let pushFailure = $state<string | null>(null);

	onMount(() => {
		void load();
		// Coming back to the page: what waits for an answer may be gone, and the day may have changed.
		const wake = () => {
			if (document.visibilityState === 'hidden' || cal === null) return;
			void refresh();
		};
		document.addEventListener('visibilitychange', wake);
		window.addEventListener('focus', wake);
		return () => {
			document.removeEventListener('visibilitychange', wake);
			window.removeEventListener('focus', wake);
		};
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') failure = strings.networkError;
		if (s.status !== 'signed_in') return;
		await refresh();
		void checkPush();
	}

	async function refresh() {
		try {
			const now = Date.now() / 1000;
			const fresh = localDate(now, zone);
			if (fresh !== today) {
				// The day rolled over: today moves, and the week view follows if it showed the old week.
				if (weekFirst === thisWeek) weekFirst = weekStart(fresh);
				today = fresh;
			}
			cal = await getCalendar(startOfToday(zone, now));
			failure = null;
		} catch (e) {
			failure = boardMessage(e);
		}
	}

	async function checkPush() {
		if (pushSupport(currentPushEnv()) !== 'available') return;
		if (pushPromptDismissed(safeStorage())) return;
		try {
			pushCard = !(await devicePushOn());
		} catch {
			pushCard = false;
		}
	}

	async function turnOn() {
		const result = await enablePush();
		if (result === 'on') pushCard = false;
		else pushFailure = result === 'denied' ? strings.push.denied : strings.push.failed;
	}

	function notNow() {
		dismissPushPrompt(safeStorage());
		pushCard = false;
	}

	async function answer(id: string, accept: boolean) {
		working = true;
		actionError = null;
		try {
			await answerRequest(id, accept);
		} catch (e) {
			actionError = boardMessage(e);
		} finally {
			working = false;
		}
		await refresh();
	}

	async function resend(joinToken: string, partnerToken: string, eventName: string) {
		const result = await shareLink(
			partnerLink(location.origin, joinToken, partnerToken),
			t.partnerShareTitle(eventName)
		);
		note = result === 'copied' ? t.linkCopied : result === 'failed' ? t.linkFailed : null;
	}

	function openText(o: OpenItem): string {
		const when = `${dayLabel(o.starts_at, zone)} · ${timeText(o.starts_at, zone)}`;
		return o.kind === 'event'
			? `${when} · ${t.teamsOpen(o.teams, o.max_teams)}`
			: `${when} · ${t.placesOpen(o.open_places)}`;
	}
</script>

<main class="page-wide-desk">
	<h1>{t.title}</h1>

	{#if failure}
		<p class="error" role="alert">{failure}</p>
	{:else if cal === null}
		<p class="hint">{t.loading}</p>
	{/if}

	{#if cal}
		{#if cal.waiting.length > 0}
			<section aria-labelledby="waiting">
				<h2 id="waiting">{t.waiting}</h2>
				{#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
				<ul class="stack">
					{#each cal.waiting as w (w.kind === 'partner' ? `p:${w.partner_token}` : w.request_id)}
						<li>
							{#if w.kind === 'swap'}
								<RequestCard
									kind="swap"
									from={w.from_name}
									current={w.my_game}
									proposed={w.their_game}
									busy={working}
									onanswer={(accept) => answer(w.request_id, accept)}
								/>
							{:else if w.kind === 'time'}
								<RequestCard
									kind="time"
									from={w.from_name}
									current={w.game}
									proposed={w.new}
									busy={working}
									onanswer={(accept) => answer(w.request_id, accept)}
								/>
							{:else}
								<div class="card partner">
									<h3>{t.partnerWaiting(w.event_name)}</h3>
									<p class="hint">{t.partnerHint}</p>
									<button
										type="button"
										class="secondary"
										onclick={() => resend(w.join_token, w.partner_token, w.event_name)}>{t.partnerResend}</button
									>
								</div>
							{/if}
						</li>
					{/each}
				</ul>
				{#if note}<p class="hint" role="status">{note}</p>{/if}
			</section>
		{/if}

		{#if pushCard}
			<section class="card" aria-label={t.pushTitle}>
				<h2>{t.pushTitle}</h2>
				<p class="hint">{t.pushHint}</p>
				{#if pushFailure}<p class="error" role="alert">{pushFailure}</p>{/if}
				<div class="actions">
					<button type="button" class="primary" onclick={turnOn}>{t.pushOn}</button>
					<button type="button" class="secondary" onclick={notNow}>{t.pushNotNow}</button>
				</div>
			</section>
		{/if}

		<div class="split">
			<section class="weekcol" aria-label={t.week}>
				<div class="weeknav">
					<button
						type="button"
						class="secondary arrow"
						aria-label={t.prevWeek}
						disabled={weekFirst <= thisWeek}
						onclick={() => (weekFirst = shiftWeek(weekFirst, -1))}>‹</button
					>
					<strong>{t.weekOf(label(weekFirst), label(weekEnd))}</strong>
					<button
						type="button"
						class="secondary arrow"
						aria-label={t.nextWeek}
						onclick={() => (weekFirst = shiftWeek(weekFirst, 1))}>›</button
					>
				</div>
				<CalendarWeek items={cal.items} first={weekFirst} {today} />
			</section>

			<section class="listcol" aria-labelledby="upcoming">
				<h2 id="upcoming">{t.upcoming}</h2>
				{#if days.length === 0}
					<p class="hint">{t.empty}</p>
				{:else}
					{#each days as day (day.date)}
						<h3 class="day">{label(day.date)}</h3>
						<ul class="stack">
							{#each day.items as item, i (`${day.date}:${i}`)}
								<li>
									<a class="row {item.kind}" href={itemHref(item)}>
										<span class="time">{itemTime(item, zone)}</span>
										<span class="what">
											<span class="title">{itemTitle(item)}</span>
											<span class="sub">{itemNote(item)}</span>
											{#if itemDetail(item)}<span class="sub">{itemDetail(item)}</span>{/if}
										</span>
									</a>
								</li>
							{/each}
						</ul>
					{/each}
				{/if}
			</section>
		</div>

		<section aria-labelledby="boards">
			<h2 id="boards">{t.boards}</h2>
			{#if cal.boards.length > 0}
				<ul class="stack">
					{#each cal.boards as b (b.token)}
						<li>
							<a class="row" href="/b/{encodeURIComponent(b.token)}">
								<span class="what">
									<span class="title">{b.title}</span>
									<span class="sub"
										>{dayLabel(b.starts_at, zone)} – {dayLabel(b.ends_at - 1, zone)} · {t.boardGames(b.my_games)}</span
									>
								</span>
							</a>
						</li>
					{/each}
				</ul>
			{/if}
			<a class="primary" href="/boards/new">{t.newBoard}</a>
		</section>

		<section aria-labelledby="open">
			<h2 id="open">{t.openToJoin}</h2>
			{#if cal.open.length === 0}
				<p class="hint">{t.openNone}</p>
			{:else}
				<ul class="stack">
					{#each cal.open as o (`${o.kind}:${o.join_token}`)}
						<li>
							<a class="row {o.kind}" href={itemHref(o)}>
								<span class="what">
									<span class="title">{o.kind === 'event' ? o.name : t.matchBy(o.owner_name)}</span>
									<span class="sub">{openText(o)}{o.place ? ` · ${o.place}` : ''}</span>
								</span>
							</a>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}
</main>

<style>
	main {
		padding-top: 16px;
		padding-bottom: 48px;
	}
	h1 {
		margin: 0 0 8px;
		font-size: 32px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
	}
	h2 {
		margin: 32px 0 12px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	h3 {
		margin: 0 0 6px;
		font-size: 16px;
		overflow-wrap: anywhere;
	}
	h3.day {
		margin: 16px 0 6px;
		font-size: 14px;
		color: var(--muted);
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	.error {
		color: var(--warn);
	}
	.stack {
		list-style: none;
		margin: 0 0 12px;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.card {
		margin-top: 16px;
		padding: 12px 14px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
	}
	.card h2 {
		margin: 0 0 4px;
		font-size: 17px;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	button,
	a.primary {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		padding: 0 18px;
		border-radius: 14px;
		font-size: 16px;
		font-weight: 650;
		text-decoration: none;
	}
	.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	a.primary {
		display: flex;
	}
	.secondary {
		border: 1px solid var(--line);
		background: var(--panel);
		color: var(--ink);
	}
	button:disabled {
		opacity: 0.5;
	}
	.row {
		display: flex;
		gap: 12px;
		align-items: center;
		min-height: var(--tap);
		padding: 8px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.time {
		flex: none;
		min-width: 4.5rem;
		font-weight: 700;
		font-size: 14px;
	}
	.what {
		display: grid;
		min-width: 0;
	}
	.title {
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.sub {
		font-size: 13px;
		color: var(--muted);
	}
	.weekcol {
		display: none;
	}
	.weeknav {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin: 0 0 8px;
	}
	.arrow {
		width: var(--tap);
		padding: 0;
		font-size: 22px;
	}
	@media (min-width: 700px) {
		.weekcol {
			display: block;
			margin-top: 24px;
		}
	}
	@media (min-width: 1100px) {
		.split {
			display: grid;
			grid-template-columns: minmax(0, 1fr) 22rem;
			gap: 32px;
			align-items: start;
		}
		.listcol h2 {
			margin-top: 24px;
		}
	}
</style>
