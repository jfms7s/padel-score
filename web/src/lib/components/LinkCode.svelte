<script lang="ts">
	// A link made to be handed over once (an invite, a reset): its QR code, the link itself and a
	// copy button, as in the share sheet.
	import QRCode from 'qrcode';
	import { strings } from '#lib/strings.ts';

	let { url, title, hint }: { url: string; title: string; hint: string } = $props();

	const s = strings.auth.share;
	let code = $state('');
	let copied = $state(false);
	let copyFailed = $state(false);

	$effect(() => {
		const text = url;
		let live = true;
		void QRCode.toString(text, {
			type: 'svg',
			margin: 1,
			errorCorrectionLevel: 'M',
			color: { dark: '#0b2236', light: '#ffffff' }
		})
			.then((svg) => {
				if (live) code = svg;
			})
			.catch(() => {
				// The link below still works without the picture.
			});
		return () => {
			live = false;
		};
	});

	async function copy() {
		copyFailed = false;
		try {
			await navigator.clipboard.writeText(url);
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch {
			copyFailed = true;
		}
	}
</script>

<figure class="qr">
	<div class="code" role="img" aria-label={s.qr}>
		<!-- The SVG is generated locally by the qrcode library from our own link. -->
		{@html code}
	</div>
	<figcaption>
		<strong>{title}</strong>
		<span class="hint">{hint}</span>
		<input class="link" readonly value={url} aria-label={title} onclick={(e) => e.currentTarget.select()} />
		<button type="button" class="primary" onclick={copy}>{copied ? s.copied : s.copy}</button>
		{#if copyFailed}<span class="hint" role="alert">{s.copyFailed}</span>{/if}
	</figcaption>
</figure>

<style>
	.qr {
		margin: 12px 0 0;
		display: flex;
		flex-direction: column;
		gap: 10px;
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 14px;
		background: var(--glass);
	}
	.code {
		width: min(100%, 220px);
		aspect-ratio: 1;
		align-self: center;
		background: #fff;
		border-radius: 8px;
		padding: 6px;
	}
	.code :global(svg) {
		display: block;
		width: 100%;
		height: 100%;
	}
	figcaption {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.link {
		font-size: 13px;
	}
</style>
