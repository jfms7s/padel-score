<script lang="ts">
	// A QR code for `text`, drawn locally as SVG by the qrcode library (nothing leaves the page).
	// `label` names the picture for screen readers; `size` is its width in pixels (default: fill
	// the container up to 220 px).
	import QRCode from 'qrcode';

	let { text, label, size }: { text: string; label: string; size?: number } = $props();

	let svg = $state('');

	$effect(() => {
		const t = text;
		let live = true;
		QRCode.toString(t, {
			type: 'svg',
			margin: 1,
			errorCorrectionLevel: 'M',
			color: { dark: '#0b2236', light: '#ffffff' }
		}).then(
			(out) => {
				if (live) svg = out;
			},
			() => {
				// The link beside the code still works without the picture.
				if (live) svg = '';
			}
		);
		return () => {
			live = false;
		};
	});
</script>

<div class="code" role="img" aria-label={label} style:width={size === undefined ? undefined : `${size}px`}>
	<!-- The SVG is generated locally by the qrcode library from our own link. -->
	{@html svg}
</div>

<style>
	.code {
		width: min(100%, 220px);
		max-width: 100%;
		aspect-ratio: 1;
		background: #fff;
		border-radius: 8px;
		padding: 6px;
	}
	.code :global(svg) {
		display: block;
		width: 100%;
		height: 100%;
	}
</style>
